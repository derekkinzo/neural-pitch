//! A constant-pitch contour produces (almost) no vibrato detections.
//!
//! 5 s of perfectly steady 440 Hz (i.e. `smoothed_cents[i] == 0.0` for
//! every frame) should fail the 5-cent extent floor inside every
//! analysis window, leaving `vibrato_ratio` very near zero (the floor
//! is 0.05). Floating-point noise in the median-filter / FFT residual
//! could cause occasional spurious peaks; the 5% allowance protects
//! against that while still failing loudly if a regression starts
//! emitting vibrato out of pure DC.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::analysis::contour::ContourResult;
use neural_pitch_core::analysis::vibrato::compute_vibrato;
use neural_pitch_core::pitch::F0Frame;

fn make_constant_contour(duration_s: f32, frame_rate_hz: f32) -> ContourResult {
    let n_frames = (duration_s * frame_rate_hz).round() as usize;
    let smoothed_cents = vec![0.0_f32; n_frames];
    let frames: Vec<F0Frame> = (0..n_frames)
        .map(|i| F0Frame {
            f0_hz: 440.0,
            confidence: 0.95,
            voiced: true,
            timestamp_samples: (i as u64) * 512,
        })
        .collect();
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
fn vibrato_no_vibrato_returns_low_ratio() {
    let contour = make_constant_contour(5.0, 93.75);

    let report = compute_vibrato(&contour, 440.0);

    assert!(
        report.vibrato_ratio < 0.05,
        "constant 440 Hz must produce vibrato_ratio < 0.05; got {}",
        report.vibrato_ratio
    );
}
