#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args
)]

//! Phase 3 RED — Issue #190 frame-drift mitigation regression test.
//!
//! A 5 s 440 Hz tone at 22.05 kHz spans roughly four `WINDOW_HOP =
//! AUDIO_N_SAMPLES - OVERLAP_FRAMES * FFT_HOP = 36_164` sample windows.
//! With the `OVERLAP_FRAMES = 30` overlap and `TRIM_FRAMES = 15` trim per
//! interior side, the stitched output MUST recover *exactly one* note
//! covering the whole span — no spurious onsets at the window-stitch
//! sample boundaries 36_164, 72_328, and 108_492.
//!
//! This is the regression test that pins the upstream Spotify
//! Issue #190 fix in our Rust port: without overlap+trim, a 5 s tone
//! would split into four notes at the window boundaries.

use neural_pitch_core::poly::PolyEstimator;
use neural_pitch_core::poly::basic_pitch::BasicPitchEstimator;
use neural_pitch_core::test_utils::signals::sine_wave;

const BASIC_PITCH_SR_HZ: u32 = 22_050;
const TONE_DURATION_MS: u64 = 5_000;

#[test]
fn basic_pitch_does_not_split_a_long_tone_at_window_boundaries() {
    let n_samples = (BASIC_PITCH_SR_HZ as u64 * TONE_DURATION_MS / 1_000) as usize;
    let audio = sine_wave(440.0, BASIC_PITCH_SR_HZ, n_samples);

    let mut est = BasicPitchEstimator::from_bundled()
        .expect("bundled Basic Pitch v1 ONNX must load under the neural feature");

    let result = est
        .analyze(&audio, BASIC_PITCH_SR_HZ)
        .expect("analyze must not error on a 5 s 440 Hz tone");

    let a4_notes: Vec<_> = result.notes.iter().filter(|n| n.midi == 69).collect();

    assert_eq!(
        a4_notes.len(),
        1,
        "5 s 440 Hz tone must produce exactly one MIDI 69 note (Issue #190 frame-drift fix); \
         got {n} A4 notes — likely a spurious split at a window-stitch boundary. \
         All recovered notes: {all:?}",
        n = a4_notes.len(),
        all = result
            .notes
            .iter()
            .map(|n| (n.midi, n.start_ms, n.end_ms))
            .collect::<Vec<_>>(),
    );

    let note = a4_notes[0];
    let recovered_ms = note.end_ms.saturating_sub(note.start_ms);
    // 5 s minus the trimming margin on both edges; allow ~250 ms tolerance.
    assert!(
        recovered_ms + 250 >= TONE_DURATION_MS,
        "the single A4 note must cover the whole 5 s span; got {recovered_ms} ms \
         (expected ≥ {min} ms)",
        min = TONE_DURATION_MS - 250,
    );
}
