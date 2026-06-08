#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args
)]

//! Phase 3 RED — resampler integration test.
//!
//! Generate the same 440 Hz tone at 48 kHz so [`BasicPitchEstimator::analyze`]
//! must invoke the rubato resampler internally before windowing. The result
//! MUST still report MIDI 69 (proving the resample preserved pitch) and the
//! reported `frame_rate_hz` MUST equal the model's native rate
//! `22_050 / 256 ≈ 86.1328` Hz (proving the resample landed in 22.05 kHz
//! land before the windowing stage).

use neural_pitch_core::poly::PolyEstimator;
use neural_pitch_core::poly::basic_pitch::BasicPitchEstimator;
use neural_pitch_core::test_utils::signals::sine_wave;

const CAPTURE_SR_HZ: u32 = 48_000;
const TONE_DURATION_MS: u64 = 1_500;
const EXPECTED_FRAME_RATE_HZ: f32 = 22_050.0 / 256.0; // ≈ 86.1328

#[test]
fn basic_pitch_resamples_48khz_input_and_recovers_a4() {
    let n_samples = (CAPTURE_SR_HZ as u64 * TONE_DURATION_MS / 1_000) as usize;
    let audio = sine_wave(440.0, CAPTURE_SR_HZ, n_samples);

    let mut est = BasicPitchEstimator::from_bundled()
        .expect("bundled Basic Pitch v1 ONNX must load under the neural feature");

    let result = est.analyze(&audio, CAPTURE_SR_HZ).expect(
        "analyze must not error on 48 kHz input — the resampler should down-rate to 22.05 kHz",
    );

    let drift_hz = (result.frame_rate_hz - EXPECTED_FRAME_RATE_HZ).abs();
    assert!(
        drift_hz < 0.01,
        "frame_rate_hz drifted {drift_hz} Hz from the model-native \
         {expected} Hz (got {actual} Hz) — resampler may not be running",
        expected = EXPECTED_FRAME_RATE_HZ,
        actual = result.frame_rate_hz,
    );

    assert!(
        !result.notes.is_empty(),
        "a 1.5 s 440 Hz tone must produce at least one detected note after resampling",
    );

    let has_a4 = result.notes.iter().any(|n| n.midi == 69);
    assert!(
        has_a4,
        "after resampling 48 kHz → 22.05 kHz, 440 Hz must still be detected as MIDI 69 \
         (recovered MIDI numbers: {midis:?})",
        midis = result.notes.iter().map(|n| n.midi).collect::<Vec<_>>(),
    );
}
