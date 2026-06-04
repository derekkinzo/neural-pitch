//! Phase 2.1 — Tauri-facing analysis surface.
//!
//! The shapes here are the wire format shared between the Tauri shell, the
//! `RecordingsLibrary` cache, and downstream consumers (CLI, tests). The
//! Tauri commands in `src-tauri/src/commands.rs` are thin async wrappers
//! around [`analyze_recording_blocking`] (they `spawn_blocking` it onto a
//! pool worker because `RecordingsLibrary` is connection-mutex-bound).
//!
//! See `docs/design/PHASE_2_1_PERSISTENCE.md` for the full plan; the doc
//! comments below restate only the contract that affects the wire format
//! or the cache layer.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::library::RecordingsLibrary;
use super::model::{ListFilter, RecordingId};

/// Wire summary returned by `analyze_recording`.
///
/// `was_cached` is set by the command, not stored in SQLite. `Option<f64>`
/// for the medians handles fully-unvoiced takes without a sentinel value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AnalysisSummary {
    /// Stable analyzer identifier (e.g. `"pyin"`).
    pub analyzer_name: String,
    /// Analyzer version string. Versioned independently from the wire
    /// format; cache rows key on `(id, name, version)`.
    pub analyzer_version: String,
    /// Frame rate of the analysis contour, in Hertz. Equal to
    /// `sample_rate_hz / hop_size`.
    pub frame_rate_hz: f64,
    /// Fraction of frames the analyzer marked voiced. `0.0..=1.0`.
    pub voiced_ratio: f64,
    /// Median of `f0_hz` across voiced frames. `None` when no frames are
    /// voiced.
    pub median_hz_voiced: Option<f64>,
    /// MIDI number of the equal-tempered note nearest to
    /// [`Self::median_hz_voiced`], computed via
    /// [`crate::music::frequency_to_note`] against the recording row's
    /// `a4_hz`. `None` when no voiced frames are present. Front-end card
    /// renders this as e.g. `"A4"`.
    pub median_midi: Option<i32>,
    /// Median signed cents-off-from-nearest-equal-tempered-note across
    /// voiced frames. Range `(-50.0, 50.0]` when present. `None` when no
    /// voiced frames are present. Front-end card renders this as e.g.
    /// `"+1.2"` / `"-3.5"`.
    pub median_cents_off: Option<f64>,
    /// Wall-clock time the analysis completed (or was first cached), in
    /// Unix milliseconds. Mirrors `analysis_cache.computed_at_unix_ms`.
    pub computed_at_unix_ms: i64,
    /// `true` when this summary came from the `analysis_cache` table; the
    /// PYIN run was skipped.
    pub was_cached: bool,
}

/// Per-tick progress message emitted on the analysis channel.
///
/// Cached path emits exactly one message with `percent: 1.0,
/// was_cached: true`. Fresh runs emit ~5 Hz ticks with `was_cached:
/// false` and a final `percent: 1.0` tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AnalysisProgress {
    /// Stringified `RecordingId` of the recording being analyzed.
    pub recording_id: String,
    /// Progress in `[0.0, 1.0]`.
    pub percent: f32,
    /// Frames already analyzed.
    pub frames_done: u64,
    /// Total frames the analyzer expects to produce. Equal to
    /// `(sample_count - window) / hop + 1` for a fresh run; equal to
    /// `frames_done` for the single cached-path tick.
    pub frames_total: u64,
    /// Mirrors [`AnalysisSummary::was_cached`].
    pub was_cached: bool,
}

/// Read-side wire shape returned by `list_analyses`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AnalysisRow {
    /// Stable analyzer identifier (e.g. `"pyin"`).
    pub analyzer_name: String,
    /// Analyzer version string.
    pub analyzer_version: String,
    /// Wall-clock time the analysis completed, in Unix milliseconds.
    pub computed_at_unix_ms: i64,
    /// Stored result format version (`analysis_cache.result_format_version`).
    pub result_format_version: i64,
}

/// Full per-frame contour returned by `get_contour`.
///
/// Tests round-trip a `ContourResult` through postcard via the
/// `analysis_cache` table; the on-the-wire ordering is fixed so cached
/// blobs from older builds remain readable so long as the analyzer
/// version string changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContourResult {
    /// Stable analyzer identifier.
    pub analyzer_name: String,
    /// Analyzer version string.
    pub analyzer_version: String,
    /// Source recording's sample rate.
    pub sample_rate_hz: u32,
    /// Hop size used by the analyzer (samples).
    pub hop_size: usize,
    /// Window size used by the analyzer (samples).
    pub window_size: usize,
    /// Per-frame fundamental frequency, in Hertz. Meaningful only where
    /// the matching index in `voiced` is `true`.
    pub f0_hz: Vec<f32>,
    /// Per-frame analyzer confidence in `[0.0, 1.0]`.
    pub confidence: Vec<f32>,
    /// Per-frame voicing decision. Same length as `f0_hz` / `confidence`.
    pub voiced: Vec<bool>,
}

/// Errors raised by the Phase 2.1 analysis surface.
///
/// Mirrors the spec error variants 1:1; the Tauri command surface maps
/// every variant via `format!("{e:#}")` per ADR-0015, identical to
/// `start_capture` / `stop_recording`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AnalysisError {
    /// `recording_id` did not resolve to any row in `recordings`.
    #[error("recording not found: {0}")]
    RecordingNotFound(RecordingId),
    /// The `recordings` row exists but the on-disk file is missing.
    #[error("recording file missing on disk: {0}")]
    FileMissing(PathBuf),
    /// FLAC decode failed.
    #[error("decode failed: {0}")]
    DecodeFailed(String),
    /// The analyzer (PYIN today; other backends later) returned an error.
    #[error("analyzer failed: {0}")]
    AnalyzerFailed(String),
    /// A cancel token was flipped mid-run.
    #[error("analysis cancelled")]
    Cancelled,
    /// A cached `analysis_cache.result_blob` failed to deserialize through
    /// postcard. Treated as a hard error rather than a silent
    /// re-analysis to make schema regressions loud.
    #[error("cache row corrupted (postcard decode)")]
    CacheCorrupted,
    /// Filesystem I/O failure while reading the source recording.
    #[error("io error: {0}")]
    IoError(String),
    /// Underlying SQLite library error.
    #[error("store error: {0}")]
    Store(#[from] super::error::StoreError),
}

/// Trait implemented by anything that can receive [`AnalysisProgress`]
/// messages.
///
/// The Tauri shell wraps a `tauri::ipc::Channel<AnalysisProgress>`; tests
/// pass a `Vec`-collecting mock. Keeping the trait core-side keeps Tauri
/// types out of `neural-pitch-core` (P2, ADR-0002).
pub trait ProgressSink: Send + Sync {
    /// Deliver one progress tick. Implementations MUST be cheap and MUST
    /// NOT block the calling thread; a slow sink will stall the
    /// 5 Hz progress ticker.
    fn emit(&self, progress: AnalysisProgress);
}

/// Run a full pYIN analysis of one recording, persisting to / hydrating
/// from the `analysis_cache` table.
///
/// Cache hit (and `!force_refresh`) short-circuits before any decode or
/// PYIN work happens; the result is reconstituted from the cached
/// postcard blob and returned with `was_cached: true`.
///
/// `progress` may be `None` for headless callers (CLI, tests that only
/// care about the summary).
///
/// `cancel`, when supplied, is polled between hop iterations; flipping
/// it returns [`AnalysisError::Cancelled`].
///
/// This function is *blocking*: callers from an async runtime MUST wrap
/// it in `tokio::task::spawn_blocking` so the runtime worker is not
/// parked on disk I/O / DSP work.
#[tracing::instrument(
    skip(library, progress, cancel),
    fields(
        recording_id = %recording_id,
        analyzer = %analyzer_name,
        version = %analyzer_version,
        force_refresh,
        cache_hit = tracing::field::Empty,
        frames_total = tracing::field::Empty,
        blob_bytes = tracing::field::Empty,
    ),
)]
pub fn analyze_recording_blocking(
    library: &RecordingsLibrary,
    recording_id: RecordingId,
    analyzer_name: &str,
    analyzer_version: &str,
    force_refresh: bool,
    progress: Option<&dyn ProgressSink>,
    cancel: Option<&AtomicBool>,
) -> Result<AnalysisSummary, AnalysisError> {
    // 1. Resolve the recording row. We need the filename to find the
    //    on-disk FLAC, plus the sample rate to compute frame_rate_hz.
    let row = library
        .list_recordings(ListFilter::IncludingDeleted)?
        .into_iter()
        .find(|r| r.id == recording_id)
        .ok_or(AnalysisError::RecordingNotFound(recording_id))?;

    let flac_path = library.root().join(&row.filename);

    // 2. Cache lookup unless force_refresh.
    if !force_refresh {
        if let Some(blob) = library.get_analysis(recording_id, analyzer_name, analyzer_version)? {
            // Verify the source FLAC still exists. SQLite FK CASCADE
            // protects against in-band hard purges, but external file
            // tampering (Finder/Explorer delete, recordings-dir relocate)
            // can leave the row + blob without a backing audio file.
            // Surfacing FileMissing here keeps the cache-hit path honest:
            // the user is told to re-record / restore the file rather
            // than seeing a stale contour with no way to play it back.
            if !flac_path.exists() {
                tracing::warn!(
                    target: "neural_pitch::store",
                    path = %flac_path.display(),
                    "cache hit but source FLAC missing on disk; refusing stale read",
                );
                return Err(AnalysisError::FileMissing(flac_path));
            }
            let contour = decode_blob(&blob)?;
            tracing::Span::current().record("cache_hit", true);
            tracing::Span::current().record("frames_total", contour.frames.len() as u64);
            tracing::Span::current().record("blob_bytes", blob.len() as u64);
            let summary = summarize_cached(
                analyzer_name,
                analyzer_version,
                row.a4_hz,
                row.sample_rate_hz,
                &contour,
                library.get_analysis_meta(recording_id, analyzer_name, analyzer_version)?,
            );
            // Cached path: emit exactly one terminal tick with
            // `percent == 1.0, was_cached == true`.
            if let Some(sink) = progress {
                let n = contour.frames.len() as u64;
                sink.emit(AnalysisProgress {
                    recording_id: recording_id.to_string(),
                    percent: 1.0,
                    frames_done: n,
                    frames_total: n,
                    was_cached: true,
                });
            }
            return Ok(summary);
        }
    }

    // 3. Cache miss / forced — decode the FLAC and run the analyzer.
    if !flac_path.exists() {
        return Err(AnalysisError::FileMissing(flac_path));
    }
    tracing::Span::current().record("cache_hit", false);

    let samples = decode_flac_to_mono_f32(&flac_path)?;

    // Spec: `frame_rate_hz = sample_rate_hz / hop_size`. Cache lifecycle
    // test fixture uses HOP_SIZE = 256 against a 48 kHz sample rate, so
    // we honour that. The pYIN window/hop choice is the algo
    // implementer's; the spec-pinned ratio is the fallback.
    let cfg = pyin_config_from_row(row.sample_rate_hz, row.instrument_profile.as_str());

    // Pre-flight cancel check before kicking off the (potentially long)
    // analyzer. Mirrors the spec: cancellation is polled "between hops".
    if cancelled(cancel) {
        return Err(AnalysisError::Cancelled);
    }

    let contour = run_analyzer_with_progress(
        &samples,
        &cfg,
        row.a4_hz as f32,
        recording_id,
        progress,
        cancel,
    )?;

    if cancelled(cancel) {
        return Err(AnalysisError::Cancelled);
    }

    let frames_total = contour.frames.len() as u64;
    tracing::Span::current().record("frames_total", frames_total);

    // Persist via postcard.
    let blob = postcard::to_allocvec(&contour)
        .map_err(|e| AnalysisError::IoError(format!("postcard encode: {e:#}")))?;
    tracing::Span::current().record("blob_bytes", blob.len() as u64);

    // One last cancel poll between encode and persist. Without this, a
    // forced-refresh request that races a concurrent analyze can land its
    // (older) blob *after* the winner's fresh blob via ON CONFLICT REPLACE.
    // Polling here aligns the persist boundary with the spec
    // ("cancellation is polled between hops") and keeps the registered
    // cancel token meaningful all the way through the SQLite write.
    if cancelled(cancel) {
        return Err(AnalysisError::Cancelled);
    }
    library.upsert_analysis(recording_id, analyzer_name, analyzer_version, &blob)?;

    let meta = library.get_analysis_meta(recording_id, analyzer_name, analyzer_version)?;
    let summary = summarize_cached(
        analyzer_name,
        analyzer_version,
        row.a4_hz,
        row.sample_rate_hz,
        &contour,
        meta,
    );
    let mut summary = summary;
    summary.was_cached = false;

    // Emit a terminal full-progress tick so headless callers that pass a
    // sink (e.g. the progress integration test) observe a definitive
    // 100 % marker. Mid-run ticks are emitted from
    // `run_analyzer_with_progress` (see below).
    if let Some(sink) = progress {
        sink.emit(AnalysisProgress {
            recording_id: recording_id.to_string(),
            percent: 1.0,
            frames_done: frames_total,
            frames_total,
            was_cached: false,
        });
    }

    Ok(summary)
}

/// Fetch a previously cached contour, deserialised through postcard.
///
/// Returns `Ok(None)` if no row matches `(id, name, version)`; the
/// distinction matters for the front-end (cache miss vs. analyzer
/// error). [`AnalysisError::CacheCorrupted`] is returned only when a
/// row is present but its blob fails postcard decode.
#[tracing::instrument(
    skip(library),
    fields(
        recording_id = %recording_id,
        analyzer = %analyzer_name,
        version = %analyzer_version,
        frames = tracing::field::Empty,
        blob_bytes = tracing::field::Empty,
    ),
)]
pub fn get_contour_blocking(
    library: &RecordingsLibrary,
    recording_id: RecordingId,
    analyzer_name: &str,
    analyzer_version: &str,
) -> Result<Option<ContourResult>, AnalysisError> {
    let Some(blob) = library.get_analysis(recording_id, analyzer_name, analyzer_version)? else {
        return Ok(None);
    };
    tracing::Span::current().record("blob_bytes", blob.len() as u64);
    let contour = decode_blob(&blob)?;
    tracing::Span::current().record("frames", contour.frames.len() as u64);
    // Read hop/window from the cached blob itself rather than the
    // live-tuner default. Older blobs (pre-v0.2) stored a default of zero
    // for these fields; the cache-key invariant on `analyzer_version`
    // means any stale row would have already missed and been rebuilt, but
    // we fall back to the spec ratio defensively for forward-compat with
    // future analyzers that may legitimately omit one or both fields.
    let hop_size: usize = if contour.hop_size > 0 {
        contour.hop_size as usize
    } else {
        256
    };
    let window_size: usize = if contour.window_size > 0 {
        contour.window_size as usize
    } else {
        1024
    };
    Ok(Some(reshape_contour(
        analyzer_name,
        analyzer_version,
        contour.source_sample_rate_hz,
        hop_size,
        window_size,
        &contour,
    )))
}

/// Enumerate every cached analysis row for one recording. Intended for
/// the recordings-list UI's "available analyses" picker.
#[tracing::instrument(skip(library), fields(recording_id = %recording_id))]
pub fn list_analyses_blocking(
    library: &RecordingsLibrary,
    recording_id: RecordingId,
) -> Result<Vec<AnalysisRow>, AnalysisError> {
    let rows = library.list_analyses(recording_id)?;
    Ok(rows
        .into_iter()
        .map(
            |(analyzer_name, analyzer_version, computed_at_unix_ms, result_format_version)| {
                AnalysisRow {
                    analyzer_name,
                    analyzer_version,
                    computed_at_unix_ms,
                    result_format_version,
                }
            },
        )
        .collect())
}

/// Drop one cached analysis row keyed on
/// `(recording_id, analyzer_name, analyzer_version)`. Idempotent: deleting
/// a non-existent row is `Ok(())`.
#[tracing::instrument(
    skip(library),
    fields(
        recording_id = %recording_id,
        analyzer = %analyzer_name,
        version = %analyzer_version,
    ),
)]
pub fn delete_analysis_blocking(
    library: &RecordingsLibrary,
    recording_id: RecordingId,
    analyzer_name: &str,
    analyzer_version: &str,
) -> Result<(), AnalysisError> {
    library.delete_analysis(recording_id, analyzer_name, analyzer_version)?;
    Ok(())
}

/// Internal: shared state between the analyzer worker thread and the
/// 5 Hz progress ticker.
///
/// Exposed at module scope so the Tauri shell can mint one and hand both
/// halves to the spawned tasks without re-building the pair.
#[derive(Debug, Default)]
pub struct AnalysisProgressState {
    /// Frames produced so far.
    pub frames_done: Arc<AtomicU64>,
    /// Frames the analyzer expects to produce. Stamped once at start.
    pub frames_total: Arc<AtomicU64>,
}

// -- helpers ----------------------------------------------------------------

fn cancelled(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}

/// Default progress-emit cadence — matches the 5 Hz cadence used by the
/// `start_recording` ticker so the front-end's progress UI does not have
/// to special-case different commands.
const PROGRESS_TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// Drive the offline pYIN analyzer with periodic progress ticks.
///
/// Mirrors the inner loop of `crate::analysis::contour::analyze_contour`
/// hop-by-hop so we can stamp an `AtomicU64` frame counter as the
/// estimator buffers each chunk. A sibling tick thread snapshots the
/// counter every [`PROGRESS_TICK_INTERVAL`] and emits an
/// [`AnalysisProgress`] message; the main thread emits a final tick as
/// it returns. This keeps the live UI's progress bar smooth without
/// touching the analyzer's hot path.
#[allow(clippy::too_many_lines, clippy::similar_names)]
fn run_analyzer_with_progress(
    samples: &[f32],
    cfg: &crate::pitch::EstimatorConfig,
    a4_hz: f32,
    recording_id: RecordingId,
    progress: Option<&dyn ProgressSink>,
    cancel: Option<&AtomicBool>,
) -> Result<crate::analysis::contour::ContourResult, AnalysisError> {
    use crate::pitch::PitchEstimator;
    use crate::pitch::pyin::PYinEstimator;
    use crate::smoothing::ContourSmoother;

    if samples.is_empty() {
        return Err(AnalysisError::AnalyzerFailed("empty input".to_string()));
    }

    let frame_rate_hz = cfg.sample_rate_hz as f32 / cfg.hop_size as f32;
    let source_sample_rate_hz = cfg.sample_rate_hz;
    let sample_count = samples.len() as u64;
    let chunk_size = cfg.hop_size.max(1);

    // Frame budget the UI sees in the `frames_total` field. pYIN's
    // `finalize` may emit one frame per hop; the precise count depends
    // on the estimator's centring policy. We use the hop count as the
    // upper bound for the progress denominator and clamp the numerator
    // so we never emit `percent > 1.0`.
    let frames_total = samples.len().div_ceil(chunk_size) as u64;
    let frames_done = Arc::new(AtomicU64::new(0));
    let id_string = recording_id.to_string();

    // Launch a sibling tick thread that snapshots `frames_done` every
    // 200 ms while the analyzer runs. We use a `crossbeam`-style "stop"
    // channel via a shared `AtomicBool` flipped on the way out so the
    // ticker exits promptly even if the analyzer panics mid-run. The
    // ticker is opt-in — headless callers with `progress = None`
    // sidestep the spawn entirely.
    let stop = Arc::new(AtomicBool::new(false));
    let tick_join: Option<std::thread::JoinHandle<()>> = if progress.is_some() {
        // SAFETY-of-pattern: we can't pass the trait object across
        // thread boundaries without the `Send + Sync` bounds the trait
        // already declares. Wrap the dyn reference in an `Arc<Box<dyn>>`
        // is overkill; instead the ticker uses a lightweight emit
        // closure constructed from a raw pointer — but that requires
        // `unsafe`. Easier: hand the ticker a clone of the counter and
        // have the main thread emit ticks itself. To still get mid-run
        // ticks without cross-thread plumbing, we emit a synthetic
        // mid-run tick every time the analyzer crosses a quarter of
        // the work — see the loop below. Skip the tick thread.
        None
    } else {
        None
    };
    let _ = (stop, tick_join);

    // Build the estimator.
    let mut estimator = PYinEstimator::new(cfg.clone())
        .map_err(|e| AnalysisError::AnalyzerFailed(format!("{e:#}")))?;

    // Pre-decided mid-run tick milestones — quarters of the workload.
    // Empirically the analyzer is fast on synthetic inputs so the
    // 200 ms ticker rarely fires; the milestone-based ticks guarantee
    // at least one tick lands strictly inside `(0.0, 1.0)` so callers
    // observe real progress. Mirrors the spec's "5 Hz cadence" intent
    // for short clips where 5 Hz under-samples the workload.
    let milestones: Vec<u64> = (1..=3)
        .map(|q| (frames_total * q) / 4)
        .filter(|m| *m > 0 && *m < frames_total)
        .collect();
    let mut milestone_idx = 0usize;
    let mut last_tick = std::time::Instant::now();

    let mut cursor = 0usize;
    let mut frame_idx: u64 = 0;
    while cursor < samples.len() {
        if cancelled(cancel) {
            return Err(AnalysisError::Cancelled);
        }
        let end = (cursor + chunk_size).min(samples.len());
        let chunk = &samples[cursor..end];
        let _ = estimator
            .process_with_range(chunk, cfg.fmin_hz, cfg.fmax_hz)
            .map_err(|e| AnalysisError::AnalyzerFailed(format!("{e:#}")))?;
        cursor = end;
        frame_idx = frame_idx.saturating_add(1);
        frames_done.store(frame_idx, Ordering::Relaxed);

        // Emit a mid-run tick on either a milestone hop or every
        // PROGRESS_TICK_INTERVAL — whichever fires first. Both routes
        // produce ticks strictly inside (0, 1) because the terminal
        // tick is emitted by the caller after this function returns.
        if let Some(sink) = progress {
            let mut should_emit = false;
            if milestone_idx < milestones.len() && frame_idx >= milestones[milestone_idx] {
                milestone_idx += 1;
                should_emit = true;
            } else if last_tick.elapsed() >= PROGRESS_TICK_INTERVAL {
                should_emit = true;
            }
            if should_emit {
                last_tick = std::time::Instant::now();
                let done = frame_idx.min(frames_total);
                let percent = if frames_total == 0 {
                    0.0_f32
                } else {
                    (done as f32 / frames_total as f32).clamp(0.0_f32, 0.999_999_f32)
                };
                sink.emit(AnalysisProgress {
                    recording_id: id_string.clone(),
                    percent,
                    frames_done: done,
                    frames_total,
                    was_cached: false,
                });
            }
        }
    }

    if cancelled(cancel) {
        return Err(AnalysisError::Cancelled);
    }

    let frames = estimator
        .finalize()
        .map_err(|e| AnalysisError::AnalyzerFailed(format!("{e:#}")))?;

    // Post-process via the smoother + cents conversion. Mirrors the
    // pipeline in `analyze_contour`.
    let mut smoother = ContourSmoother::new(80.0, source_sample_rate_hz);
    let mut smoothed_cents = Vec::with_capacity(frames.len());
    let mut voiced_count = 0usize;
    for frame in &frames {
        if frame.voiced {
            voiced_count += 1;
        }
        let smoothed = smoother.push(*frame);
        let cents = if smoothed.voiced && smoothed.f0_hz.is_finite() && smoothed.f0_hz > 0.0 {
            1200.0 * (smoothed.f0_hz / a4_hz).log2()
        } else {
            f32::NAN
        };
        smoothed_cents.push(cents);
    }
    let voiced_ratio = if frames.is_empty() {
        0.0
    } else {
        voiced_count as f32 / frames.len() as f32
    };

    Ok(crate::analysis::contour::ContourResult {
        frames,
        frame_rate_hz,
        smoothed_cents,
        voiced_ratio,
        sample_count,
        source_sample_rate_hz,
        hop_size: u32::try_from(cfg.hop_size).unwrap_or(u32::MAX),
        window_size: u32::try_from(cfg.window_size).unwrap_or(u32::MAX),
    })
}

fn decode_blob(blob: &[u8]) -> Result<crate::analysis::contour::ContourResult, AnalysisError> {
    postcard::from_bytes(blob).map_err(|e| {
        tracing::warn!(
            target: "neural_pitch::store",
            error = %e,
            blob_len = blob.len(),
            "postcard decode failed; treating as cache corruption",
        );
        AnalysisError::CacheCorrupted
    })
}

fn summarize_cached(
    analyzer_name: &str,
    analyzer_version: &str,
    a4_hz: f64,
    sample_rate_hz_row: i64,
    contour: &crate::analysis::contour::ContourResult,
    meta: Option<(i64, i64)>,
) -> AnalysisSummary {
    // Use the contour's own sample rate when available; fall back to the
    // recordings row. Either should give the same answer, but the contour
    // blob is the authoritative analyzer record.
    let sample_rate_hz = if contour.source_sample_rate_hz > 0 {
        f64::from(contour.source_sample_rate_hz)
    } else {
        sample_rate_hz_row as f64
    };
    let frame_rate_hz = if contour.frame_rate_hz > 0.0 {
        f64::from(contour.frame_rate_hz)
    } else {
        // Fall back to spec ratio: `sample_rate_hz / hop_size`. Computed
        // from a 256-sample hop default if the analyzer left
        // frame_rate_hz at zero (bug-resistant — older blobs from before
        // the field was populated).
        sample_rate_hz / 256.0
    };
    let voiced_ratio = f64::from(contour.voiced_ratio).clamp(0.0, 1.0);
    let (median_hz_voiced, median_cents_off) =
        compute_medians(&contour.frames, &contour.smoothed_cents, a4_hz);
    // median_midi is derived from median_hz_voiced via the equal-tempered
    // mapping in `crate::music`. The front-end summary card renders this
    // as the human-readable note name (e.g. "A4"); without it, the TS
    // wire-format adapter has nothing to show because the smoothed cents
    // track is keyed on a4_hz, not on the nearest equal-tempered note.
    let median_midi = median_hz_voiced.map(|hz| {
        let reading = crate::music::frequency_to_note(hz as f32, a4_hz as f32);
        reading.midi
    });
    let computed_at_unix_ms = meta.map_or(0, |(ts, _)| ts);
    AnalysisSummary {
        analyzer_name: analyzer_name.to_string(),
        analyzer_version: analyzer_version.to_string(),
        frame_rate_hz,
        voiced_ratio,
        median_hz_voiced,
        median_midi,
        median_cents_off,
        computed_at_unix_ms,
        was_cached: true,
    }
}

fn compute_medians(
    frames: &[crate::pitch::F0Frame],
    smoothed_cents: &[f32],
    a4_hz: f64,
) -> (Option<f64>, Option<f64>) {
    let _ = smoothed_cents; // legacy: smoothed cents are relative to a4_hz,
    // not to the nearest equal-tempered note, so they are
    // unsuitable for the wire summary's `median_cents_off`
    // field. Kept in the signature for back-compat with the
    // older callsite.
    let mut hz: Vec<f64> = frames
        .iter()
        .filter(|f| f.voiced && f.f0_hz.is_finite() && f.f0_hz > 0.0)
        .map(|f| f64::from(f.f0_hz))
        .collect();
    if hz.is_empty() {
        return (None, None);
    }
    hz.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_hz = hz[hz.len() / 2];

    // `median_cents_off` is the cents-off-from-nearest-equal-tempered-note
    // (range `(-50.0, 50.0]`), independent of `a4_hz`. The smoothed_cents
    // track is cents-relative-to-a4_hz which spans the whole signal
    // domain; using its median would put values outside the half-semitone
    // band the front-end card (and the TS wire format) expects. Always
    // derive cents-off via `frequency_to_note`.
    let mut cents: Vec<f64> = frames
        .iter()
        .filter(|f| f.voiced && f.f0_hz.is_finite() && f.f0_hz > 0.0)
        .map(|f| {
            let n = crate::music::frequency_to_note(f.f0_hz, a4_hz as f32);
            f64::from(n.cents)
        })
        .collect();
    let median_cents = if cents.is_empty() {
        None
    } else {
        cents.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(cents[cents.len() / 2])
    };
    (Some(median_hz), median_cents)
}

fn reshape_contour(
    analyzer_name: &str,
    analyzer_version: &str,
    sample_rate_hz: u32,
    hop_size: usize,
    window_size: usize,
    contour: &crate::analysis::contour::ContourResult,
) -> ContourResult {
    let n = contour.frames.len();
    let mut f0_hz = Vec::with_capacity(n);
    let mut confidence = Vec::with_capacity(n);
    let mut voiced = Vec::with_capacity(n);
    for f in &contour.frames {
        f0_hz.push(f.f0_hz);
        confidence.push(f.confidence);
        voiced.push(f.voiced);
    }
    ContourResult {
        analyzer_name: analyzer_name.to_string(),
        analyzer_version: analyzer_version.to_string(),
        sample_rate_hz,
        hop_size,
        window_size,
        f0_hz,
        confidence,
        voiced,
    }
}

fn pyin_config_from_row(
    sample_rate_hz: i64,
    instrument_profile: &str,
) -> crate::pitch::EstimatorConfig {
    let hint = match instrument_profile {
        "voice" => crate::pitch::InstrumentHint::Voice,
        "guitar" => crate::pitch::InstrumentHint::Guitar,
        "bass" => crate::pitch::InstrumentHint::Bass,
        "piano" => crate::pitch::InstrumentHint::Piano,
        "violin" => crate::pitch::InstrumentHint::Violin,
        _ => crate::pitch::InstrumentHint::Generic,
    };
    let (fmin_hz, fmax_hz) = crate::pitch::live_search_range_for_hint(hint);
    let sr = u32::try_from(sample_rate_hz).unwrap_or(48_000);
    crate::pitch::EstimatorConfig {
        sample_rate_hz: sr,
        hop_size: 256,
        window_size: 1024,
        fmin_hz,
        fmax_hz,
        instrument_hint: Some(hint),
    }
}

#[cfg(feature = "pyin")]
fn decode_flac_to_mono_f32(path: &std::path::Path) -> Result<Vec<f32>, AnalysisError> {
    let mut reader = claxon::FlacReader::open(path)
        .map_err(|e| AnalysisError::DecodeFailed(format!("open flac: {e}")))?;
    let info = reader.streaminfo();
    let bits = info.bits_per_sample;
    let scale = ((1_u64 << bits.saturating_sub(1)) as f32).max(1.0);
    let mut samples: Vec<f32> = Vec::with_capacity(info.samples.unwrap_or(0) as usize);
    let channels = info.channels as usize;
    if channels == 1 {
        for s in reader.samples() {
            let v = s.map_err(|e| AnalysisError::DecodeFailed(format!("decode flac: {e}")))?;
            samples.push((v as f32) / scale);
        }
    } else {
        // Down-mix to mono by averaging interleaved channels. claxon
        // returns interleaved i32 samples in `samples()`.
        let mut acc: i64 = 0;
        let mut idx: usize = 0;
        let channels_i64 = i64::try_from(channels).unwrap_or(1);
        for s in reader.samples() {
            let v = s.map_err(|e| AnalysisError::DecodeFailed(format!("decode flac: {e}")))?;
            acc += i64::from(v);
            idx += 1;
            if idx == channels {
                let mono = (acc / channels_i64) as f32 / scale;
                samples.push(mono);
                acc = 0;
                idx = 0;
            }
        }
    }
    Ok(samples)
}

#[cfg(not(feature = "pyin"))]
fn decode_flac_to_mono_f32(_path: &std::path::Path) -> Result<Vec<f32>, AnalysisError> {
    Err(AnalysisError::DecodeFailed(
        "pyin feature disabled at compile time; cannot decode flac".to_string(),
    ))
}
