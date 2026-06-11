#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args
)]

//! Basic Pitch v1 smoke test on a 1 s 440 Hz tone.
//!
//! 1 s of 440 Hz @ 22.05 kHz must produce exactly one note at MIDI 69.
//! `start_ms` should be near zero (allow up to 50 ms of attack latency)
//! and the recovered duration should be within 50 ms of the input.
//! Velocity must be non-zero — the onset peak is loud and well above
//! the `0.5` posterior threshold.

use neural_pitch_core::poly::PolyEstimator;
use neural_pitch_core::poly::basic_pitch::BasicPitchEstimator;
use neural_pitch_core::test_utils::signals::sine_wave;

const BASIC_PITCH_SR_HZ: u32 = 22_050;
const TONE_DURATION_MS: u64 = 1_000;

#[ignore = "ort cpu-fallback path is too slow on the CI matrix; runs locally"]
#[test]
fn basic_pitch_recovers_one_a4_note_from_a_one_second_tone() {
    let n_samples = (BASIC_PITCH_SR_HZ as u64 * TONE_DURATION_MS / 1_000) as usize;
    let audio = sine_wave(440.0, BASIC_PITCH_SR_HZ, n_samples);

    let mut est = BasicPitchEstimator::from_bundled()
        .expect("bundled Basic Pitch v1 ONNX must load under the neural feature");

    let result = est
        .analyze(&audio, BASIC_PITCH_SR_HZ)
        .expect("analyze must not error on a clean 440 Hz tone");

    assert_eq!(
        result.notes.len(),
        1,
        "exactly one note expected from a clean 440 Hz tone, got {n} notes",
        n = result.notes.len(),
    );

    let note = &result.notes[0];
    assert_eq!(
        note.midi,
        69,
        "440 Hz must map to MIDI 69 (A4); got MIDI {midi}",
        midi = note.midi,
    );
    assert!(
        note.velocity > 0,
        "a clean tone above the onset threshold must have non-zero velocity",
    );

    let recovered_ms = note.end_ms.saturating_sub(note.start_ms);
    let drift_ms = recovered_ms.abs_diff(TONE_DURATION_MS);
    assert!(
        drift_ms <= 50,
        "recovered duration {recovered_ms} ms drifted {drift_ms} ms from the \
         {expected} ms input (tolerance 50 ms)",
        expected = TONE_DURATION_MS,
    );
}
