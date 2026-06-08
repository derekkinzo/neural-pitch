#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Phase 4 RED — movable-do solfege mapping.
//!
//! In C major, MIDI 60 (C4) → `Do`. In A major, MIDI 69 (A4) → `Do`.
//! That is, the tonic is always `Do` regardless of which key root is
//! supplied.

use neural_pitch_core::training::{KeyMode, Note, NoteName, SolfegeSyllable};

#[test]
fn c_in_c_major_is_do() {
    let note = Note::from_midi_in_key(60, NoteName::C, KeyMode::Major);
    assert_eq!(note.solfege_movable, SolfegeSyllable::Do);
}

#[test]
fn a_in_a_major_is_do() {
    let note = Note::from_midi_in_key(69, NoteName::A, KeyMode::Major);
    assert_eq!(note.solfege_movable, SolfegeSyllable::Do);
}

#[test]
fn fifth_degree_in_c_major_is_sol() {
    // G4 (MIDI 67) is the dominant of C major → `Sol`.
    let note = Note::from_midi_in_key(67, NoteName::C, KeyMode::Major);
    assert_eq!(note.solfege_movable, SolfegeSyllable::Sol);
}
