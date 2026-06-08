#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::collapsible_match,
    clippy::uninlined_format_args
)]

//! Phase 3 RED — pitch-bend curve round-trip.
//!
//! Hand-build a [`PolyResult`] with a single A4 note carrying a linear
//! 0 → 50 cents pitch-bend curve over 500 ms (≈ 43 contour samples at
//! the Basic Pitch frame rate of 86.13 Hz). After SMF export and parse,
//! the track MUST contain at least 40 `PitchBend` events whose first
//! value is ≈ 8192 (centre = no bend) and whose last value is
//! ≈ 8192 + 50 * 4096 / 100 ≈ 10_240. A ±2 LSB tolerance accounts for
//! the 14-bit MIDI quantisation.

use midly::{MidiMessage, Smf, TrackEventKind};
use neural_pitch_core::poly::midi::{MidiExportOptions, poly_result_to_smf};
use neural_pitch_core::poly::{NoteEvent, PolyResult};

const FRAME_RATE_HZ: f32 = 22_050.0 / 256.0;
const N_CURVE_SAMPLES: usize = 43; // ≈ 500 ms at 86.13 Hz
const TARGET_END_CENTS: f32 = 50.0;

fn linear_bend_curve(n: usize, end_cents: f32) -> Vec<i16> {
    (0..n)
        .map(|i| {
            let frac = if n == 1 {
                0.0
            } else {
                i as f32 / (n - 1) as f32
            };
            (frac * end_cents).round() as i16
        })
        .collect()
}

#[test]
fn midi_export_emits_a_pitch_bend_stream_matching_the_curve() {
    let curve = linear_bend_curve(N_CURVE_SAMPLES, TARGET_END_CENTS);

    let result = PolyResult {
        notes: vec![NoteEvent {
            midi: 69,
            start_ms: 0,
            end_ms: 500,
            velocity: 100,
            pitch_bend_curve: Some(curve),
        }],
        frame_rate_hz: FRAME_RATE_HZ,
        model_version: "basic-pitch-1.0".to_string(),
        duration_ms: 500,
    };

    let bytes = poly_result_to_smf(&result, MidiExportOptions::default())
        .expect("poly_result_to_smf must succeed for a one-note buffer with a pitch-bend curve");

    let smf = Smf::parse(&bytes).expect("emitted SMF must parse cleanly");
    let track = smf
        .tracks
        .first()
        .expect("emitted SMF must contain at least one track");

    let bend_values: Vec<u16> = track
        .iter()
        .filter_map(|ev| match ev.kind {
            TrackEventKind::Midi { message, .. } => match message {
                MidiMessage::PitchBend { bend } => Some(bend.0.as_int()),
                _ => None,
            },
            _ => None,
        })
        .collect();

    assert!(
        bend_values.len() >= 40,
        "expected at least 40 PitchBend events to cover the 500 ms curve at \
         ≈ 86 Hz; got {n}",
        n = bend_values.len(),
    );

    // First bend should be at centre (no deviation).
    let first = i32::from(bend_values[0]);
    assert!(
        (first - 8192).abs() <= 2,
        "first PitchBend value {first} must be within ±2 LSB of centre (8192)",
    );

    // Last bend should be at centre + 50 cents in the ±2-semitone range:
    // 8192 + 50 * 4096 / 100 = 10_240.
    let expected_last = 8192 + (TARGET_END_CENTS * 4096.0 / 100.0).round() as i32;
    let last = i32::from(*bend_values.last().expect("at least one bend present"));
    assert!(
        (last - expected_last).abs() <= 2,
        "last PitchBend value {last} must be within ±2 LSB of expected {expected_last} \
         (centre + {cents} cents)",
        cents = TARGET_END_CENTS,
    );
}
