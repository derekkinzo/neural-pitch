//! Phase 2.1 — sub-window input for `analyze_contour`.
//!
//! `analysis::contour::analyze_contour` documents that "a degenerate
//! (window > samples) input is allowed — pYIN's Center framing pads the
//! signal so even a sub-frame buffer produces an output, just one with
//! very low confidence" (see contour.rs:114-116). The other pyin_*.rs
//! tests all use ≥ 0.5 s of audio, leaving the documented degenerate
//! path uncovered. This test feeds a 2048-sample buffer (half a 4096
//! window) and asserts the analyzer returns Ok with a non-empty frame
//! buffer, a finite voiced_ratio, and a smoothed_cents vector aligned
//! with frames.len().
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::analysis::contour::analyze_contour;
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};
use neural_pitch_core::test_utils::signals::sine_wave;

#[test]
fn analyze_contour_accepts_sub_window_input() {
    let cfg = EstimatorConfig {
        sample_rate_hz: 48_000,
        window_size: 4096,
        hop_size: 1024,
        fmin_hz: 60.0,
        fmax_hz: 1100.0,
        instrument_hint: Some(InstrumentHint::Voice),
    };
    // 2048 samples == half a window. Center-framing inside the pyin
    // backend pads-both-sides so this MUST still produce >= 1 frame.
    let samples = sine_wave(440.0, cfg.sample_rate_hz, 2048);

    let result = analyze_contour(&samples, &cfg, 440.0)
        .expect("analyze_contour MUST accept sub-window input per contour.rs:114-116");

    assert!(
        !result.frames.is_empty(),
        "Center padding must produce at least one frame for sub-window input; got {} frames",
        result.frames.len(),
    );
    assert!(
        result.voiced_ratio.is_finite(),
        "voiced_ratio must be finite even on degenerate input; got {}",
        result.voiced_ratio,
    );
    assert!(
        (0.0..=1.0).contains(&result.voiced_ratio),
        "voiced_ratio must lie in [0,1]; got {}",
        result.voiced_ratio,
    );
    assert_eq!(
        result.smoothed_cents.len(),
        result.frames.len(),
        "smoothed_cents must stay aligned with frames; \
         got smoothed_cents.len()={}, frames.len()={}",
        result.smoothed_cents.len(),
        result.frames.len(),
    );
    assert_eq!(
        result.sample_count,
        samples.len() as u64,
        "sample_count must round-trip the sub-window input length",
    );
    assert_eq!(
        u32::try_from(cfg.hop_size).unwrap(),
        result.hop_size,
        "hop_size must round-trip cfg.hop_size",
    );
    assert_eq!(
        u32::try_from(cfg.window_size).unwrap(),
        result.window_size,
        "window_size must round-trip cfg.window_size",
    );
}
