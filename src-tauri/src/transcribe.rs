//! Phase 3 — file-import / Basic Pitch transcribe / MIDI export shell.
//!
//! Three new IPC entry points live here as pure-blocking headless twins so
//! the Tauri command layer in [`crate::commands`] can `spawn_blocking` them
//! and the integration tests under `tests/` can call them directly without
//! standing up a full Tauri runtime (mirrors the
//! [`neural_pitch_core::store::analyze_recording_blocking`] pattern that
//! Phase 2.1 ships).
//!
//! 1. [`import_audio_file_blocking`] — extension-gated to
//!    `{wav, flac, mp3}`, stat-rejects sources `>` [`IMPORT_SIZE_LIMIT_BYTES`],
//!    parses the WAV header inline (RIFF/WAVE/fmt /data) for the
//!    sample-rate / channels / duration probe, copies the source bytes into
//!    `{recordings_dir}/imports/<uuid>.<ext>`, and stamps a row with
//!    `instrument_profile = "Imported"`.
//!
//! 2. [`transcribe_recording_blocking`] — looks up the recording row, hits
//!    the `analysis_cache` (`("basic-pitch", "1.0")` key), and on a cache
//!    miss decodes the on-disk WAV, runs a deterministic mono pitch + onset
//!    extractor over the buffer, persists the postcard-encoded
//!    [`WirePolyResult`], and returns [`TranscribeSummary`].
//!
//! 3. [`export_midi_blocking`] — postcard-decodes the cached blob, emits an
//!    SMF type-1 byte stream via the always-on `midly` dependency, and
//!    atomically writes it to `dest_path` (`<dest>.partial` → `fsync` →
//!    `rename`).
//!
//! The whole module is gated behind `feature = "neural"` at the
//! `mod transcribe;` declaration in `lib.rs`; no inner `#![cfg]` is
//! needed here.

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use midly::num::{u4, u7, u15, u24, u28};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};
use neural_pitch_core::store::{
    ListFilter, NewRecording, Recording, RecordingId, RecordingsLibrary,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable analyzer name persisted in `analysis_cache.analyzer_name` for
/// the Basic Pitch transcribe surface. Matches
/// [`neural_pitch_core::poly::basic_pitch::BasicPitchEstimator::name`]
/// minus the `-v1` suffix so the cache key reads the same as the on-the-
/// wire IPC discriminant a future `BackendKind::BasicPitch` arm will use.
pub const BASIC_PITCH_ANALYZER_NAME: &str = "basic-pitch";

/// Stable analyzer version persisted in
/// `analysis_cache.analyzer_version`. Bump in lock-step with any
/// [`WirePolyResult`] schema change so cached blobs invalidate cleanly.
pub const BASIC_PITCH_ANALYZER_VERSION: &str = "1.0";

/// Hard cap on the size of an importable audio file. 500 MiB matches the
/// Phase 3 spec — at 24-bit / 48 kHz / mono FLAC this is roughly 2 hours
/// of material; lossless captures longer than that should be edited
/// outside the app.
pub const IMPORT_SIZE_LIMIT_BYTES: u64 = 500 * 1024 * 1024;

/// MIDI tempo persisted in the exported SMF. Matches the Phase 3 spec's
/// 120 BPM default — `60_000_000 / tempo_bpm` µs/quarter.
const TEMPO_BPM: u32 = 120;

/// Ticks per quarter note in the exported SMF. 480 PPQ is the Logic /
/// Reaper default and round-trips cleanly through the millisecond → tick
/// conversion at the contour frame rate.
const TICKS_PER_QUARTER: u16 = 480;

/// Per-tick progress message emitted on the transcribe channel.
///
/// Cached path emits exactly one message with `percent: 1.0`,
/// `current_window == total_windows`. Fresh runs emit one tick per inference
/// window the deterministic transcriber processes (single window in the
/// Phase 3 baseline) so the front-end progress UI ticks exactly once per
/// audio buffer regardless of cache vs. fresh path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TranscribeProgress {
    /// Stringified [`RecordingId`] of the recording being transcribed.
    pub recording_id: String,
    /// Progress in `[0.0, 1.0]`.
    pub percent: f32,
    /// Inference windows already processed.
    pub current_window: u64,
    /// Total inference windows the model expects to run.
    pub total_windows: u64,
}

/// Wire summary returned by [`transcribe_recording_blocking`].
///
/// Mirrors the [`neural_pitch_core::store::AnalysisSummary`] shape so the
/// front-end can re-use its existing `normaliseSummary` adapter:
/// `was_cached` discriminates the cache-hit fast path, `note_count`
/// surfaces the count of MIDI notes the basic-pitch model recovered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TranscribeSummary {
    /// Stable analyzer identifier (`"basic-pitch"`).
    pub analyzer_name: String,
    /// Analyzer version string (`"1.0"` for Phase 3 baseline).
    pub analyzer_version: String,
    /// `true` when this summary came from `analysis_cache`; the inference
    /// run was skipped.
    pub was_cached: bool,
    /// Number of `NoteEvent`s recovered from the audio buffer.
    pub note_count: u64,
    /// Total duration of the analysed audio buffer, in milliseconds.
    pub duration_ms: u64,
    /// Wall-clock time the analysis completed (or was first cached), in
    /// Unix milliseconds. Mirrors `analysis_cache.computed_at_unix_ms`.
    pub computed_at_unix_ms: i64,
}

/// Sink trait fed by [`transcribe_recording_blocking`] so the Tauri shell
/// can adapt a `tauri::ipc::Channel<TranscribeProgress>` without dragging
/// Tauri types into this module. The blocking analyzer runs inside
/// `spawn_blocking`, so any `Channel::send` happens off the tokio runtime
/// thread — RT-safety properties match
/// [`neural_pitch_core::store::ProgressSink`].
pub trait TranscribeProgressSink: Send + Sync {
    /// Emit one progress tick. Implementations MUST tolerate the receiver
    /// closing early (`Result::is_err()` on the underlying `Channel::send`
    /// is logged at `debug!` and otherwise ignored), matching the
    /// `start_recording` contract.
    fn emit(&self, progress: TranscribeProgress);
}

/// Postcard wire shape persisted to `analysis_cache.result_blob`.
///
/// Decoupled from `neural_pitch_core::poly::PolyResult` because the upstream
/// type does not derive serde traits and the IPC layer must not reach into
/// the poly module. Conversion is one-way (transcribe → wire) at write time
/// and trivial at read time (the export path operates on the wire shape
/// directly via [`wire_to_smf`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WirePolyResult {
    /// Note events recovered from the buffer. Ordered by `start_ms` so the
    /// downstream MIDI emitter can walk in playback order.
    notes: Vec<WireNoteEvent>,
    /// Total duration of the analysed audio buffer, in milliseconds.
    duration_ms: u64,
}

/// One note event recovered from the deterministic transcribe pass. Mirrors
/// `neural_pitch_core::poly::NoteEvent` minus the optional pitch-bend curve
/// (Phase 3 baseline does not emit per-note pitch bend; the curve hook lives
/// in [`crate::commands::export_midi`] when the algo team's
/// `BasicPitchEstimator` ships its variant).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireNoteEvent {
    /// MIDI note number (`0..=127`).
    midi: u8,
    /// Onset timestamp, in milliseconds since the start of the buffer.
    start_ms: u64,
    /// Offset timestamp, in milliseconds. Always strictly greater than
    /// `start_ms`.
    end_ms: u64,
    /// MIDI velocity in `1..=127` (a recovered note is never velocity 0).
    velocity: u8,
}

/// Typed error surface for [`import_audio_file_blocking`].
///
/// All variants flatten at the IPC boundary via `format!("{e:#}")`. The
/// front-end keeps regex-free `match` on prefix substrings *only* in the
/// (rare) case it needs to discriminate `already imported` for UX —
/// otherwise the message is purely user-facing copy.
#[derive(Debug, Error)]
pub enum ImportError {
    /// File extension is not in the gated `{wav, flac, mp3}` set.
    #[error("unsupported extension: {0}")]
    UnsupportedExtension(String),
    /// Header parse / decode of the source file failed.
    #[error("decode failed: {0}")]
    DecodeFailed(String),
    /// Filesystem I/O failure while probing or copying the source.
    #[error(transparent)]
    IoError(#[from] std::io::Error),
    /// Source file exceeds the [`IMPORT_SIZE_LIMIT_BYTES`] hard cap.
    #[error("file too large: {bytes} > 500 MiB")]
    TooLarge {
        /// Observed file size in bytes (from `std::fs::metadata`).
        bytes: u64,
    },
    /// Persistence-layer error from `RecordingsLibrary::insert_recording`.
    #[error("library insert failed: {0}")]
    LibraryInsert(String),
}

/// Typed error surface for [`transcribe_recording_blocking`] /
/// [`export_midi_blocking`].
#[derive(Debug, Error)]
pub enum TranscribeError {
    /// The supplied recording id did not resolve through
    /// `library.list_recordings(IncludingDeleted)`.
    #[error("recording not found: {0}")]
    RecordingNotFound(RecordingId),
    /// Decode of the on-disk audio file failed during the inference path.
    #[error("decode failed: {0}")]
    DecodeFailed(String),
    /// Inference path returned no audio buffer to analyse.
    #[error("analyzer failed: {0}")]
    AnalyzerFailed(String),
    /// No `analysis_cache` row exists for `(recording_id, "basic-pitch",
    /// "1.0")`; the caller MUST run [`transcribe_recording_blocking`]
    /// before [`export_midi_blocking`].
    #[error("transcribe first")]
    NotTranscribed,
    /// Postcard decode of the cached result blob failed — a schema-version
    /// mismatch or row corruption.
    #[error("cache corrupted: {0}")]
    CacheCorrupted(String),
    /// MIDI emission failed (e.g. `midly::Smf::write`).
    #[error("midi emit failed: {0}")]
    MidiEmitFailed(String),
    /// Persistence-layer error from `library.upsert_analysis(..)` or the
    /// SQLite read path.
    #[error("library failure: {0}")]
    Library(String),
    /// `std::fs` error during the atomic write of the SMF bytes.
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

/// Headless twin of the `import_audio_file` Tauri command.
///
/// Workflow:
/// 1. Lower-case extension; reject anything outside `{wav, flac, mp3}`.
/// 2. `metadata()` for size; reject `> IMPORT_SIZE_LIMIT_BYTES` *before*
///    opening the file (cheap rejection — no decoder spin-up cost).
/// 3. Probe the WAV header inline (RIFF/WAVE/fmt /data). The Phase 3
///    baseline only ships the WAV path; FLAC / MP3 probes are reserved for
///    a follow-up that pulls in `claxon` / `minimp3`.
/// 4. Mint a [`RecordingId`], copy the source file to
///    `{recordings_dir}/imports/<uuid>.<ext>` (preserves bytes),
/// 5. Insert a `NewRecording` with `instrument_profile = "Imported"`,
///    `a4_hz = 440.0`, the probed sample-rate / channels / duration, and
///    return the new id.
pub fn import_audio_file_blocking(
    library: &RecordingsLibrary,
    recordings_dir: &Path,
    source_path: &Path,
) -> Result<RecordingId, ImportError> {
    // 1) Extension gate.
    let ext = source_path
        .extension()
        .and_then(|os| os.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !matches!(ext.as_str(), "wav" | "flac" | "mp3") {
        return Err(ImportError::UnsupportedExtension(ext));
    }

    // 2) Stat-reject oversized sources before opening.
    let meta = std::fs::metadata(source_path)?;
    if meta.len() > IMPORT_SIZE_LIMIT_BYTES {
        return Err(ImportError::TooLarge { bytes: meta.len() });
    }

    // 3) Probe. Only the WAV path is wired for the Phase 3 baseline; the
    //    FLAC / MP3 arms surface a `decode failed` error that the front-end
    //    can coerce into the "format-not-yet-supported" copy until the
    //    follow-up resampler lands.
    let probe = match ext.as_str() {
        "wav" => probe_wav(source_path)?,
        other => {
            return Err(ImportError::DecodeFailed(format!(
                "{other} probe not yet wired (Phase 3 ships WAV only)"
            )));
        }
    };

    // 4) Mint a *file* UUIDv7 up-front so we can lock in the on-disk
    //    filename column before SQLite mints its own row id. The row id
    //    and the file id are independent — the library's
    //    `insert_recording` hardcodes a fresh UUIDv7 mint internally, so
    //    we cannot reuse one identifier for both. The row's `filename`
    //    column points at the file id, the row's `id` is the SQLite-
    //    minted one. This is the same shape the live-capture path uses
    //    (see `commands::start_recording`, which mints two UUIDs as
    //    well).
    let imports_dir = recordings_dir.join("imports");
    std::fs::create_dir_all(&imports_dir)?;

    let file_id = RecordingId::new_v7();
    let dest_filename = format!("{file_id}.{ext}");
    let dest_path = imports_dir.join(&dest_filename);
    std::fs::copy(source_path, &dest_path)?;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    let row_filename = format!("imports/{dest_filename}");

    let id = library
        .insert_recording(NewRecording {
            filename: row_filename,
            created_at_unix_ms: now_ms,
            duration_ms: i64::try_from(probe.duration_ms).unwrap_or(i64::MAX),
            sample_rate_hz: i64::from(probe.sample_rate_hz),
            channels: i64::from(probe.channels),
            // 0 sentinel for lossy / unknown; WAV probe could populate this
            // but the Phase 3 spec deliberately keeps it as a sentinel so
            // the row-shape matches future FLAC / MP3 imports without an
            // append-only schema change.
            bit_depth: 0,
            format: ext,
            a4_hz: 440.0,
            instrument_profile: "Imported".to_string(),
            user_label: None,
        })
        .map_err(|e| {
            // Best-effort cleanup of the orphan file copy on insert failure.
            // We deliberately ignore the unlink error — surfacing the
            // original library error is more useful to the caller than
            // the secondary cleanup error.
            let _ = std::fs::remove_file(&dest_path);
            ImportError::LibraryInsert(format!("{e:#}"))
        })?;

    Ok(id)
}

/// Probed metadata returned by [`probe_wav`].
struct AudioProbe {
    sample_rate_hz: u32,
    channels: u16,
    duration_ms: u64,
}

/// Inline RIFF/WAVE/fmt /data parser.
///
/// Skips unknown chunks (LIST, bext, etc.) until it finds the `fmt ` and
/// `data` chunks. Only PCM (format-tag 1) is accepted — IEEE float WAVs
/// (format-tag 3) and other extension formats are rejected as
/// `DecodeFailed` so the front-end can surface a clear "unsupported
/// codec" message rather than silently mis-parse the sample data.
fn probe_wav(path: &Path) -> Result<AudioProbe, ImportError> {
    let mut f = std::fs::File::open(path)?;
    let mut header = [0_u8; 12];
    f.read_exact(&mut header)
        .map_err(|e| ImportError::DecodeFailed(format!("RIFF header read: {e}")))?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err(ImportError::DecodeFailed(
            "not a RIFF/WAVE file".to_string(),
        ));
    }

    let mut sample_rate_hz: u32 = 0;
    let mut channels: u16 = 0;
    let mut block_align: u16 = 0;
    let mut bits_per_sample: u16 = 0;
    let mut data_size: u64 = 0;
    let mut found_fmt = false;
    let mut found_data = false;

    loop {
        let mut chunk_header = [0_u8; 8];
        if let Err(e) = f.read_exact(&mut chunk_header) {
            // EOF before we found the data chunk is a hard error.
            if !found_data {
                return Err(ImportError::DecodeFailed(format!(
                    "EOF before data chunk: {e}"
                )));
            }
            break;
        }
        let id = &chunk_header[0..4];
        let size = u32::from_le_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]);

        if id == b"fmt " {
            let mut fmt = vec![0_u8; size as usize];
            f.read_exact(&mut fmt)
                .map_err(|e| ImportError::DecodeFailed(format!("fmt chunk read: {e}")))?;
            if fmt.len() < 16 {
                return Err(ImportError::DecodeFailed("fmt chunk too short".to_string()));
            }
            let format_tag = u16::from_le_bytes([fmt[0], fmt[1]]);
            // 1 = PCM, 3 = IEEE float, 0xFFFE = WAVE_FORMAT_EXTENSIBLE.
            if format_tag != 1 && format_tag != 3 {
                return Err(ImportError::DecodeFailed(format!(
                    "unsupported wav format-tag: {format_tag}"
                )));
            }
            channels = u16::from_le_bytes([fmt[2], fmt[3]]);
            sample_rate_hz = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
            block_align = u16::from_le_bytes([fmt[12], fmt[13]]);
            bits_per_sample = u16::from_le_bytes([fmt[14], fmt[15]]);
            found_fmt = true;
        } else if id == b"data" {
            data_size = u64::from(size);
            found_data = true;
            // We don't need to read the data here — duration is purely a
            // function of the data-chunk size + format params. Skip the
            // payload so a future loop iteration could pick up trailing
            // chunks without having to seek.
            std::io::copy(&mut (&f).take(u64::from(size)), &mut std::io::sink())
                .map_err(|e| ImportError::DecodeFailed(format!("data chunk skip: {e}")))?;
            // Most WAVs end at the data chunk; exit early so the loop does
            // not attempt to parse trailing junk on producers that emit
            // padding.
            break;
        } else {
            // Skip unknown chunk. WAV chunks pad to even bytes; honour the
            // pad on odd `size`.
            let skip = u64::from(size) + u64::from(size & 1);
            std::io::copy(&mut (&f).take(skip), &mut std::io::sink())
                .map_err(|e| ImportError::DecodeFailed(format!("skip chunk {id:?}: {e}")))?;
        }
    }

    if !found_fmt || !found_data {
        return Err(ImportError::DecodeFailed(
            "missing fmt or data chunk".to_string(),
        ));
    }
    if sample_rate_hz == 0 || channels == 0 || block_align == 0 || bits_per_sample == 0 {
        return Err(ImportError::DecodeFailed(
            "invalid fmt parameters".to_string(),
        ));
    }

    let n_frames = data_size / u64::from(block_align);
    // duration_ms = n_frames * 1000 / sample_rate
    let duration_ms = n_frames
        .saturating_mul(1_000)
        .checked_div(u64::from(sample_rate_hz))
        .unwrap_or(0);

    Ok(AudioProbe {
        sample_rate_hz,
        channels,
        duration_ms,
    })
}

/// Headless twin of the `transcribe_recording` Tauri command.
///
/// Cache hit + `!force_refresh`: postcard-decode the existing
/// `analysis_cache` row, emit one terminal `TranscribeProgress` tick, and
/// return [`TranscribeSummary`] with `was_cached = true`.
///
/// Cache miss / `force_refresh`: open the WAV under `recordings_dir`,
/// run a deterministic mono pitch-and-onset extractor on the buffer to
/// produce one or more [`WireNoteEvent`]s, postcard-encode the result,
/// upsert via `library.upsert_analysis(..)`, and return
/// [`TranscribeSummary`] with `was_cached = false`.
///
/// `stem_kind` keys the analysis cache on the four-tuple
/// `(recording_id, "basic-pitch", "1.0", stem_kind)`. `None` round-trips
/// as SQL NULL — the un-stemmed full-mix transcribe path — and matches
/// every pre-V0003 row verbatim. `Some(StemKind::*)` distinguishes per-
/// stem cache entries so transcribing the vocals stem does not clobber
/// the previously-cached full-mix transcription. The on-disk source path
/// is also re-routed when `stem_kind` is set: instead of the recording's
/// top-level FLAC, the helper reads
/// `<recordings_dir>/<recording_id>/stems/<slug>.flac` (the same path
/// `separate_stems_blocking` writes).
pub fn transcribe_recording_blocking(
    library: &RecordingsLibrary,
    recordings_dir: &Path,
    recording_id: RecordingId,
    force_refresh: bool,
    stem_kind: Option<crate::stems::StemKind>,
    progress: Option<&dyn TranscribeProgressSink>,
) -> Result<TranscribeSummary, TranscribeError> {
    let row = resolve_recording(library, recording_id)?;

    let cache_kind = stem_kind.map(crate::stems::StemKind::cache_discriminant);

    // Cache lookup. On hit + !force_refresh, decode the meta row only and
    // emit a terminal tick.
    if !force_refresh {
        let blob = library
            .get_analysis_for_stem(
                recording_id,
                BASIC_PITCH_ANALYZER_NAME,
                BASIC_PITCH_ANALYZER_VERSION,
                cache_kind,
            )
            .map_err(|e| TranscribeError::Library(format!("{e:#}")))?;
        if let Some(blob) = blob {
            let wire: WirePolyResult = postcard::from_bytes(&blob)
                .map_err(|e| TranscribeError::CacheCorrupted(format!("{e:#}")))?;
            let meta = library
                .get_analysis_meta_for_stem(
                    recording_id,
                    BASIC_PITCH_ANALYZER_NAME,
                    BASIC_PITCH_ANALYZER_VERSION,
                    cache_kind,
                )
                .map_err(|e| TranscribeError::Library(format!("{e:#}")))?;
            let computed_at_unix_ms = meta.map_or(0, |(ts, _)| ts);
            emit_terminal(progress, recording_id);
            return Ok(TranscribeSummary {
                analyzer_name: BASIC_PITCH_ANALYZER_NAME.to_string(),
                analyzer_version: BASIC_PITCH_ANALYZER_VERSION.to_string(),
                was_cached: true,
                note_count: u64::try_from(wire.notes.len()).unwrap_or(u64::MAX),
                duration_ms: wire.duration_ms,
                computed_at_unix_ms,
            });
        }
    }

    // Cache miss — decode the WAV and run the deterministic transcriber.
    // For the un-stemmed (full-mix) path the source is the recording's
    // top-level file; for a per-stem call we re-route to the FLAC the
    // separator wrote under <recording_id>/stems/<slug>.flac.
    let path = match stem_kind {
        Some(kind) => recordings_dir
            .join(recording_id.to_string())
            .join("stems")
            .join(format!("{}.flac", kind.slug())),
        None => recordings_dir.join(&row.filename),
    };
    let (samples, sample_rate_hz) =
        decode_audio_mono(&path).map_err(|e| TranscribeError::DecodeFailed(format!("{e:#}")))?;
    if samples.is_empty() {
        return Err(TranscribeError::AnalyzerFailed(
            "audio buffer is empty".to_string(),
        ));
    }
    let duration_ms = (samples.len() as u64).saturating_mul(1_000) / u64::from(sample_rate_hz);

    let notes = transcribe_buffer(&samples, sample_rate_hz, duration_ms);
    let wire = WirePolyResult { notes, duration_ms };
    let blob = postcard::to_allocvec(&wire)
        .map_err(|e| TranscribeError::AnalyzerFailed(format!("postcard encode: {e:#}")))?;

    library
        .upsert_analysis_for_stem(
            recording_id,
            BASIC_PITCH_ANALYZER_NAME,
            BASIC_PITCH_ANALYZER_VERSION,
            cache_kind,
            &blob,
        )
        .map_err(|e| TranscribeError::Library(format!("{e:#}")))?;

    let meta = library
        .get_analysis_meta_for_stem(
            recording_id,
            BASIC_PITCH_ANALYZER_NAME,
            BASIC_PITCH_ANALYZER_VERSION,
            cache_kind,
        )
        .map_err(|e| TranscribeError::Library(format!("{e:#}")))?;
    let computed_at_unix_ms = meta.map_or(0, |(ts, _)| ts);
    let note_count = u64::try_from(wire.notes.len()).unwrap_or(u64::MAX);

    emit_terminal(progress, recording_id);

    Ok(TranscribeSummary {
        analyzer_name: BASIC_PITCH_ANALYZER_NAME.to_string(),
        analyzer_version: BASIC_PITCH_ANALYZER_VERSION.to_string(),
        was_cached: false,
        note_count,
        duration_ms,
        computed_at_unix_ms,
    })
}

/// Headless twin of the `export_midi` Tauri command.
///
/// Postcard-decodes the cached `(recording_id, "basic-pitch", "1.0")` blob,
/// emits an SMF type-1 byte stream via `midly`, and atomically writes the
/// bytes to `dest_path` (`<dest>.partial` → `fsync` → `rename`). Returns
/// `bytes_written`.
pub fn export_midi_blocking(
    library: &RecordingsLibrary,
    recording_id: RecordingId,
    dest_path: &Path,
) -> Result<u64, TranscribeError> {
    let blob = library
        .get_analysis(
            recording_id,
            BASIC_PITCH_ANALYZER_NAME,
            BASIC_PITCH_ANALYZER_VERSION,
        )
        .map_err(|e| TranscribeError::Library(format!("{e:#}")))?
        .ok_or(TranscribeError::NotTranscribed)?;
    let wire: WirePolyResult = postcard::from_bytes(&blob)
        .map_err(|e| TranscribeError::CacheCorrupted(format!("{e:#}")))?;

    let smf_bytes = wire_to_smf(&wire)?;

    // Atomic write: <dest>.partial → fsync → rename. Mirrors the FLAC sink's
    // contract so a crashed export does not leave a half-written .mid file
    // sitting at the user's chosen destination.
    let partial = partial_path(dest_path);
    {
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&partial)?;
        f.write_all(&smf_bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&partial, dest_path)?;

    Ok(u64::try_from(smf_bytes.len()).unwrap_or(u64::MAX))
}

/// Resolve a recording id by walking `list_recordings(IncludingDeleted)`
/// (mirrors the existing `get_recording_path` IPC command).
fn resolve_recording(
    library: &RecordingsLibrary,
    id: RecordingId,
) -> Result<Recording, TranscribeError> {
    let rows = library
        .list_recordings(ListFilter::IncludingDeleted)
        .map_err(|e| TranscribeError::Library(format!("{e:#}")))?;
    rows.into_iter()
        .find(|r| r.id == id)
        .ok_or(TranscribeError::RecordingNotFound(id))
}

/// Route to the right per-extension decoder so the per-stem transcribe
/// path (which reads FLAC) and the full-mix transcribe path (which
/// reads WAV from the import flow) share one entry point.
fn decode_audio_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "wav" => decode_wav_mono(path),
        "flac" => decode_flac_mono(path),
        other => Err(format!("unsupported source extension: {other}")),
    }
}

/// Decode a FLAC into a mono `f32` buffer at the source sample rate.
/// Multi-channel sources are mono-summed (mean of channels).
fn decode_flac_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = claxon::FlacReader::open(path).map_err(|e| format!("open flac: {e}"))?;
    let info = reader.streaminfo();
    let bits = info.bits_per_sample;
    let scale = (((1_u64 << bits.saturating_sub(1)) as f32).max(1.0)).recip();
    let channels = usize::try_from(info.channels.max(1)).unwrap_or(1);
    let mut samples: Vec<f32> = Vec::with_capacity(info.samples.unwrap_or(0) as usize);
    if channels == 1 {
        for s in reader.samples() {
            let v = s.map_err(|e| format!("decode flac: {e}"))?;
            samples.push((v as f32) * scale);
        }
    } else {
        let channels_i64 = i64::try_from(channels).unwrap_or(1);
        let mut acc: i64 = 0;
        let mut idx: usize = 0;
        for s in reader.samples() {
            let v = s.map_err(|e| format!("decode flac: {e}"))?;
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
    Ok((samples, info.sample_rate))
}

/// Decode a 16-bit PCM mono / stereo WAV file into a mono `f32` buffer in
/// `[-1, 1]`. Stereo sources are mono-summed (mean of channels). Other bit
/// depths surface as `Err` so the caller can return a typed
/// [`TranscribeError::DecodeFailed`].
fn decode_wav_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
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
    let mut data_payload: Vec<u8> = Vec::new();
    let mut format_tag: u16 = 0;
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

    // Decode interleaved PCM into mono f32. PCM 16 (format-tag 1) and IEEE
    // float 32 (format-tag 3) are the two shapes the live recording flow
    // and the import-test fixtures emit.
    let samples = match (format_tag, bits_per_sample) {
        (1, 16) => decode_pcm16_mono(&data_payload, channels),
        (1, 24) => decode_pcm24_mono(&data_payload, channels),
        (3, 32) => decode_float32_mono(&data_payload, channels),
        (tag, bits) => {
            return Err(format!("unsupported wav format-tag {tag} / {bits}-bit"));
        }
    };
    Ok((samples, sample_rate_hz))
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
            // sign-extend 24-bit little-endian
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

/// Deterministic transcribe: zero-crossing-rate frequency estimator over a
/// single window, plus an RMS-derived velocity. Recovers exactly one
/// note for clean monophonic material — which is the contract the Phase 3
/// integration tests assert ("note_count >= 1 for a 1 s 440 Hz tone").
///
/// This is the placeholder integration the IPC layer ships against until
/// the algo team's `BasicPitchEstimator::from_bundled` polyphonic
/// transcriber lands; the Tauri command surface, the cache key, the wire
/// shape, and the front-end progress contract are stable across the swap.
fn transcribe_buffer(samples: &[f32], sample_rate_hz: u32, duration_ms: u64) -> Vec<WireNoteEvent> {
    if samples.is_empty() || duration_ms == 0 {
        return Vec::new();
    }

    // RMS gates near-silence so we do not emit a note for a recording that
    // is just background noise.
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    if rms < 0.01 {
        return Vec::new();
    }

    // Zero-crossing rate → fundamental frequency estimate. For a clean
    // sinusoid this is exact; for harmonic-rich material it picks up the
    // strongest periodic component, which is good enough for the Phase 3
    // baseline.
    let mut crossings: u64 = 0;
    for window in samples.windows(2) {
        let prev = window[0];
        let curr = window[1];
        if prev.signum() != curr.signum() && (prev != 0.0 || curr != 0.0) {
            crossings += 1;
        }
    }
    if crossings == 0 {
        return Vec::new();
    }
    let seconds = samples.len() as f32 / sample_rate_hz as f32;
    if seconds <= 0.0 {
        return Vec::new();
    }
    let freq_hz = crossings as f32 / (2.0 * seconds);
    if !(27.5..=4_186.0).contains(&freq_hz) {
        // Outside the 88-key piano range; reject as noise.
        return Vec::new();
    }

    // Hz → MIDI. 440 Hz → 69, +1 per semitone.
    let midi_f = 69.0 + 12.0 * (freq_hz / 440.0).log2();
    let midi = midi_f.round().clamp(0.0, 127.0) as u8;

    // Velocity from RMS, scaled into the standard MIDI range.
    let vel_f = (rms * 4.0 * 127.0).clamp(1.0, 127.0);
    let velocity = vel_f.round() as u8;

    vec![WireNoteEvent {
        midi,
        start_ms: 0,
        end_ms: duration_ms,
        velocity,
    }]
}

/// Emit a single terminal `percent: 1.0` tick on the optional progress
/// sink. Mirrors the cache-hit contract the Phase 3 spec asserts.
fn emit_terminal(progress: Option<&dyn TranscribeProgressSink>, id: RecordingId) {
    if let Some(sink) = progress {
        sink.emit(TranscribeProgress {
            recording_id: id.to_string(),
            percent: 1.0,
            current_window: 1,
            total_windows: 1,
        });
    }
}

/// Compose the `<dest>.partial` companion path.
///
/// Appends `.partial` to whatever extension the caller supplied so e.g.
/// `out.mid` → `out.mid.partial`. Mirrors the FLAC sink's atomic-write
/// shape.
fn partial_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".partial");
    PathBuf::from(s)
}

/// Convert a [`WirePolyResult`] into SMF type-1 byte stream.
///
/// Single track, single channel (channel 0). Per note:
/// 1. `NoteOn` at `start_ms`,
/// 2. `NoteOff` at `end_ms`.
///
/// The track preamble emits one `SetTempo` meta and the trailing
/// `EndOfTrack` meta is required by the SMF spec so `midly::Smf::parse`
/// round-trips cleanly.
fn wire_to_smf(wire: &WirePolyResult) -> Result<Vec<u8>, TranscribeError> {
    let header = Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(TICKS_PER_QUARTER)),
    );

    // Build the (timestamp, event) list, sort by timestamp, then convert to
    // delta-time-encoded TrackEvents. ms → tick conversion at the supplied
    // tempo:
    //   ticks_per_ms = TICKS_PER_QUARTER * tempo_bpm / 60_000
    let ticks_per_ms = f64::from(TICKS_PER_QUARTER) * f64::from(TEMPO_BPM) / 60_000.0;
    let ms_to_ticks = |ms: u64| -> u32 {
        let raw = (ms as f64 * ticks_per_ms).round();
        if raw < 0.0 {
            0
        } else if raw > f64::from(u32::MAX) {
            u32::MAX
        } else {
            raw as u32
        }
    };

    // (tick, kind) pairs. Tempo and end-of-track are stamped at boundary
    // ticks 0 and `total_ticks` respectively.
    let mut events: Vec<(u32, TrackEventKind<'static>)> =
        Vec::with_capacity(wire.notes.len() * 2 + 2);
    let total_ticks = ms_to_ticks(wire.duration_ms);
    let tempo_us_per_quarter = 60_000_000_u32 / TEMPO_BPM;
    events.push((
        0,
        TrackEventKind::Meta(MetaMessage::Tempo(u24::new(tempo_us_per_quarter))),
    ));

    for n in &wire.notes {
        let start_tick = ms_to_ticks(n.start_ms);
        let end_tick = ms_to_ticks(n.end_ms.max(n.start_ms.saturating_add(1)));
        events.push((
            start_tick,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOn {
                    key: u7::new(n.midi.min(127)),
                    vel: u7::new(n.velocity.clamp(1, 127)),
                },
            },
        ));
        events.push((
            end_tick,
            TrackEventKind::Midi {
                channel: u4::new(0),
                message: MidiMessage::NoteOff {
                    key: u7::new(n.midi.min(127)),
                    vel: u7::new(0),
                },
            },
        ));
    }
    events.push((total_ticks, TrackEventKind::Meta(MetaMessage::EndOfTrack)));

    // Stable sort so simultaneous events keep their insertion order
    // (NoteOn-before-NoteOff for the same tick is important when a note is
    // length 0 — though we clamp to length-1 above so that path is rare).
    events.sort_by_key(|(t, _)| *t);

    let mut track: Vec<TrackEvent<'static>> = Vec::with_capacity(events.len());
    let mut prev_tick: u32 = 0;
    for (tick, kind) in events {
        let delta_ticks = tick.saturating_sub(prev_tick);
        // u28 is 28-bit; `from` masks to fit. Clamp explicitly so a
        // malformed buffer cannot produce a malformed varlen.
        let delta = u28::new(delta_ticks & 0x0FFF_FFFF);
        track.push(TrackEvent { delta, kind });
        prev_tick = tick;
    }

    let smf = Smf {
        header,
        tracks: vec![track],
    };
    let mut out = Vec::with_capacity(128);
    smf.write(&mut out)
        .map_err(|e| TranscribeError::MidiEmitFailed(e.to_string()))?;
    Ok(out)
}
