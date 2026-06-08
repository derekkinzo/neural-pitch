#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args
)]

//! Phase 3 RED — silence MUST yield zero notes.
//!
//! 1 s of pure zeros at 22.05 kHz must produce an empty `notes` vector.
//! The simplest sanity check; also the cheapest test to drive an
//! end-to-end pipeline path through `from_bundled` + `analyze` before
//! the real ORT inference is wired.

use neural_pitch_core::poly::PolyEstimator;
use neural_pitch_core::poly::basic_pitch::BasicPitchEstimator;
use neural_pitch_core::test_utils::signals::silence;

const BASIC_PITCH_SR_HZ: u32 = 22_050;
const SILENCE_DURATION_MS: u64 = 1_000;

#[test]
fn basic_pitch_emits_no_notes_for_pure_silence() {
    let n_samples = (BASIC_PITCH_SR_HZ as u64 * SILENCE_DURATION_MS / 1_000) as usize;
    let audio = silence(n_samples);

    let mut est = BasicPitchEstimator::from_bundled()
        .expect("bundled Basic Pitch v1 ONNX must load under the neural feature");

    let result = est
        .analyze(&audio, BASIC_PITCH_SR_HZ)
        .expect("analyze must not error on a buffer of zeros");

    assert!(
        result.notes.is_empty(),
        "1 s of silence must produce zero notes; got {n}: {notes:?}",
        n = result.notes.len(),
        notes = result.notes.iter().map(|n| n.midi).collect::<Vec<_>>(),
    );
}
