#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! `ChordQuality::classify` over root-position triads, seventh chords,
//! a first-inversion case that must collapse to the same root-position
//! match, the Db / C# enharmonic equivalence, and the Sus2 / Sus4
//! disambiguation that requires an explicit root.
//!
//! Spec ties:
//! - `[0, 4, 7] -> Major`
//! - `[0, 3, 7] -> Minor`
//! - `[0, 3, 6] -> Diminished`
//! - `[0, 4, 7, 10] -> Dominant7`
//! - `[0, 4, 7, 11] -> Major7`
//! - inversions normalise: `[4, 7, 12] -> Major` (same as `[0, 4, 7]`).
//! - enharmonic equivalence: `[1, 5, 8]` (Db/C#) -> Major.
//! - sus disambiguation: `Csus2` / `Gsus4` share `{0, 2, 7}`; the
//!   caller MUST supply the root to resolve.

use neural_pitch_core::training::ChordQuality;

#[test]
fn major_triad_root_position() {
    assert_eq!(
        ChordQuality::classify(&[0, 4, 7]),
        Some(ChordQuality::Major)
    );
}

#[test]
fn minor_triad_root_position() {
    assert_eq!(
        ChordQuality::classify(&[0, 3, 7]),
        Some(ChordQuality::Minor)
    );
}

#[test]
fn diminished_triad_root_position() {
    assert_eq!(
        ChordQuality::classify(&[0, 3, 6]),
        Some(ChordQuality::Diminished)
    );
}

#[test]
fn dominant_seventh_root_position() {
    assert_eq!(
        ChordQuality::classify(&[0, 4, 7, 10]),
        Some(ChordQuality::Dominant7)
    );
}

#[test]
fn major_seventh_root_position() {
    assert_eq!(
        ChordQuality::classify(&[0, 4, 7, 11]),
        Some(ChordQuality::Major7)
    );
}

#[test]
fn first_inversion_major_triad_normalises() {
    // [E, G, C] (first inversion of C major) collapses to PC set
    // {0, 4, 7} and must classify as Major.
    assert_eq!(
        ChordQuality::classify(&[4, 7, 12]),
        Some(ChordQuality::Major)
    );
}

#[test]
fn enharmonic_db_major_equals_csharp_major() {
    // Db major = {Db, F, Ab} = {1, 5, 8}. Spelling-agnostic: pcs
    // collapse the same way regardless of accidental glyph, so the
    // classifier returns Major for either spelling.
    assert_eq!(
        ChordQuality::classify(&[1, 5, 8]),
        Some(ChordQuality::Major)
    );
}

#[test]
fn sus_pcs_alone_is_ambiguous_via_classify() {
    // `Csus2` = {C, D, G} = {0, 2, 7}; `Gsus4` = {G, C, D} = {0, 2, 7}.
    // The rotation-search path cannot pick a side, so it returns None.
    assert_eq!(ChordQuality::classify(&[0, 2, 7]), None);
}

#[test]
fn csus2_and_gsus4_disambiguate_via_root() {
    // Same pcs, different root → different quality.
    assert_eq!(
        ChordQuality::classify_with_root(&[0, 2, 7], 0),
        Some(ChordQuality::Sus2)
    );
    assert_eq!(
        ChordQuality::classify_with_root(&[0, 2, 7], 7),
        Some(ChordQuality::Sus4)
    );
}
