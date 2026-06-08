#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args,
    clippy::explicit_iter_loop
)]

//! Phase 3 RED — chord recall test.
//!
//! Sum three equal-amplitude sinusoids at A4 (MIDI 69), C#5 (MIDI 73), and
//! E5 (MIDI 76) — an A-major triad — at 22.05 kHz. The recovered note set
//! MUST be a *superset* of `{69, 73, 76}`. We assert recall, not precision:
//! Basic Pitch v1 is imperfect on synthetic sines and may emit a handful of
//! spurious notes (octave doubles, weak harmonics). The test passes as long
//! as the three intended pitches are all present.

use core::f32::consts::TAU;

use neural_pitch_core::poly::PolyEstimator;
use neural_pitch_core::poly::basic_pitch::BasicPitchEstimator;

const BASIC_PITCH_SR_HZ: u32 = 22_050;
const TONE_DURATION_MS: u64 = 1_500;
// A4, C#5, E5 — A-major triad.
const A4_HZ: f32 = 440.0;
const CS5_HZ: f32 = 554.365_3;
const E5_HZ: f32 = 659.255_1;

fn three_tone(sample_rate_hz: u32, n_samples: usize) -> Vec<f32> {
    let sr = sample_rate_hz as f32;
    let mut out = Vec::with_capacity(n_samples);
    for n in 0..n_samples {
        let t = n as f32 / sr;
        let a = (TAU * A4_HZ * t).sin();
        let b = (TAU * CS5_HZ * t).sin();
        let c = (TAU * E5_HZ * t).sin();
        out.push((a + b + c) / 3.0);
    }
    // Peak-normalise to ~0.95 so onset posteriors stay well above 0.5.
    let peak = out.iter().map(|v| v.abs()).fold(0.0_f32, f32::max);
    if peak > 0.0 {
        let g = 0.95 / peak;
        for v in out.iter_mut() {
            *v *= g;
        }
    }
    out
}

#[test]
// On the GitHub-hosted ubuntu-latest test matrix this test ran for >70
// minutes before being cancelled (locally it finishes in ~14 seconds).
// The likely cause is ort's bundled ONNX Runtime falling back to a
// generic CPU codepath on the runner. The smaller basic_pitch_*
// tests run in <2s under the same configuration, so the chord test is
// the outlier. Marked #[ignore] for the test matrix; surface it via
// the voice-acceptance / dedicated nightly job in a follow-up.
#[ignore = "ort cpu-fallback path is too slow on the CI matrix; runs locally"]
fn basic_pitch_recovers_an_a_major_triad() {
    let n_samples = (BASIC_PITCH_SR_HZ as u64 * TONE_DURATION_MS / 1_000) as usize;
    let audio = three_tone(BASIC_PITCH_SR_HZ, n_samples);

    let mut est = BasicPitchEstimator::from_bundled()
        .expect("bundled Basic Pitch v1 ONNX must load under the neural feature");

    let result = est
        .analyze(&audio, BASIC_PITCH_SR_HZ)
        .expect("analyze must not error on a clean three-tone chord");

    let recovered: std::collections::BTreeSet<u8> = result.notes.iter().map(|n| n.midi).collect();

    for expected in [69_u8, 73, 76] {
        assert!(
            recovered.contains(&expected),
            "chord recall: expected MIDI {expected} (A-major triad component) \
             missing from recovered set {recovered:?}",
        );
    }
}
