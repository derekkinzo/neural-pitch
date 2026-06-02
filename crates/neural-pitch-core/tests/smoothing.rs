//! Tests for `ContourSmoother`. The smoother is real day-1 code, so these
//! tests run as part of `cargo test`.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::pitch::F0Frame;
use neural_pitch_core::smoothing::ContourSmoother;

fn voiced_frame(hz: f32, ts: u64) -> F0Frame {
    F0Frame {
        f0_hz: hz,
        confidence: 1.0,
        voiced: true,
        timestamp_samples: ts,
    }
}

#[test]
fn ten_constant_frames_smooth_to_input() {
    let mut s = ContourSmoother::new(50.0, 48_000);
    let mut last = None;
    for i in 0..10 {
        last = Some(s.push(voiced_frame(440.0, i)));
    }
    let f = last.expect("at least one frame pushed");
    let cents_off = 1200.0 * (f.f0_hz / 440.0).log2();
    assert!(cents_off.abs() < 0.5, "smoothed off by {cents_off} cents");
    assert!(f.voiced);
}

#[test]
fn unvoiced_frames_pass_through_unchanged() {
    let mut s = ContourSmoother::new(50.0, 48_000);
    let unvoiced = F0Frame {
        f0_hz: 0.0,
        confidence: 0.0,
        voiced: false,
        timestamp_samples: 42,
    };
    let out = s.push(unvoiced);
    assert!(!out.voiced);
    assert_eq!(out.timestamp_samples, 42);
}

#[test]
fn reset_clears_history() {
    let mut s = ContourSmoother::new(50.0, 48_000);
    let _ = s.push(voiced_frame(440.0, 0));
    let _ = s.push(voiced_frame(440.0, 1));
    s.reset();
    let out = s.push(voiced_frame(880.0, 2));
    let cents_off = 1200.0 * (out.f0_hz / 880.0).log2();
    assert!(
        cents_off.abs() < 0.5,
        "after reset, smoother should not be biased by old 440 Hz history; off by {cents_off} cents",
    );
}
