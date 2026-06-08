#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! `ScaleMode::classify` recognises Ionian, Dorian, the major
//! pentatonic, and friends from their pitch-class sets *given* a
//! tonic. Without a tonic, all seven diatonic modes share the same
//! pitch-class set and are ambiguous; `enumerate_candidates` is the
//! escape hatch for free-form callers.

use neural_pitch_core::training::ScaleMode;

#[test]
fn ionian_seven_note_with_c_tonic() {
    assert_eq!(
        ScaleMode::classify(&[0, 2, 4, 5, 7, 9, 11], 0),
        Some(ScaleMode::Ionian)
    );
}

#[test]
fn dorian_seven_note_with_d_tonic() {
    // D Dorian shares the C-major pitch-class set, but with D=2 as
    // the tonic the interval signature resolves to Dorian.
    assert_eq!(
        ScaleMode::classify(&[2, 4, 5, 7, 9, 11, 0], 2),
        Some(ScaleMode::Dorian)
    );
}

#[test]
fn pentatonic_major_five_note_with_c_tonic() {
    assert_eq!(
        ScaleMode::classify(&[0, 2, 4, 7, 9], 0),
        Some(ScaleMode::PentatonicMajor)
    );
}

#[test]
fn diatonic_pcs_enumerate_all_modal_candidates() {
    // The C-major pitch-class set is a rotation of every diatonic mode;
    // enumerate_candidates surfaces all seven (mode, tonic) pairs.
    let candidates = ScaleMode::enumerate_candidates(&[0, 2, 4, 5, 7, 9, 11]);
    assert_eq!(candidates.len(), 7, "got: {candidates:?}");
    assert!(candidates.contains(&(ScaleMode::Ionian, 0)));
    assert!(candidates.contains(&(ScaleMode::Dorian, 2)));
    assert!(candidates.contains(&(ScaleMode::Aeolian, 9)));
}

#[test]
fn classify_returns_none_when_tonic_not_in_pcs() {
    // Tonic = D (pc 2) but the pcs do not contain 2 — undefined.
    assert_eq!(ScaleMode::classify(&[0, 4, 7], 2), None);
}
