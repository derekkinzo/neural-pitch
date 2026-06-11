#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args,
    clippy::explicit_iter_loop
)]

//! SMF export of a single A4 note.
//!
//! Hand-build a [`PolyResult`] with one A4 from 0..1000 ms, call
//! [`poly_result_to_smf`], parse the resulting bytes back through
//! `midly::Smf::parse`, and verify two invariants:
//!
//! 1. The track preamble emits the RPN 0 (Pitch-Bend Sensitivity)
//!    initialiser — the four-byte sequence `B0 65 00 / B0 64 00 /
//!    B0 06 02 / B0 26 00` — *before* the first NoteOn. Without this,
//!    DAWs that default to ±12 semitones (Logic, Reaper) would
//!    misinterpret the per-note pitch bend stream.
//! 2. A NoteOn with key 69 (A4) is present somewhere on the track.

use midly::{MetaMessage, MidiMessage, Smf, TrackEventKind};
use neural_pitch_core::poly::midi::{MidiExportOptions, poly_result_to_smf};
use neural_pitch_core::poly::{NoteEvent, PolyResult};

const FRAME_RATE_HZ: f32 = 22_050.0 / 256.0;

#[test]
fn midi_export_a4_includes_rpn_prelude_and_note_on_69() {
    let result = PolyResult {
        notes: vec![NoteEvent {
            midi: 69,
            start_ms: 0,
            end_ms: 1_000,
            velocity: 100,
            pitch_bend_curve: None,
        }],
        frame_rate_hz: FRAME_RATE_HZ,
        model_version: "basic-pitch-1.0".to_string(),
        duration_ms: 1_000,
    };

    let bytes = poly_result_to_smf(&result, MidiExportOptions::default())
        .expect("poly_result_to_smf must succeed for a one-note buffer");

    let smf = Smf::parse(&bytes).expect("emitted SMF must parse cleanly");
    assert!(
        !smf.tracks.is_empty(),
        "emitted SMF must contain at least one track",
    );
    let track = &smf.tracks[0];

    // Walk the track tracking RPN 0 prelude state-machine and the first NoteOn.
    let mut saw_rpn_msb_0 = false;
    let mut saw_rpn_lsb_0 = false;
    let mut saw_data_msb_2 = false;
    let mut saw_data_lsb_0 = false;
    let mut rpn_prelude_complete_before_note_on = false;
    let mut saw_note_on_69 = false;

    for ev in track.iter() {
        if let TrackEventKind::Midi { message, .. } = ev.kind {
            match message {
                MidiMessage::Controller { controller, value } => {
                    let cc = u8::from(controller);
                    let v = u8::from(value);
                    match (cc, v) {
                        (0x65, 0x00) => saw_rpn_msb_0 = true,
                        (0x64, 0x00) if saw_rpn_msb_0 => saw_rpn_lsb_0 = true,
                        (0x06, 0x02) if saw_rpn_lsb_0 => saw_data_msb_2 = true,
                        (0x26, 0x00) if saw_data_msb_2 => saw_data_lsb_0 = true,
                        _ => {}
                    }
                }
                MidiMessage::NoteOn { key, vel } => {
                    if saw_data_lsb_0 {
                        rpn_prelude_complete_before_note_on = true;
                    }
                    if u8::from(key) == 69 && u8::from(vel) > 0 {
                        saw_note_on_69 = true;
                        break;
                    }
                }
                _ => {}
            }
        }
        // Allow the SetTempo meta to appear anywhere before NoteOn — it's
        // not part of the prelude state-machine but its presence is
        // required for a well-formed type-1 SMF.
        if let TrackEventKind::Meta(MetaMessage::Tempo(_)) = ev.kind {
            // The decoder accepts the tempo meta; nothing to assert here.
        }
    }

    assert!(
        rpn_prelude_complete_before_note_on,
        "RPN 0 prelude (B0 65 00 / B0 64 00 / B0 06 02 / B0 26 00) must \
         appear in order before the first NoteOn — got msb_0={saw_rpn_msb_0}, \
         lsb_0={saw_rpn_lsb_0}, data_msb_2={saw_data_msb_2}, data_lsb_0={saw_data_lsb_0}",
    );
    assert!(
        saw_note_on_69,
        "an A4 NoteOn (key 69, velocity > 0) must be present on the track",
    );
}
