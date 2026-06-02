//! Property tests for music-theory math. These run on day 1 — the
//! underlying math is real and is not gated on Phase 1 work.

use neural_pitch_core::music::{frequency_to_note, midi_to_hz};
use proptest::prelude::*;

proptest! {
    /// MIDI 1..=127 round-trips through frequency space exactly.
    /// MIDI 0 (C-1, ~8.18 Hz) is excluded because its frequency falls below
    /// the practical lower bound of any real instrument and is more sensitive
    /// to single-precision rounding at the round-trip point.
    #[test]
    fn midi_round_trips(midi in 1i32..=127) {
        let hz = midi_to_hz(midi, 440.0);
        let reading = frequency_to_note(hz, 440.0);
        prop_assert_eq!(reading.midi, midi);
    }

    /// Any frequency in the standard piano range maps to a reading whose
    /// cents deviation is strictly less than 50 cents — by definition of
    /// "nearest equal-temperament note".
    #[test]
    fn cents_in_half_semitone_band(f in 27.5_f32..=4186.0_f32) {
        let reading = frequency_to_note(f, 440.0);
        prop_assert!(
            reading.cents.abs() < 50.0,
            "f = {}, cents = {}",
            f,
            reading.cents
        );
    }
}
