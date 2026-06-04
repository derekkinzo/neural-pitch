//! Tier-1 unit test: a pinned `InstrumentHint::Bass` returns the
//! 35–500 Hz range no matter what voiced f0 history is on the ring.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp
)]

use neural_pitch_core::pitch::F0Frame;
use neural_pitch_core::pitch::InstrumentHint;
use neural_pitch_core::pitch::auto_prior::{AutoPrior, BASS_RANGE};

#[test]
fn bass_hint_overrides_voiced_history() {
    let mut p = AutoPrior::new(400).with_hint(InstrumentHint::Bass);
    // Inject a long stream of 1000 Hz voiced f0 — the auto-mode median
    // would never produce a bass range from this. The pinned hint must
    // win regardless.
    for i in 0..200 {
        p.update(F0Frame {
            f0_hz: 1000.0,
            confidence: 0.9,
            voiced: true,
            timestamp_samples: i,
        });
    }
    assert_eq!(p.current_range(), BASS_RANGE);
}
