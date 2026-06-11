#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! When every voiced frame is exactly on the target MIDI,
//! `in_tune_fraction` is `1.0` and `mean_cents_error_abs` is `0.0`.

use approx::assert_relative_eq;
use neural_pitch_core::music::midi_to_hz;
use neural_pitch_core::pipeline::PitchUpdate;
use neural_pitch_core::training::{HitWindow, MatchUpdate, TargetMatcher};

const TARGET_MIDI: i32 = 60; // middle C
const A4_HZ: f32 = 440.0;
const EMIT_RATE_HZ: f32 = 93.0;

fn frame_at(midi: i32, ts: u64) -> PitchUpdate {
    let f0 = midi_to_hz(midi, A4_HZ);
    PitchUpdate {
        timestamp_samples: ts,
        f0_hz: f0,
        confidence: 0.95,
        voiced: true,
        smoothed_cents: 0.0,
        target_midi: midi,
        target_hz: f0,
    }
}

#[test]
fn ten_frames_on_target_score_perfectly() {
    let window = HitWindow {
        start_midi: TARGET_MIDI,
        end_midi: TARGET_MIDI,
        tolerance_cents: 20.0,
    };
    // ring_window_ms long enough to retain all 10 frames; output_hz low
    // enough that we drive the matcher with `flush()` for a deterministic
    // result.
    let mut matcher = TargetMatcher::with_params(
        window,
        EMIT_RATE_HZ,
        /* ring_window_ms = */ 1_000,
        /* output_hz = */ 1.0,
    );

    for i in 0..10u64 {
        let _ignored: Option<MatchUpdate> =
            matcher.push(frame_at(TARGET_MIDI, i * 256), TARGET_MIDI, A4_HZ);
    }

    let final_update: MatchUpdate = matcher.flush();
    assert_relative_eq!(final_update.in_tune_fraction, 1.0_f32, epsilon = 1e-4);
    assert_relative_eq!(final_update.mean_cents_error_abs, 0.0_f32, epsilon = 1e-3);
    assert_eq!(final_update.frames_observed, 10);
}
