//! Golden test: every MIDI note number 0..=127 round-trips through
//! `midi_to_hz` -> `frequency_to_note` within 0.001 cents at A4 = 440 Hz.

use neural_pitch_core::music::{frequency_to_note, midi_to_hz};

#[test]
fn midi_round_trip_within_thousandth_cent() {
    let a4 = 440.0_f32;
    for midi in 0..=127i32 {
        let hz = midi_to_hz(midi, a4);
        let reading = frequency_to_note(hz, a4);
        assert_eq!(
            reading.midi, midi,
            "midi {} round-tripped to {} (hz = {})",
            midi, reading.midi, hz
        );
        assert!(
            reading.cents.abs() < 0.001,
            "midi {} cents = {} (hz = {})",
            midi,
            reading.cents,
            hz
        );
    }
}
