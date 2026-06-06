#![allow(missing_docs)]
#![cfg(feature = "pyin")]

//! Phase 2.1 TDD-RED: postcard round-trip of `ContourResult`.
//!
//! `analyze_contour` → `postcard::to_allocvec` → `postcard::from_bytes` must
//! reproduce the contour frame-by-frame within a 0.1 cent tolerance. Until
//! `analyze_contour` is implemented the test panics with `todo!` long before
//! the round-trip; that is the red signal.
//!
//! `F0Frame` deliberately does not derive `PartialEq` (NaN), so the
//! comparison goes through a cents-tolerance helper.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::analysis::contour::{ContourResult, analyze_contour};
use neural_pitch_core::pitch::{EstimatorConfig, F0Frame, InstrumentHint};
use neural_pitch_core::test_utils::signals::sine_wave;

fn frames_close(a: &F0Frame, b: &F0Frame) -> bool {
    if a.voiced != b.voiced {
        return false;
    }
    if a.timestamp_samples != b.timestamp_samples {
        return false;
    }
    if (a.confidence - b.confidence).abs() > 1e-4 {
        return false;
    }
    if !a.voiced {
        return true;
    }
    // For voiced frames, compare in cents — a sub-0.1 cent delta is well
    // below any precision the postcard encoding can introduce on f32.
    if !a.f0_hz.is_finite() || !b.f0_hz.is_finite() {
        return false;
    }
    let cents = 1200.0 * (a.f0_hz / b.f0_hz).log2();
    cents.abs() < 0.1
}

#[test]
fn pyin_cache_roundtrip_postcard_preserves_contour() {
    let cfg = EstimatorConfig {
        sample_rate_hz: 48_000,
        window_size: 4096,
        hop_size: 1024,
        fmin_hz: 60.0,
        fmax_hz: 1100.0,
        instrument_hint: Some(InstrumentHint::Voice),
    };
    // 1 s of clean 440 Hz so the contour is non-trivial.
    let samples = sine_wave(440.0, cfg.sample_rate_hz, cfg.sample_rate_hz as usize);

    let original = analyze_contour(&samples, &cfg, 440.0)
        .expect("analyze_contour should succeed once Phase 2.1 ships");

    // Serialise → bytes → deserialise.
    let bytes: Vec<u8> = postcard::to_allocvec(&original)
        .expect("postcard::to_allocvec must serialise ContourResult");
    assert!(
        !bytes.is_empty(),
        "postcard serialised an empty blob — schema regression"
    );
    let round_tripped: ContourResult = postcard::from_bytes(&bytes)
        .expect("postcard::from_bytes must deserialise the just-encoded blob");

    // Top-level scalar fields must round-trip exactly.
    assert!(
        (original.frame_rate_hz - round_tripped.frame_rate_hz).abs() < f32::EPSILON,
        "frame_rate_hz changed across round-trip: {} -> {}",
        original.frame_rate_hz,
        round_tripped.frame_rate_hz
    );
    assert_eq!(
        original.source_sample_rate_hz, round_tripped.source_sample_rate_hz,
        "source_sample_rate_hz changed across round-trip"
    );
    assert_eq!(
        original.sample_count, round_tripped.sample_count,
        "sample_count changed across round-trip"
    );
    assert!(
        (original.voiced_ratio - round_tripped.voiced_ratio).abs() < f32::EPSILON,
        "voiced_ratio changed across round-trip"
    );
    assert_eq!(
        original.frames.len(),
        round_tripped.frames.len(),
        "frames vector length changed across round-trip"
    );
    assert_eq!(
        original.smoothed_cents.len(),
        round_tripped.smoothed_cents.len(),
        "smoothed_cents vector length changed across round-trip"
    );

    // Frame-by-frame approximate equality.
    for (i, (a, b)) in original
        .frames
        .iter()
        .zip(round_tripped.frames.iter())
        .enumerate()
    {
        assert!(
            frames_close(a, b),
            "frame {i} differs across round-trip: {a:?} vs {b:?}"
        );
    }

    for (i, (a, b)) in original
        .smoothed_cents
        .iter()
        .zip(round_tripped.smoothed_cents.iter())
        .enumerate()
    {
        if a.is_nan() && b.is_nan() {
            continue;
        }
        assert!(
            (a - b).abs() < 1e-4,
            "smoothed_cents[{i}] differs across round-trip: {a} vs {b}"
        );
    }
}
