#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! Chromatic movable-do with a direction hint.
//!
//! In C major:
//! - MIDI 61 (C#4 / Db4) → `Di` (raised Do, ascending direction default).
//! - MIDI 63 ascending → `Ri` (raised Re).
//! - MIDI 63 descending → `Me` (lowered Mi).

use neural_pitch_core::training::{Direction, KeyMode, Note, NoteName, SolfegeSyllable};

#[test]
fn raised_do_in_c_major_is_di() {
    let note = Note::from_midi_in_key(61, NoteName::C, KeyMode::Major);
    assert_eq!(note.solfege_movable, SolfegeSyllable::Di);
}

#[test]
fn raised_re_ascending_in_c_major_is_ri() {
    let note = Note::from_midi_in_key_dir(63, NoteName::C, KeyMode::Major, Direction::Ascending);
    assert_eq!(note.solfege_movable, SolfegeSyllable::Ri);
}

#[test]
fn lowered_mi_descending_in_c_major_is_me() {
    let note = Note::from_midi_in_key_dir(63, NoteName::C, KeyMode::Major, Direction::Descending);
    assert_eq!(note.solfege_movable, SolfegeSyllable::Me);
}
