//! HTDemucs stem-separation surface (Defossez 2021).
//!
//! Splits a recording into the four standard Demucs buses (vocals /
//! drums / bass / other) via the HTDemucs ONNX model and writes each as
//! a FLAC under `$APPDATA/recordings/<recording_id>/stems/<stem>.flac`.
//! The pointer row lives in `stem_results` (see V0003 migration); the
//! FLAC payload lives on disk so the DB stays small and the WAL stays
//! fast.
//!
//! This module exposes pure-blocking headless twins so the Tauri command
//! layer in [`crate::commands`] can `spawn_blocking` them and the
//! integration tests under `tests/` can call them directly without
//! standing up a full Tauri runtime — same shape as
//! [`crate::transcribe`].
//!
//! The whole module is gated behind `feature = "neural"` at the
//! `mod stems;` declaration in `lib.rs`; the inner core surface in
//! `neural_pitch_core::stems` runs the ONNX inference pass and the
//! 48 kHz ↔ 44.1 kHz resample pair.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingSink};
use neural_pitch_core::store::{ListFilter, RecordingId, RecordingsLibrary};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

/// Stable separator-version string persisted in
/// `stem_results.separator_version` and on the cache key. Bumped in
/// lock-step with any HTDemucs model swap or the on-the-wire
/// [`SeparateProgress`] / [`StemSummary`] schema change so cached blobs
/// invalidate cleanly on any ONNX checkpoint swap.
pub const HTDEMUCS_SEPARATOR_VERSION: &str = "htdemucs-4.0.1";

/// SHA-256 checksum of the HTDemucs ONNX model bundle, mirroring the
/// build-time constant verified inside
/// [`neural_pitch_core::stems::download::ensure_at`] before the temp
/// file is renamed into `<models_dir>/htdemucs.onnx`.
pub const HTDEMUCS_MODEL_SHA256: &str = neural_pitch_core::stems::HTDEMUCS_SHA256;

/// Public download URL for the HTDemucs ONNX model bundle. The model is
/// not committed to the repo (~316 MB); first separation attempts fetch
/// it on demand and cache it under `<models_dir>/htdemucs.onnx`.
/// Surfaced in error messages when the user is offline so the front-end
/// can paste the URL into a manual download flow.
pub const HTDEMUCS_MODEL_URL: &str = neural_pitch_core::stems::HTDEMUCS_MODEL_URL;

/// One of the four standard Demucs buses. Serialised as `snake_case` so
/// the on-the-wire IPC discriminant matches the on-disk filename
/// (`vocals.flac`, `drums.flac`, …) verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StemKind {
    /// Lead + backing vocal bus.
    Vocals,
    /// Drum kit / percussive bus.
    Drums,
    /// Bass-instrument bus (electric, acoustic, synth bass).
    Bass,
    /// Everything else — keys, guitars, strings, FX.
    Other,
}

impl StemKind {
    /// Filename slug used for the on-disk FLAC under
    /// `<recordings_dir>/<recording_id>/stems/<slug>.flac`.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Vocals => "vocals",
            Self::Drums => "drums",
            Self::Bass => "bass",
            Self::Other => "other",
        }
    }

    /// Discriminant string persisted in `analysis_cache.stem_kind` for
    /// per-stem analysis cache rows. Matches [`StemKind::slug`] today;
    /// kept as a separate accessor so a future schema can decouple the
    /// on-disk filename from the SQL column without churning callers.
    #[must_use]
    pub const fn cache_discriminant(self) -> &'static str {
        self.slug()
    }
}

/// Stages emitted by [`separate_stems_blocking`] over the progress
/// channel. Discriminated union so the front-end TS surface is a
/// `{ stage: "..."; percent: number }` tagged union rather than a free-
/// form string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeparateStage {
    /// First-run path — the HTDemucs ONNX is being fetched into
    /// `<models_dir>/htdemucs.onnx`.
    Download,
    /// Source recording is being decoded into the f32 PCM buffer the
    /// separator consumes.
    Decode,
    /// HTDemucs inference is running on the decoded buffer.
    Separate,
    /// Four-stem PCM output is being FLAC-encoded to disk.
    Encode,
}

/// Per-tick progress message emitted on the [`separate_stems`]
/// (Tauri command) / [`separate_stems_blocking`] (headless twin)
/// channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SeparateProgress {
    /// Stringified [`RecordingId`] of the recording being separated.
    pub recording_id: String,
    /// Stage discriminator. The four stages run sequentially per
    /// invocation; the front-end can derive an overall ETA from
    /// `(stage, percent)`.
    pub stage: SeparateStage,
    /// Stage-local progress in `[0.0, 1.0]`.
    pub percent: f32,
}

/// Sink trait fed by [`separate_stems_blocking`]. Implementations MUST
/// tolerate the receiver closing early — channel-based assertions in
/// tests rely on the dropped-consumer path being a `tracing::debug!`
/// no-op rather than a panic, mirroring the
/// [`crate::transcribe::TranscribeProgressSink`] contract.
pub trait SeparateProgressSink: Send + Sync {
    /// Emit one progress tick.
    fn emit(&self, progress: SeparateProgress);
}

/// Wire summary returned by [`separate_stems_blocking`].
///
/// Mirrors the [`crate::transcribe::TranscribeSummary`] shape so the
/// front-end can re-use its existing summary adapter pattern. The four
/// path columns are absolute paths under `recordings_dir` so the
/// front-end can hand them straight to the asset-protocol resolver.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StemSummary {
    /// Stable separator identifier (`"htdemucs-4.0.1"`).
    pub separator_version: String,
    /// `true` when this summary came from the `stem_results` table; the
    /// HTDemucs inference was skipped entirely.
    pub was_cached: bool,
    /// On-disk FLAC for the vocals bus.
    pub vocals_path: String,
    /// On-disk FLAC for the drums bus.
    pub drums_path: String,
    /// On-disk FLAC for the bass bus.
    pub bass_path: String,
    /// On-disk FLAC for the "other" bus (keys / guitar / strings / FX).
    pub other_path: String,
    /// Wall-clock time the separation completed (or was first cached),
    /// in Unix milliseconds. Mirrors `stem_results.completed_at_unix_ms`.
    pub completed_at_unix_ms: i64,
}

/// Per-tick progress message emitted by
/// [`download_stem_model_blocking`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DownloadProgress {
    /// Bytes downloaded so far.
    pub bytes_downloaded: u64,
    /// Total bytes the server reports via `Content-Length`. `0` if the
    /// server omitted the header.
    pub total_bytes: u64,
    /// Progress in `[0.0, 1.0]`. Falls back to `0.0` when
    /// `total_bytes == 0`.
    pub percent: f32,
}

/// Sink trait for [`download_stem_model_blocking`]. Same drop-tolerance
/// contract as [`SeparateProgressSink`].
pub trait DownloadProgressSink: Send + Sync {
    /// Emit one download-progress tick.
    fn emit(&self, progress: DownloadProgress);
}

/// Typed error surface for [`separate_stems_blocking`] /
/// [`download_stem_model_blocking`].
#[derive(Debug, Error)]
pub enum StemError {
    /// The supplied recording id did not resolve through the recordings
    /// library.
    #[error("recording not found: {0}")]
    RecordingNotFound(RecordingId),
    /// HTDemucs ONNX model is not present on disk and the user is
    /// offline. The front-end surfaces [`HTDEMUCS_MODEL_URL`] verbatim
    /// so the operator can pre-fetch on a metered network.
    #[error("model not downloaded; pre-fetch from {0}")]
    ModelMissing(&'static str),
    /// Decoded SHA-256 did not match [`HTDEMUCS_MODEL_SHA256`].
    #[error("model checksum mismatch (expected {expected}, observed {observed})")]
    ChecksumMismatch {
        /// Expected SHA-256 from the build-time constant.
        expected: String,
        /// Observed SHA-256 from the freshly-downloaded payload.
        observed: String,
    },
    /// Decode of the on-disk recording FLAC / WAV failed.
    #[error("decode failed: {0}")]
    DecodeFailed(String),
    /// HTDemucs separation pass failed.
    #[error("separator failed: {0}")]
    SeparatorFailed(String),
    /// FLAC encode of one of the four stem buses failed.
    #[error("flac encode failed: {0}")]
    EncodeFailed(String),
    /// Persistence-layer error from `library.upsert_*` or the SQLite
    /// read path.
    #[error("library failure: {0}")]
    Library(String),
    /// `std::fs` error during stem-directory creation or atomic FLAC
    /// rename.
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    /// The cancellation token was tripped between two checkpoints
    /// (decode → separate → encode). The partial outputs (if any) are
    /// removed before returning.
    #[error("separation cancelled")]
    Cancelled,
    /// The `spawn_blocking` worker panicked. Reified as a typed variant
    /// so the Tauri command can flatten it to a `String` without
    /// special-casing `JoinError::is_panic()`.
    #[error("separation worker panicked: {0}")]
    Panicked(String),
}

impl From<neural_pitch_core::stems::StemError> for StemError {
    fn from(value: neural_pitch_core::stems::StemError) -> Self {
        use neural_pitch_core::stems::StemError as Core;
        match value {
            Core::ModelNotFound(_) | Core::OfflineFirstUse { .. } => {
                Self::ModelMissing(HTDEMUCS_MODEL_URL)
            }
            Core::HashMismatch { expected, actual } => Self::ChecksumMismatch {
                expected,
                observed: actual,
            },
            Core::Cancelled => Self::Cancelled,
            Core::Io(e) => Self::IoError(e),
            Core::Ort(msg) | Core::Configuration(msg) => Self::SeparatorFailed(msg),
            // The upstream `StemError` is `#[non_exhaustive]`; new
            // variants surface here as a generic separator failure
            // until they grow a dedicated mapping.
            other => Self::SeparatorFailed(format!("{other:#}")),
        }
    }
}

/// Counter snapshot returned by
/// [`StemSeparator::onnx_invocation_count`]. The cache-hit
/// fast-path tests assert that a second invocation does not increment
/// this counter — i.e. the ONNX session is never touched on a cached
/// re-separation.
#[derive(Debug, Clone, Copy, Default)]
pub struct OnnxInvocationSnapshot {
    /// Total HTDemucs inference passes run during the lifetime of the
    /// `Arc<StemSeparator>`.
    pub count: u64,
}

/// Lazily-initialised HTDemucs separator. The `Arc` is shared across
/// `separate_stems` invocations so the ONNX session and the
/// inference-count counter persist across cache-hit fast-paths and
/// cache-miss warm-up paths.
///
/// The inner `neural_pitch_core::stems::StemSeparator` is constructed on
/// the first cache-miss separation pass; the ONNX session load is
/// O(seconds), so deferring it past startup keeps app boot snappy.
#[derive(Default)]
pub struct StemSeparator {
    /// Inference-count snapshot. Bumped exactly once per cache-miss
    /// separation pass; cache-hit fast paths never touch the counter.
    invocation_count: AtomicU64,
    /// Optional override for the on-disk model cache directory. `None`
    /// resolves the platform default via
    /// `neural_pitch_core::stems::StemSeparator::ensure_model`.
    models_dir: Option<PathBuf>,
    /// Lazily-initialised ONNX-backed core separator. Populated under
    /// the mutex on the first cache-miss pass and reused thereafter.
    inner: Mutex<Option<neural_pitch_core::stems::StemSeparator>>,
}

impl StemSeparator {
    /// Construct a separator that resolves the model cache via the
    /// per-platform default (`directories::ProjectDirs`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a separator that resolves the model cache under an
    /// explicit directory. The Tauri shell uses this so the on-disk
    /// model location matches `app.path().app_data_dir()`.
    #[must_use]
    pub fn with_models_dir(models_dir: PathBuf) -> Self {
        Self {
            invocation_count: AtomicU64::new(0),
            models_dir: Some(models_dir),
            inner: Mutex::new(None),
        }
    }

    /// Snapshot of the inference counter. The persistence test
    /// drives `separate_stems_blocking` twice and asserts the counter
    /// did not increment between the two calls — proves the second
    /// invocation came from `stem_results` and never touched the ONNX
    /// session.
    #[must_use]
    pub fn onnx_invocation_count(&self) -> OnnxInvocationSnapshot {
        OnnxInvocationSnapshot {
            count: self.invocation_count.load(Ordering::Relaxed),
        }
    }
}

/// Sample rate locked for stem FLACs. Matches the recording pipeline's
/// 48 kHz default so the FLAC sink accepts the buffer without an
/// intermediate resample.
const STEM_SAMPLE_RATE_HZ: u32 = 48_000;

/// Mono channel count locked for the stem-separation pipeline. The
/// recording sink writes mono FLACs and the core separator returns one
/// `Vec<f32>` per stem at the input channel count.
const STEM_CHANNELS: u32 = 1;

/// Headless twin of the `separate_stems` Tauri command.
///
/// Workflow:
/// 1. Resolve the recording row through `library.list_recordings(..)`.
/// 2. Cache lookup against `stem_results` keyed on
///    `(recording_id, HTDEMUCS_SEPARATOR_VERSION)`. On hit the four
///    FLAC paths are returned and the ONNX session is never touched.
/// 3. On miss decode the on-disk recording, ensure the HTDemucs ONNX
///    model is cached on disk (downloads on first use), run the
///    four-bus split via [`neural_pitch_core::stems::StemSeparator`],
///    FLAC-encode each of the four stems into
///    `<recordings_dir>/<recording_id>/stems/<slug>.flac`, and upsert
///    one `stem_results` row.
/// 4. Emit one [`SeparateProgress`] tick per stage milestone; tolerate
///    the receiver closing early.
/// 5. Poll `cancel` between stages and return [`StemError::Cancelled`]
///    promptly when the token trips. Partial files are removed before
///    returning so a cancelled run leaves no orphans.
///
/// The `separator` argument is taken as `Arc<StemSeparator>` so a
/// cancel-then-restart sequence reuses the warm separator handle.
//
// `Arc<StemSeparator>` + `CancellationToken` are intentionally moved so
// callers compose with `tokio::task::spawn_blocking` without re-cloning
// at every site; the inner body holds the values across the synchronous
// decode → separate → encode pipeline.
#[allow(clippy::needless_pass_by_value)]
#[tracing::instrument(skip(library, separator, progress), fields(recording_id = %recording_id))]
pub fn separate_stems_blocking(
    library: &RecordingsLibrary,
    recordings_dir: &Path,
    recording_id: RecordingId,
    separator: Arc<StemSeparator>,
    cancel: CancellationToken,
    progress: Option<&dyn SeparateProgressSink>,
) -> Result<StemSummary, StemError> {
    // Pre-flight cancel check so a token tripped before the call ever
    // started returns the correct typed error rather than completing.
    if cancel.is_cancelled() {
        return Err(StemError::Cancelled);
    }

    // 1. Resolve the recording row. Mirrors the transcribe path's
    //    `IncludingDeleted` round-trip so a soft-deleted row is still
    //    resolvable for separation (the pointer to the FLAC is
    //    preserved by the soft-delete contract).
    let row = library
        .list_recordings(ListFilter::IncludingDeleted)
        .map_err(|e| StemError::Library(format!("{e:#}")))?
        .into_iter()
        .find(|r| r.id == recording_id)
        .ok_or(StemError::RecordingNotFound(recording_id))?;

    // 2. Cache lookup. On hit emit one terminal tick and return without
    //    touching the separator handle. The < 50 ms latency contract
    //    is structurally preserved here: a single SQLite point-lookup
    //    on the indexed `(recording_id, separator_version)` key plus a
    //    couple of `String` allocations is well under the budget on
    //    every contemporary disk.
    let cached = library
        .get_stem_result(recording_id, HTDEMUCS_SEPARATOR_VERSION)
        .map_err(|e| StemError::Library(format!("{e:#}")))?;
    if let Some(row) = cached {
        emit_terminal(progress, recording_id);
        return Ok(StemSummary {
            separator_version: HTDEMUCS_SEPARATOR_VERSION.to_string(),
            was_cached: true,
            vocals_path: row.vocals_path,
            drums_path: row.drums_path,
            bass_path: row.bass_path,
            other_path: row.other_path,
            completed_at_unix_ms: row.completed_at_unix_ms,
        });
    }

    // 3. Cache miss — decode the source.
    if cancel.is_cancelled() {
        return Err(StemError::Cancelled);
    }
    emit(progress, recording_id, SeparateStage::Decode, 0.0);
    let source_path = recordings_dir.join(&row.filename);
    let samples = decode_mono_48k(&source_path)
        .map_err(|e| StemError::DecodeFailed(format!("{e:#}: {}", source_path.display())))?;
    emit(progress, recording_id, SeparateStage::Decode, 1.0);

    if cancel.is_cancelled() {
        return Err(StemError::Cancelled);
    }

    // 4. Lazy-init the core separator (ensures the ONNX model is
    //    cached, opens an ort session) and run the four-bus split.
    //    The inner mutex is held only across the inference call so the
    //    encode step below runs without blocking concurrent
    //    re-separations on the shared `Arc<StemSeparator>`.
    let split = run_separation(&separator, &samples, &cancel, progress, recording_id)?;

    if cancel.is_cancelled() {
        return Err(StemError::Cancelled);
    }

    // 5. Encode the four stems atomically. Each FLAC is written through
    //    the existing `FlacRecordingSink` so the on-disk shape matches
    //    the live-recording pipeline byte-for-byte and the front-end's
    //    `convertFileSrc` round-trip works without an extension swap.
    emit(progress, recording_id, SeparateStage::Encode, 0.0);
    let stems_dir = recordings_dir.join(recording_id.to_string()).join("stems");
    std::fs::create_dir_all(&stems_dir)?;
    let written = match write_four_stems(&stems_dir, &split, &cancel) {
        Ok(p) => p,
        Err(e) => {
            // Best-effort cleanup of any partial FLACs left behind.
            let _ = std::fs::remove_dir_all(&stems_dir);
            return Err(e);
        }
    };
    emit(progress, recording_id, SeparateStage::Encode, 1.0);

    if cancel.is_cancelled() {
        // Cleanup partial files before surfacing the cancellation so a
        // cancelled run leaves no orphans on disk.
        let _ = std::fs::remove_dir_all(&stems_dir);
        return Err(StemError::Cancelled);
    }

    let now_ms = unix_now_ms();
    let vocals_path = path_to_string(&written.vocals)?;
    let drums_path = path_to_string(&written.drums)?;
    let bass_path = path_to_string(&written.bass)?;
    let other_path = path_to_string(&written.other)?;

    library
        .upsert_stem_result(
            recording_id,
            HTDEMUCS_SEPARATOR_VERSION,
            now_ms,
            &vocals_path,
            &drums_path,
            &bass_path,
            &other_path,
        )
        .map_err(|e| StemError::Library(format!("{e:#}")))?;

    Ok(StemSummary {
        separator_version: HTDEMUCS_SEPARATOR_VERSION.to_string(),
        was_cached: false,
        vocals_path,
        drums_path,
        bass_path,
        other_path,
        completed_at_unix_ms: now_ms,
    })
}

/// Read one stem FLAC into memory and return the raw bytes. The caller
/// (front-end) wraps the bytes into a synthetic `blob:` URL feeding the
/// existing `PlaybackPanel`. Looks up `stem_results` to resolve the
/// on-disk path, so a cancel-then-replay sequence cannot return bytes
/// from a stale FLAC.
pub fn read_stem_audio_blocking(
    library: &RecordingsLibrary,
    _recordings_dir: &Path,
    recording_id: RecordingId,
    stem: StemKind,
) -> Result<Vec<u8>, StemError> {
    let row = library
        .get_stem_result(recording_id, HTDEMUCS_SEPARATOR_VERSION)
        .map_err(|e| StemError::Library(format!("{e:#}")))?
        .ok_or(StemError::RecordingNotFound(recording_id))?;

    let path: &str = match stem {
        StemKind::Vocals => &row.vocals_path,
        StemKind::Drums => &row.drums_path,
        StemKind::Bass => &row.bass_path,
        StemKind::Other => &row.other_path,
    };
    let bytes = std::fs::read(path)?;
    Ok(bytes)
}

/// Headless twin of the `download_stem_model` Tauri command. Pulls the
/// HTDemucs ONNX from [`HTDEMUCS_MODEL_URL`] (~316 MB), verifies the
/// streaming SHA-256 against [`HTDEMUCS_MODEL_SHA256`], and atomically
/// renames the temp file into `<models_dir>/htdemucs.onnx`. Idempotent:
/// a cache hit verifies the on-disk SHA-256 and returns immediately.
pub fn download_stem_model_blocking(
    models_dir: &Path,
    progress: Option<&dyn DownloadProgressSink>,
) -> Result<(), StemError> {
    std::fs::create_dir_all(models_dir)?;
    if let Some(sink) = progress {
        sink.emit(DownloadProgress {
            bytes_downloaded: 0,
            total_bytes: neural_pitch_core::stems::HTDEMUCS_SIZE_BYTES,
            percent: 0.0,
        });
    }
    let total = neural_pitch_core::stems::HTDEMUCS_SIZE_BYTES;
    neural_pitch_core::stems::download::ensure_at(models_dir, |percent| {
        if let Some(sink) = progress {
            sink.emit(DownloadProgress {
                bytes_downloaded: ((f64::from(percent) * total as f64) as u64).min(total),
                total_bytes: total,
                percent,
            });
        }
    })?;
    if let Some(sink) = progress {
        sink.emit(DownloadProgress {
            bytes_downloaded: total,
            total_bytes: total,
            percent: 1.0,
        });
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Internal helpers
// ----------------------------------------------------------------------------

/// Result of [`write_four_stems`] — paths are absolute under
/// `<recordings_dir>/<recording_id>/stems/`.
struct WrittenStems {
    vocals: PathBuf,
    drums: PathBuf,
    bass: PathBuf,
    other: PathBuf,
}

/// In-memory four-bus split fed into [`write_four_stems`]. Each
/// `Vec<f32>` is mono PCM at [`STEM_SAMPLE_RATE_HZ`].
struct FourBusSplit {
    vocals: Vec<f32>,
    drums: Vec<f32>,
    bass: Vec<f32>,
    other: Vec<f32>,
}

/// Resolve the on-disk model cache, downloading it on first use.
/// Forwards download progress onto the supplied closure. The cancel
/// token is polled inside the streaming-read loop so a tripped token
/// short-circuits the download mid-blob rather than waiting for the
/// full ~316 MB to finish.
fn ensure_model_cached<F: FnMut(f32)>(
    separator: &StemSeparator,
    progress: F,
    cancel: &CancellationToken,
) -> Result<PathBuf, StemError> {
    let path = if let Some(dir) = separator.models_dir.as_ref() {
        neural_pitch_core::stems::StemSeparator::ensure_model_at_with_cancel(dir, progress, cancel)?
    } else {
        // The default-platform path is reserved for non-Tauri callers;
        // it does not currently expose a cancel-aware variant. The
        // shell always supplies an explicit models_dir, so this branch
        // is exercised only by the headless tests.
        neural_pitch_core::stems::StemSeparator::ensure_model(progress)?
    };
    Ok(path)
}

/// Run one HTDemucs separation pass. Holds the inner-mutex guard for
/// the duration of the inference call so two concurrent calls on the
/// same `Arc<StemSeparator>` serialise on the underlying ONNX session.
/// Lazy-initialises the core ONNX-backed separator on the first call,
/// emitting `Download` ticks while the model is being fetched.
fn run_separation(
    separator: &StemSeparator,
    samples: &[f32],
    cancel: &CancellationToken,
    progress: Option<&dyn SeparateProgressSink>,
    recording_id: RecordingId,
) -> Result<FourBusSplit, StemError> {
    let mut inner_guard = separator.inner.lock();
    if inner_guard.is_none() {
        emit(progress, recording_id, SeparateStage::Download, 0.0);
        let model_path = ensure_model_cached(
            separator,
            |p| {
                emit(progress, recording_id, SeparateStage::Download, p);
            },
            cancel,
        )?;
        emit(progress, recording_id, SeparateStage::Download, 1.0);
        *inner_guard = Some(neural_pitch_core::stems::StemSeparator::open(&model_path)?);
    }
    let core = inner_guard
        .as_mut()
        .ok_or_else(|| StemError::SeparatorFailed("core separator missing".to_string()))?;

    emit(progress, recording_id, SeparateStage::Separate, 0.0);
    separator.invocation_count.fetch_add(1, Ordering::Relaxed);
    let stems = core.separate(
        samples,
        STEM_SAMPLE_RATE_HZ,
        STEM_CHANNELS,
        |p| emit(progress, recording_id, SeparateStage::Separate, p),
        cancel,
    )?;
    emit(progress, recording_id, SeparateStage::Separate, 1.0);
    Ok(FourBusSplit {
        vocals: stems.vocals,
        drums: stems.drums,
        bass: stems.bass,
        other: stems.other,
    })
}

/// Encode each of the four stems through the existing
/// `FlacRecordingSink`. The sink enforces 48 kHz / mono / 24-bit on its
/// own; the core separator returns mono buffers matching that shape
/// when invoked with `channels = 1`.
fn write_four_stems(
    stems_dir: &Path,
    split: &FourBusSplit,
    cancel: &CancellationToken,
) -> Result<WrittenStems, StemError> {
    let written = WrittenStems {
        vocals: stems_dir.join("vocals.flac"),
        drums: stems_dir.join("drums.flac"),
        bass: stems_dir.join("bass.flac"),
        other: stems_dir.join("other.flac"),
    };

    write_one_stem(&written.vocals, &split.vocals, cancel)?;
    if cancel.is_cancelled() {
        return Err(StemError::Cancelled);
    }
    write_one_stem(&written.drums, &split.drums, cancel)?;
    if cancel.is_cancelled() {
        return Err(StemError::Cancelled);
    }
    write_one_stem(&written.bass, &split.bass, cancel)?;
    if cancel.is_cancelled() {
        return Err(StemError::Cancelled);
    }
    write_one_stem(&written.other, &split.other, cancel)?;

    Ok(written)
}

/// Per-stem encode chunk size, in samples per call into the FLAC sink.
///
/// ~1 s of audio at the locked 48 kHz rate; the cancellation token is
/// polled between chunks so an in-flight cancel mid-encode short-
/// circuits within the documented <500 ms budget.
const ENCODE_CHUNK_SAMPLES: usize = STEM_SAMPLE_RATE_HZ as usize;

/// Per-stem chunked encoder.
///
/// Splits the encode into ~1 s chunks at 48 kHz so the cancellation
/// token can be polled mid-encode — without this an in-flight ~10 min
/// stem would burn through its full encode wall-clock cost before the
/// outer cancel-between-stems check fired, blowing the documented
/// <500 ms cancellation budget.
fn write_one_stem(
    path: &Path,
    samples: &[f32],
    cancel: &CancellationToken,
) -> Result<(), StemError> {
    let mut sink = FlacRecordingSink::create(path, STEM_SAMPLE_RATE_HZ)
        .map_err(|e| StemError::EncodeFailed(format!("create {}: {e:#}", path.display())))?;
    let mut idx = 0;
    while idx < samples.len() {
        if cancel.is_cancelled() {
            // The partially-written FLAC is removed by the caller's
            // `remove_dir_all(stems_dir)` cleanup path, so we only need
            // to surface the cancellation here.
            return Err(StemError::Cancelled);
        }
        let end = (idx + ENCODE_CHUNK_SAMPLES).min(samples.len());
        sink.write(&samples[idx..end])
            .map_err(|e| StemError::EncodeFailed(format!("write {}: {e:#}", path.display())))?;
        idx = end;
    }
    let boxed: Box<dyn RecordingSink> = Box::new(sink);
    boxed
        .finalize()
        .map_err(|e| StemError::EncodeFailed(format!("finalize {}: {e:#}", path.display())))?;
    Ok(())
}

/// Decode a 16-bit / 24-bit / float-32 PCM WAV or FLAC into a mono
/// `f32` buffer at 48 kHz. The recording pipeline is locked to 48 kHz
/// so the source rate is implicit; a future heterogeneous-rate import
/// path adds a `rubato` resample stage here without changing the public
/// surface.
fn decode_mono_48k(path: &Path) -> Result<Vec<f32>, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "wav" => decode_wav_mono_48k(path),
        // FLAC decode is wired through `claxon` (already a transitive
        // dep behind `feature = "pyin"`); the live recording path emits
        // FLAC so this arm covers production-recorded sources.
        "flac" => decode_flac_mono_48k(path),
        other => Err(format!("unsupported source extension: {other}")),
    }
}

fn decode_wav_mono_48k(path: &Path) -> Result<Vec<f32>, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let mut header = [0_u8; 12];
    f.read_exact(&mut header)
        .map_err(|e| format!("riff: {e}"))?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".to_string());
    }

    let mut sample_rate_hz: u32 = 0;
    let mut channels: u16 = 0;
    let mut bits_per_sample: u16 = 0;
    let mut format_tag: u16 = 0;
    let mut data_payload: Vec<u8> = Vec::new();
    let mut found_fmt = false;
    let mut found_data = false;

    loop {
        let mut chunk_header = [0_u8; 8];
        if f.read_exact(&mut chunk_header).is_err() {
            break;
        }
        let id = [
            chunk_header[0],
            chunk_header[1],
            chunk_header[2],
            chunk_header[3],
        ];
        let size = u32::from_le_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]);

        if &id == b"fmt " {
            let mut fmt = vec![0_u8; size as usize];
            f.read_exact(&mut fmt).map_err(|e| format!("fmt: {e}"))?;
            if fmt.len() < 16 {
                return Err("fmt chunk too short".to_string());
            }
            format_tag = u16::from_le_bytes([fmt[0], fmt[1]]);
            channels = u16::from_le_bytes([fmt[2], fmt[3]]);
            sample_rate_hz = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
            bits_per_sample = u16::from_le_bytes([fmt[14], fmt[15]]);
            found_fmt = true;
        } else if &id == b"data" {
            let mut payload = vec![0_u8; size as usize];
            f.read_exact(&mut payload)
                .map_err(|e| format!("data: {e}"))?;
            data_payload = payload;
            found_data = true;
            break;
        } else {
            let skip = u64::from(size) + u64::from(size & 1);
            std::io::copy(&mut (&f).take(skip), &mut std::io::sink())
                .map_err(|e| format!("skip: {e}"))?;
        }
    }
    if !found_fmt || !found_data {
        return Err("missing fmt or data chunk".to_string());
    }
    if sample_rate_hz != STEM_SAMPLE_RATE_HZ {
        return Err(format!(
            "stem separator requires {STEM_SAMPLE_RATE_HZ} Hz source; got {sample_rate_hz} Hz"
        ));
    }

    Ok(match (format_tag, bits_per_sample) {
        (1, 16) => decode_pcm16_mono(&data_payload, channels),
        (1, 24) => decode_pcm24_mono(&data_payload, channels),
        (3, 32) => decode_float32_mono(&data_payload, channels),
        (tag, bits) => return Err(format!("unsupported wav format-tag {tag} / {bits}-bit")),
    })
}

#[cfg(feature = "neural")]
fn decode_flac_mono_48k(path: &Path) -> Result<Vec<f32>, String> {
    // FLAC decode via `claxon` (already a transitive dep behind
    // `feature = "pyin"` for the contour analyser; surfaced here as a
    // first-class dep so the live-recording flow's FLAC sources can
    // actually feed the stem separator). Down-mixes interleaved
    // multi-channel sources to mono and verifies the source rate
    // matches the locked 48 kHz internal rate.
    let mut reader = claxon::FlacReader::open(path).map_err(|e| format!("open flac: {e:#}"))?;
    let info = reader.streaminfo();
    if info.sample_rate != STEM_SAMPLE_RATE_HZ {
        return Err(format!(
            "stem separator requires {STEM_SAMPLE_RATE_HZ} Hz source; got {} Hz",
            info.sample_rate
        ));
    }
    let bits = info.bits_per_sample;
    let scale = (((1_u64 << bits.saturating_sub(1)) as f32).max(1.0)).recip();
    let channels = usize::try_from(info.channels.max(1)).unwrap_or(1);
    let mut samples: Vec<f32> = Vec::with_capacity(info.samples.unwrap_or(0) as usize);
    if channels == 1 {
        for s in reader.samples() {
            let v = s.map_err(|e| format!("decode flac: {e:#}"))?;
            samples.push((v as f32) * scale);
        }
    } else {
        // claxon emits interleaved i32 frames; down-mix to mono.
        let channels_i64 = i64::try_from(channels).unwrap_or(1);
        let mut acc: i64 = 0;
        let mut idx: usize = 0;
        for s in reader.samples() {
            let v = s.map_err(|e| format!("decode flac: {e:#}"))?;
            acc += i64::from(v);
            idx += 1;
            if idx == channels {
                let mono = (acc / channels_i64) as f32 * scale;
                samples.push(mono);
                acc = 0;
                idx = 0;
            }
        }
    }
    Ok(samples)
}

fn decode_pcm16_mono(bytes: &[u8], channels: u16) -> Vec<f32> {
    let bytes_per_frame = 2 * usize::from(channels.max(1));
    if bytes_per_frame == 0 {
        return Vec::new();
    }
    let n_frames = bytes.len() / bytes_per_frame;
    let mut out = Vec::with_capacity(n_frames);
    let inv = 1.0_f32 / f32::from(i16::MAX);
    for frame in 0..n_frames {
        let mut sum = 0.0_f32;
        for ch in 0..usize::from(channels.max(1)) {
            let off = frame * bytes_per_frame + ch * 2;
            let s = i16::from_le_bytes([bytes[off], bytes[off + 1]]);
            sum += f32::from(s) * inv;
        }
        out.push(sum / f32::from(channels.max(1)));
    }
    out
}

fn decode_pcm24_mono(bytes: &[u8], channels: u16) -> Vec<f32> {
    let bytes_per_frame = 3 * usize::from(channels.max(1));
    if bytes_per_frame == 0 {
        return Vec::new();
    }
    let n_frames = bytes.len() / bytes_per_frame;
    let mut out = Vec::with_capacity(n_frames);
    let inv = 1.0_f32 / 8_388_608.0_f32;
    for frame in 0..n_frames {
        let mut sum = 0.0_f32;
        for ch in 0..usize::from(channels.max(1)) {
            let off = frame * bytes_per_frame + ch * 3;
            let raw = i32::from(bytes[off])
                | (i32::from(bytes[off + 1]) << 8)
                | (i32::from(bytes[off + 2]) << 16);
            let signed = if raw & 0x0080_0000 != 0 {
                raw | -0x0100_0000
            } else {
                raw
            };
            sum += signed as f32 * inv;
        }
        out.push(sum / f32::from(channels.max(1)));
    }
    out
}

fn decode_float32_mono(bytes: &[u8], channels: u16) -> Vec<f32> {
    let bytes_per_frame = 4 * usize::from(channels.max(1));
    if bytes_per_frame == 0 {
        return Vec::new();
    }
    let n_frames = bytes.len() / bytes_per_frame;
    let mut out = Vec::with_capacity(n_frames);
    for frame in 0..n_frames {
        let mut sum = 0.0_f32;
        for ch in 0..usize::from(channels.max(1)) {
            let off = frame * bytes_per_frame + ch * 4;
            let s =
                f32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
            sum += s;
        }
        out.push(sum / f32::from(channels.max(1)));
    }
    out
}

fn emit(
    progress: Option<&dyn SeparateProgressSink>,
    recording_id: RecordingId,
    stage: SeparateStage,
    percent: f32,
) {
    if let Some(sink) = progress {
        sink.emit(SeparateProgress {
            recording_id: recording_id.to_string(),
            stage,
            percent,
        });
    }
}

fn emit_terminal(progress: Option<&dyn SeparateProgressSink>, recording_id: RecordingId) {
    emit(progress, recording_id, SeparateStage::Encode, 1.0);
}

fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn path_to_string(p: &Path) -> Result<String, StemError> {
    p.to_str()
        .map(str::to_string)
        .ok_or_else(|| StemError::EncodeFailed(format!("non-utf8 stem path: {}", p.display())))
}
