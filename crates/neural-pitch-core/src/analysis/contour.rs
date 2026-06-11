#![allow(clippy::doc_markdown)]
//! Offline pitch-contour analyser.
//!
//! [`analyze_contour`] is the single entry point: feed it raw `f32` samples
//! plus an [`EstimatorConfig`] and an `a4_hz` reference, and it returns a
//! [`ContourResult`] containing the full per-frame F0 contour, voiced ratio,
//! and a smoothed cents track.
//!
//! Pipeline:
//!   1. Construct a [`crate::pitch::pyin::PYinEstimator`] from `cfg`.
//!   2. Slide a hop-aligned window over `samples`, calling
//!      `process_with_range` with the (constructor-time) auto-prior range.
//!   3. Call [`crate::pitch::pyin::PYinEstimator::finalize`] to materialise
//!      the contour.
//!   4. Post-process via [`crate::smoothing::ContourSmoother`] to derive a
//!      cents track relative to the `a4_hz` reference.
//!   5. Compute `voiced_ratio` and assemble [`ContourResult`].

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::pitch::pyin::PYinEstimator;
use crate::pitch::{EstimatorConfig, EstimatorError, F0Frame, PitchEstimator};
use crate::smoothing::ContourSmoother;

/// Stable analyser-name constant for the pYIN backend's `analysis_cache`
/// rows. Mirrors `analyzer_name` in the persistence schema.
pub const PYIN_ANALYZER_NAME: &str = "pyin";

/// Stable analyser-version constant. Bump whenever pYIN's tuneables or the
/// on-the-wire [`ContourResult`] shape changes — this drives a cache miss
/// for every previously-cached row, forcing a re-analyse on next access.
///
/// Contributor invariant — bump in lock-step with ANY of:
///   * the field set or wire ordering of [`ContourResult`] (or any nested
///     type it embeds — e.g. [`crate::pitch::F0Frame`]),
///   * the analyzer parameters that materially change the f0 contour
///     (default fmin/fmax, smoothing window, hop/window defaults, voicing
///     threshold),
///   * the postcard format version itself.
///
/// Failure to bump leads to silent stale-cache hits where an old blob
/// decodes against the new shape and surfaces wrong values to the UI. The
/// `pyin_analyzer_version_bump` integration test guards the SQL key but
/// not the wire shape — that contract is owned by this comment.
pub const PYIN_ANALYZER_VERSION: &str = "0.2";

/// Default contour-smoother window in milliseconds for the offline analyser.
/// 80 ms matches the live tuner default (`commands.rs::SMOOTHER_MS`) so the
/// offline cents track is not visually different from a live capture of the
/// same audio.
const SMOOTHER_WINDOW_MS: f32 = 80.0;

/// Whole-file analysis result for one recording, one analyser, one version.
///
/// Serialised via `postcard` into the `analysis_cache.result_blob`
/// column. The on-the-wire shape is versioned by
/// [`PYIN_ANALYZER_VERSION`]; bump it whenever the field set changes in
/// a way older readers cannot ignore.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[must_use = "the analyzer's per-frame contour — dropping it discards the analysis"]
pub struct ContourResult {
    /// Per-frame F0 contour straight off `PYinEstimator::finalize`. May be
    /// empty for an unvoiced or near-silent recording.
    pub frames: Vec<F0Frame>,

    /// Frame rate in Hertz. For the 48 kHz / 1024-hop default this is
    /// `48000 / 1024 = 46.875`.
    pub frame_rate_hz: f32,

    /// `ContourSmoother` output expressed in cents relative to `a4_hz`.
    /// One entry per frame in [`Self::frames`]; unvoiced frames carry
    /// `f32::NAN` so the cents track stays the same length as `frames`.
    pub smoothed_cents: Vec<f32>,

    /// Fraction of output frames marked voiced after pYIN's HMM voicing
    /// decision and the post-filter residual-voicing gate (i.e. the value
    /// is `voiced_count / frames.len()` over the *post-smoother* frame
    /// stream, not a pure pYIN-`voiced_prob`-above-threshold metric).
    /// Range `[0.0, 1.0]`.
    pub voiced_ratio: f32,

    /// Total sample count of the source audio fed into [`analyze_contour`].
    pub sample_count: u64,

    /// Sample rate of the source audio in Hertz — preserved on the blob so
    /// readers can re-derive timing without re-decoding the FLAC.
    pub source_sample_rate_hz: u32,

    /// Hop size used by the analyzer, in samples. Preserved on the blob so
    /// downstream consumers (`get_contour_blocking`, frame-time
    /// reconstruction) do not have to assume the live-tuner default.
    pub hop_size: u32,

    /// Window size used by the analyzer, in samples. Same rationale as
    /// [`Self::hop_size`].
    pub window_size: u32,
}

/// Errors raised by [`analyze_contour`] and the Tauri commands that wrap it.
///
/// Distinct from [`crate::pitch::EstimatorError`]: the analyser owns the
/// FLAC decode + smoothing layers in addition to the estimator, so its
/// surface is a strict superset.
#[derive(Debug, Error)]
pub enum AnalysisError {
    /// The pYIN estimator returned an error during finalisation.
    #[error(transparent)]
    Estimator(#[from] EstimatorError),

    /// The decoded FLAC was zero-length or otherwise had no samples to
    /// analyse. Distinguishes "empty input" from a downstream estimator
    /// configuration error so callers can surface the correct UI message.
    #[error("input audio buffer is empty")]
    EmptyInput,

    /// Generic I/O failure (file open, FLAC decode buffer, etc.).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Analyse a recording end-to-end and return its contour.
///
/// `samples` must be mono `f32` PCM in `[-1.0, 1.0]`. `cfg.sample_rate_hz`
/// is the rate of `samples`; it is *not* re-derived from `cfg`. `a4_hz` is
/// the reference pitch used by the cents conversion downstream.
pub fn analyze_contour(
    samples: &[f32],
    cfg: &EstimatorConfig,
    a4_hz: f32,
) -> Result<ContourResult, AnalysisError> {
    if samples.is_empty() {
        return Err(AnalysisError::EmptyInput);
    }

    let frame_rate_hz = cfg.sample_rate_hz as f32 / cfg.hop_size as f32;
    let source_sample_rate_hz = cfg.sample_rate_hz;
    let sample_count = samples.len() as u64;

    // Build the estimator. A degenerate (window > samples) input is allowed
    // — pYIN's Center framing pads the signal so even a sub-frame buffer
    // produces an output, just one with very low confidence.
    let mut estimator = PYinEstimator::new(cfg.clone())?;

    // Feed in hop-aligned chunks. The estimator buffers internally and
    // ignores the hop alignment — the only reason we slice by hop_size is
    // to mirror the live-path pacing so a future bounded-lookahead variant
    // can drop in without re-plumbing this loop.
    //
    // We pass `process_with_range` with the full `cfg.fmin_hz`/`cfg.fmax_hz`
    // range; `auto_prior::range_for_hint` is the public lookup, but the
    // caller has already encoded their hint into `cfg` so re-deriving the
    // range here would double-apply it.
    let chunk_size = cfg.hop_size.max(1);
    let mut cursor = 0usize;
    while cursor < samples.len() {
        let end = (cursor + chunk_size).min(samples.len());
        let chunk = &samples[cursor..end];
        let _ = estimator.process_with_range(chunk, cfg.fmin_hz, cfg.fmax_hz)?;
        cursor = end;
    }

    let frames = estimator.finalize()?;

    // Post-process via the smoother + a cents conversion. The smoother
    // operates on `f0_hz`; we then convert the smoothed Hz to cents
    // relative to the `a4_hz` reference. Unvoiced frames carry NaN so the
    // returned `smoothed_cents` vector stays aligned with `frames`.
    let mut smoother = ContourSmoother::new(SMOOTHER_WINDOW_MS, source_sample_rate_hz);
    let mut smoothed_cents = Vec::with_capacity(frames.len());
    let mut voiced_count = 0usize;
    for frame in &frames {
        if frame.voiced {
            voiced_count += 1;
        }
        let smoothed = smoother.push(*frame);
        let cents = if smoothed.voiced && smoothed.f0_hz.is_finite() && smoothed.f0_hz > 0.0 {
            hz_to_cents(smoothed.f0_hz, a4_hz)
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

    Ok(ContourResult {
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

/// Convert a positive frequency in Hz to cents relative to `a4_hz`.
///
/// Returns `0.0` when `hz == a4_hz` exactly. Caller MUST guard against
/// non-finite or non-positive inputs; this helper is intentionally total
/// over the `(0, +inf)` domain.
fn hz_to_cents(hz: f32, a4_hz: f32) -> f32 {
    debug_assert!(hz > 0.0 && a4_hz > 0.0);
    1200.0 * (hz / a4_hz).log2()
}
