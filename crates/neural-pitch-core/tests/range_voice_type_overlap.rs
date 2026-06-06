//! Phase 2.3 TDD-RED: voice-type hint reports overlapping types.
//!
//! 200 voiced frames spanning C3 (MIDI 48, ~130.81 Hz) up to C5
//! (MIDI 72, ~523.25 Hz) lands inside the New Grove range for both
//! tenor (C3–C5) and baritone (G2–G4) when the comfortable trim is
//! applied. The hint is informational only — we MUST NOT
//! collapse to a single type. The test asserts that at least both
//! `Tenor` and `Baritone` are present in the returned hint, and the
//! order is deterministic so the assertion stays stable across runs.
//!
//! Until [`compute_range`] is implemented this test panics with `todo!`,
//! which is the red signal.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::analysis::contour::ContourResult;
use neural_pitch_core::analysis::range::{VoiceType, compute_range};
use neural_pitch_core::pitch::F0Frame;

fn voiced_frame(f0_hz: f32, idx: u64) -> F0Frame {
    F0Frame {
        f0_hz,
        confidence: 0.95,
        voiced: true,
        timestamp_samples: idx * 512,
    }
}

fn make_contour(frames: Vec<F0Frame>) -> ContourResult {
    let n = frames.len();
    let smoothed_cents = vec![0.0_f32; n];
    ContourResult {
        frames,
        frame_rate_hz: 93.75,
        smoothed_cents,
        voiced_ratio: 1.0,
        sample_count: (n as u64) * 512,
        source_sample_rate_hz: 48_000,
        hop_size: 512,
        window_size: 2048,
    }
}

/// Convert MIDI number to Hz under a4=440 reference.
fn midi_to_hz(midi: i32) -> f32 {
    440.0_f32 * 2.0_f32.powf((midi - 69) as f32 / 12.0)
}

#[test]
fn range_voice_type_overlap_reports_tenor_and_baritone() {
    // Distribute 200 frames evenly across MIDI 48..=72 (C3..=C5).
    // This is exactly the tenor range and overlaps the upper part of
    // the baritone range.
    let mut frames: Vec<F0Frame> = Vec::with_capacity(200);
    let span: i32 = 72 - 48;
    for i in 0..200 {
        let midi = 48 + (i * span) / 199;
        let hz = midi_to_hz(midi);
        frames.push(voiced_frame(hz, i as u64));
    }
    let contour = make_contour(frames);

    let report = compute_range(&contour, 440.0);

    let hint = report
        .voice_type_hint
        .as_ref()
        .expect("voice_type_hint must be Some when ≥50 voiced frames are present");

    assert!(
        hint.contains(&VoiceType::Tenor),
        "voice_type_hint must include Tenor for a C3–C5 contour; got {hint:?}"
    );
    assert!(
        hint.contains(&VoiceType::Baritone),
        "voice_type_hint must include Baritone for a C3–C5 contour; got {hint:?}"
    );
}
