//! Tier-1 unit test: 99 voiced 220 Hz frames + 1 voiced 880 Hz frame
//! must not let the octave-doubled outlier dominate. The median picks the
//! 220 Hz cluster, so the upper bound stays well below 700 Hz.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp
)]

use neural_pitch_core::pitch::F0Frame;
use neural_pitch_core::pitch::auto_prior::AutoPrior;

#[test]
fn single_octave_outlier_does_not_pull_median() {
    let mut p = AutoPrior::new(400);
    for i in 0..99 {
        p.update(F0Frame {
            f0_hz: 220.0,
            confidence: 0.9,
            voiced: true,
            timestamp_samples: i,
        });
    }
    p.update(F0Frame {
        f0_hz: 880.0,
        confidence: 0.9,
        voiced: true,
        timestamp_samples: 99,
    });
    let (_, hi) = p.current_range();
    // 220 * 2^1.5 ≈ 622 Hz; if the outlier dragged the median up the
    // upper bound would shoot past 1000 Hz.
    assert!(hi < 700.0, "expected upper bound < 700 Hz, got {hi}");
}
