//! 5 Hz / ±50 cent vibrato is recovered from the synthetic
//! smoothed-cents track.
//!
//! 5 s of contour at 93.75 fps with `smoothed_cents[i] = 50.0 *
//! sin(2*pi*5*t)` — i.e. a 5 Hz sinusoidal modulation of ±50 cents around
//! 0 cents (A4 = 440 Hz). The detector should report:
//!   * `(median_rate_hz - 5.0).abs() < 0.1` Hz — bin width is ~0.092 Hz,
//!     so a 0.1 Hz tolerance leaves room for the nearest-bin pick without
//!     letting the test pass on a 4 Hz or 6 Hz misfire.
//!   * `(median_extent_cents - 50.0).abs() < 5.0` — 10% relative
//!     tolerance accounts for FFT sidelobe leakage from the rectangular
//!     window without admitting a 30 cent or 80 cent answer.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use core::f32::consts::TAU;

use neural_pitch_core::analysis::contour::ContourResult;
use neural_pitch_core::analysis::vibrato::compute_vibrato;
use neural_pitch_core::pitch::F0Frame;

fn make_vibrato_contour(
    duration_s: f32,
    frame_rate_hz: f32,
    vibrato_rate_hz: f32,
    extent_cents: f32,
) -> ContourResult {
    let n_frames = (duration_s * frame_rate_hz).round() as usize;
    let mut smoothed_cents = Vec::with_capacity(n_frames);
    let mut frames = Vec::with_capacity(n_frames);
    for i in 0..n_frames {
        let t = i as f32 / frame_rate_hz;
        let cents = extent_cents * (TAU * vibrato_rate_hz * t).sin();
        // f0_hz = 440 * 2^(cents/1200)
        let f0_hz = 440.0_f32 * (cents / 1200.0).exp2();
        smoothed_cents.push(cents);
        frames.push(F0Frame {
            f0_hz,
            confidence: 0.95,
            voiced: true,
            timestamp_samples: (i as u64) * 512,
        });
    }
    ContourResult {
        frames,
        frame_rate_hz,
        smoothed_cents,
        voiced_ratio: 1.0,
        sample_count: (n_frames as u64) * 512,
        source_sample_rate_hz: 48_000,
        hop_size: 512,
        window_size: 2048,
    }
}

#[test]
fn vibrato_synthetic_5hz_50cents_recovered() {
    let contour = make_vibrato_contour(5.0, 93.75, 5.0, 50.0);

    let report = compute_vibrato(&contour, 440.0);

    let rate_err = (report.median_rate_hz - 5.0).abs();
    assert!(
        rate_err < 0.1,
        "median_rate_hz {} should be within 0.1 Hz of 5.0 (err = {rate_err})",
        report.median_rate_hz
    );

    let extent_err = (report.median_extent_cents - 50.0).abs();
    assert!(
        extent_err < 5.0,
        "median_extent_cents {} should be within 5 cents of 50.0 (err = {extent_err})",
        report.median_extent_cents
    );

    assert!(
        report.vibrato_ratio > 0.5,
        "vibrato_ratio {} should exceed 0.5 on a 5 s sustained-vibrato fixture",
        report.vibrato_ratio
    );
}
