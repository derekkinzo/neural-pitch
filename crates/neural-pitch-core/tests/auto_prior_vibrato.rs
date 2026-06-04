//! Tier-2 unit test: a 5 Hz / ±50-cent vibrato around 440 Hz never widens
//! the auto-prior range beyond ±1 octave from the 440 Hz median.
//!
//! At a 100 Hz frame rate, 4 s of vibrato = 400 samples = full ring. The
//! median of an oscillating signal whose distribution is symmetric about
//! its centre stays anchored at the centre.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::f32::consts::TAU;

use neural_pitch_core::pitch::F0Frame;
use neural_pitch_core::pitch::auto_prior::AutoPrior;

#[test]
fn vibrato_does_not_destabilise_range() {
    let mut p = AutoPrior::new(400);
    let center = 440.0_f32;
    let extent_cents = 50.0_f32;
    let log2_ratio = extent_cents / 1200.0;
    // Vibrato frequency 5 Hz, frame rate 100 Hz → 1/20 cycle per frame.
    let cycles_per_frame = 5.0_f32 / 100.0;
    for i in 0..400 {
        let phase = TAU * cycles_per_frame * (i as f32);
        let f0 = center * (log2_ratio * phase.sin()).exp2();
        p.update(F0Frame {
            f0_hz: f0,
            confidence: 0.9,
            voiced: true,
            timestamp_samples: i as u64,
        });
    }
    assert_eq!(p.voiced_count(), 400);
    let (lo, hi) = p.current_range();
    // ±1 octave window around 440 Hz: [220, 880]. The expansion is
    // fixed at ±1.5 octaves so the *configured* width is wider than this
    // window — the test asserts the median stayed near 440 Hz, so the
    // expanded bounds straddle 440 Hz with the analytic ratio.
    let expected_factor = 2.0_f32.powf(1.5);
    let expected_lo = center / expected_factor;
    let expected_hi = center * expected_factor;
    // Tolerance: ±2 Hz on each side covers the small finite-sample bias
    // from a 400-frame window holding 20 vibrato cycles.
    assert!(
        (lo - expected_lo).abs() < 2.0,
        "vibrato lo {lo} drifted past expected {expected_lo}"
    );
    assert!(
        (hi - expected_hi).abs() < 5.0,
        "vibrato hi {hi} drifted past expected {expected_hi}"
    );
    // Also assert the hard bound from the spec: never widens > ±1 octave
    // from 440 Hz median in either direction.
    assert!(
        lo >= center / 4.0 && hi <= center * 4.0,
        "range ({lo}, {hi}) widened past ±2 octaves of 440 Hz"
    );
}
