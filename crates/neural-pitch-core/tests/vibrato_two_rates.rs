//! Two distinct vibrato rates appear in `per_window`.
//!
//! 6 s contour at 93.75 fps, with a 50 cent extent throughout. The first
//! 3 s carries 5 Hz vibrato; the second 3 s carries 7 Hz vibrato. The
//! detector slides 1-second windows with 50% overlap, so several
//! windows fall entirely in each half. The test asserts that:
//!   * at least one `VibratoWindow` reports a rate within 0.3 Hz of 5.0,
//!   * at least one reports a rate within 0.3 Hz of 7.0.
//!
//! 0.3 Hz is well above the FFT bin width (~0.092 Hz at 1024-bin /
//! 93.75 fps) but tight enough to keep a 5 Hz reading from
//! masquerading as 7 Hz.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use core::f32::consts::TAU;

use neural_pitch_core::analysis::contour::ContourResult;
use neural_pitch_core::analysis::vibrato::compute_vibrato;
use neural_pitch_core::pitch::F0Frame;

fn make_two_rate_contour(
    half_duration_s: f32,
    frame_rate_hz: f32,
    rate_a_hz: f32,
    rate_b_hz: f32,
    extent_cents: f32,
) -> ContourResult {
    let n_per_half = (half_duration_s * frame_rate_hz).round() as usize;
    let total = n_per_half * 2;
    let mut smoothed_cents = Vec::with_capacity(total);
    let mut frames = Vec::with_capacity(total);

    // First half: rate_a.
    for i in 0..n_per_half {
        let t = i as f32 / frame_rate_hz;
        let cents = extent_cents * (TAU * rate_a_hz * t).sin();
        let f0_hz = 440.0_f32 * (cents / 1200.0).exp2();
        smoothed_cents.push(cents);
        frames.push(F0Frame {
            f0_hz,
            confidence: 0.95,
            voiced: true,
            timestamp_samples: (i as u64) * 512,
        });
    }

    // Second half: rate_b. Continuous time: keep advancing `t` so the
    // sinusoid joins smoothly at the boundary (no discontinuity that
    // would inject broadband content into either window's FFT).
    for i in 0..n_per_half {
        let t = (n_per_half + i) as f32 / frame_rate_hz;
        let cents = extent_cents * (TAU * rate_b_hz * t).sin();
        let f0_hz = 440.0_f32 * (cents / 1200.0).exp2();
        smoothed_cents.push(cents);
        frames.push(F0Frame {
            f0_hz,
            confidence: 0.95,
            voiced: true,
            timestamp_samples: ((n_per_half + i) as u64) * 512,
        });
    }

    ContourResult {
        frames,
        frame_rate_hz,
        smoothed_cents,
        voiced_ratio: 1.0,
        sample_count: (total as u64) * 512,
        source_sample_rate_hz: 48_000,
        hop_size: 512,
        window_size: 2048,
    }
}

#[test]
fn vibrato_two_rates_yields_distinct_per_window_rates() {
    let contour = make_two_rate_contour(3.0, 93.75, 5.0, 7.0, 50.0);

    let report = compute_vibrato(&contour, 440.0);

    assert!(
        !report.per_window.is_empty(),
        "per_window must contain at least one detected window for a clean two-rate fixture"
    );

    let near_5 = report
        .per_window
        .iter()
        .any(|w| (w.rate_hz - 5.0).abs() < 0.3);
    let near_7 = report
        .per_window
        .iter()
        .any(|w| (w.rate_hz - 7.0).abs() < 0.3);

    assert!(
        near_5,
        "no per_window entry within 0.3 Hz of 5.0; got rates {:?}",
        report
            .per_window
            .iter()
            .map(|w| w.rate_hz)
            .collect::<Vec<_>>()
    );
    assert!(
        near_7,
        "no per_window entry within 0.3 Hz of 7.0; got rates {:?}",
        report
            .per_window
            .iter()
            .map(|w| w.rate_hz)
            .collect::<Vec<_>>()
    );
}
