//! Unit test: after 100 voiced frames at 220 Hz, the auto-prior's
//! range is within 0.5 Hz of `(220 * 2^-1.5, 220 * 2^+1.5)`.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp
)]

use neural_pitch_core::pitch::F0Frame;
use neural_pitch_core::pitch::auto_prior::AutoPrior;

#[test]
fn median_220hz_yields_pm_1_5_octaves() {
    let mut p = AutoPrior::new(400);
    for i in 0..100 {
        p.update(F0Frame {
            f0_hz: 220.0,
            confidence: 0.9,
            voiced: true,
            timestamp_samples: i,
        });
    }
    let (lo, hi) = p.current_range();
    let factor = 2.0_f32.powf(1.5);
    let expected_lo = 220.0_f32 / factor;
    let expected_hi = 220.0_f32 * factor;
    assert!(
        (lo - expected_lo).abs() < 0.5,
        "fmin {lo} should be within 0.5 Hz of {expected_lo}"
    );
    assert!(
        (hi - expected_hi).abs() < 0.5,
        "fmax {hi} should be within 0.5 Hz of {expected_hi}"
    );
}
