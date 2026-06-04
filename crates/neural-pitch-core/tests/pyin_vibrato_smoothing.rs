//! Phase 2.1 TDD-RED: pYIN + smoother together must average vibrato.
//!
//! 5 Hz vibrato ±50 cents around 440 Hz over 1 s → median cents within 5 of
//! the true centre, range < 60 cents. The smoother is what compresses the
//! ±50 c excursion; until `analyze_contour` is implemented the test panics
//! with `todo!`.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::analysis::contour::analyze_contour;
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};
use neural_pitch_core::test_utils::signals::vibrato_signal;

#[test]
fn pyin_vibrato_smoothing_within_5_cents() {
    let cfg = EstimatorConfig {
        sample_rate_hz: 48_000,
        window_size: 4096,
        hop_size: 1024,
        fmin_hz: 60.0,
        fmax_hz: 1100.0,
        instrument_hint: Some(InstrumentHint::Voice),
    };
    // 1 second of 5 Hz vibrato, ±50 cents around 440 Hz.
    let samples = vibrato_signal(
        440.0,
        5.0,
        50.0,
        cfg.sample_rate_hz,
        cfg.sample_rate_hz as usize,
    );

    let result = analyze_contour(&samples, &cfg, 440.0)
        .expect("analyze_contour should succeed on a clean vibrato signal");

    // Smoothed cents track should compress the ±50 c excursion.
    let mut smoothed: Vec<f32> = result
        .smoothed_cents
        .iter()
        .copied()
        .filter(|c| c.is_finite())
        .collect();
    assert!(
        !smoothed.is_empty(),
        "no finite smoothed cents — pipeline regressed"
    );
    smoothed.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = smoothed.len();
    let median_cents = if n.is_multiple_of(2) {
        0.5 * (smoothed[n / 2 - 1] + smoothed[n / 2])
    } else {
        smoothed[n / 2]
    };

    assert!(
        median_cents.abs() < 5.0,
        "median smoothed cents = {median_cents} (>= 5)"
    );

    let lo = smoothed[0];
    let hi = smoothed[n - 1];
    let range = hi - lo;
    assert!(
        range < 60.0,
        "smoothed cents range = {range} (>= 60); smoother failed to flatten ±50 c vibrato"
    );

    // The vibrato signal is heavily voiced — we should see a high voiced
    // ratio even with the ±50 cent modulation.
    assert!(
        result.voiced_ratio > 0.9,
        "voiced_ratio {} <= 0.9 on a clean vibrato signal",
        result.voiced_ratio
    );
}
