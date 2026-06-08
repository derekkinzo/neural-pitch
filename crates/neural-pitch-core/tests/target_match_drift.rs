#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Phase 4 RED — drift handling.
//!
//! Five frames at +10 cents (within tolerance) and five at +30 cents
//! (outside tolerance), with `tolerance_cents = 20.0`, must score
//! `in_tune_fraction == 0.5`.

use approx::assert_relative_eq;
use neural_pitch_core::music::midi_to_hz;
use neural_pitch_core::pipeline::PitchUpdate;
use neural_pitch_core::training::{HitWindow, MatchUpdate, TargetMatcher};

const TARGET_MIDI: i32 = 60;
const A4_HZ: f32 = 440.0;
const EMIT_RATE_HZ: f32 = 93.0;

/// Build a voiced `PitchUpdate` `cents_offset` cents *above* the target.
fn frame_off_cents(cents_offset: f32, ts: u64) -> PitchUpdate {
    let target_hz = midi_to_hz(TARGET_MIDI, A4_HZ);
    let f0 = target_hz * (cents_offset / 1200.0).exp2();
    PitchUpdate {
        timestamp_samples: ts,
        f0_hz: f0,
        confidence: 0.9,
        voiced: true,
        smoothed_cents: 0.0,
        target_midi: TARGET_MIDI,
        target_hz,
    }
}

#[test]
fn half_in_tune_half_drifting() {
    let window = HitWindow {
        start_midi: TARGET_MIDI,
        end_midi: TARGET_MIDI,
        tolerance_cents: 20.0,
    };
    let mut matcher = TargetMatcher::with_params(
        window,
        EMIT_RATE_HZ,
        /* ring_window_ms = */ 1_000,
        /* output_hz = */ 1.0,
    );

    for i in 0..5u64 {
        let _ = matcher.push(frame_off_cents(10.0, i * 256), TARGET_MIDI, A4_HZ);
    }
    for i in 5..10u64 {
        let _ = matcher.push(frame_off_cents(30.0, i * 256), TARGET_MIDI, A4_HZ);
    }

    let final_update: MatchUpdate = matcher.flush();
    assert_relative_eq!(final_update.in_tune_fraction, 0.5_f32, epsilon = 1e-3);
    assert_eq!(final_update.frames_observed, 10);
    // Mean abs error: (5 * 10 + 5 * 30) / 10 = 20 cents.
    assert_relative_eq!(final_update.mean_cents_error_abs, 20.0_f32, epsilon = 0.5);
}
