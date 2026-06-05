//! Phase 2.1 TDD-RED: pYIN must report ~zero voicing on pure silence.
//!
//! 1 s of zero samples → `voiced_ratio < 0.05`. Until `analyze_contour` is
//! implemented the test panics with `todo!`.
#![cfg(feature = "pyin")]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::analysis::contour::analyze_contour;
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};
use neural_pitch_core::test_utils::signals::silence;

#[test]
fn pyin_silence_voiced_ratio_below_5_percent() {
    let cfg = EstimatorConfig {
        sample_rate_hz: 48_000,
        window_size: 4096,
        hop_size: 1024,
        fmin_hz: 60.0,
        fmax_hz: 1100.0,
        instrument_hint: Some(InstrumentHint::Voice),
    };
    // 1 second of pure silence.
    let samples = silence(cfg.sample_rate_hz as usize);
    let result = analyze_contour(&samples, &cfg, 440.0)
        .expect("analyze_contour should not error on silence (the analyzer is total)");

    assert!(
        result.voiced_ratio < 0.05,
        "silence reported voiced_ratio = {} (>= 0.05)",
        result.voiced_ratio
    );

    // Voicing-OFF frames should still produce timestamps so callers see a
    // continuous frame stream. Spec-internal: voiced ratio is the only
    // hard gate, but we sanity-check that at least *some* frames came back.
    assert!(
        !result.frames.is_empty(),
        "silence produced zero frames — frame-rate timing is corrupted"
    );
}
