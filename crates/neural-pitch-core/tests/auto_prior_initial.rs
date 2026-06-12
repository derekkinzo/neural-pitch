//! Unit test: a freshly-constructed AutoPrior returns the generic
//! 60–2000 Hz range.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp
)]

use neural_pitch_core::pitch::auto_prior::{AutoPrior, GENERIC_RANGE};

#[test]
fn cold_start_returns_generic_range() {
    let mut p = AutoPrior::new(400);
    assert_eq!(p.current_range(), GENERIC_RANGE);
    assert_eq!(p.voiced_count(), 0);
}
