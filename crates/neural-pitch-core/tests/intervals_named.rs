#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Phase 4 RED — named intervals expose semitone, cents, and
//! `up_from_midi` / `down_from_midi` helpers.
//!
//! Spec ties:
//! - `Interval::MinorThird.up_from_midi(60) == 63`
//! - `Interval::MajorThird.up_from_midi(69) == 73`
//! - `Interval::Tritone.up_from_midi(60) == 66`
//! - `Interval::MinorThird.cents() == 300.0` (equal-temperament cents).

use approx::assert_relative_eq;
use neural_pitch_core::training::Interval;

#[test]
fn minor_third_up_from_c4_is_eb4() {
    assert_eq!(Interval::MinorThird.up_from_midi(60), 63);
}

#[test]
fn major_third_up_from_a4_is_csharp5() {
    assert_eq!(Interval::MajorThird.up_from_midi(69), 73);
}

#[test]
fn tritone_up_from_c4_is_fsharp4() {
    assert_eq!(Interval::Tritone.up_from_midi(60), 66);
}

#[test]
fn minor_third_cents_is_three_hundred() {
    assert_relative_eq!(Interval::MinorThird.cents(), 300.0_f32, epsilon = 1e-4);
}

#[test]
fn perfect_fifth_semitones_is_seven() {
    assert_eq!(Interval::PerfectFifth.semitones(), 7);
}

#[test]
fn octave_down_from_c4_is_c3() {
    assert_eq!(Interval::Octave.down_from_midi(60), 48);
}
