//! Solfege syllables (movable & fixed do) and the canonical [`Note`]
//! shape used across drills.
//!
//! Movable-do is computed by `(midi - key_root_midi).rem_euclid(12)` and
//! looked up in the chromatic syllable table for the supplied
//! [`KeyMode`]. Fixed-do is a 12-entry pitch-class lookup.

use serde::{Deserialize, Serialize};

/// Solfege syllables, including chromatic raises (`Di`, `Ri`, `Fi`,
/// `Si`, `Li`) and chromatic lowers (`Ra`, `Me`, `Se`, `Le`, `Te`) used
/// by chromatic movable-do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SolfegeSyllable {
    /// Do (1).
    Do,
    /// Di (raised 1) — chromatic ascending.
    Di,
    /// Ra (lowered 2) — chromatic descending.
    Ra,
    /// Re (2).
    Re,
    /// Ri (raised 2) — chromatic ascending.
    Ri,
    /// Me (lowered 3) — chromatic descending.
    Me,
    /// Mi (3).
    Mi,
    /// Fa (4).
    Fa,
    /// Fi (raised 4) — chromatic ascending.
    Fi,
    /// Se (lowered 5) — chromatic descending.
    Se,
    /// Sol (5).
    Sol,
    /// Si (raised 5) — chromatic ascending.
    Si,
    /// Le (lowered 6) — chromatic descending.
    Le,
    /// La (6).
    La,
    /// Li (raised 6) — chromatic ascending.
    Li,
    /// Te (lowered 7) — chromatic descending.
    Te,
    /// Ti (7).
    Ti,
}

/// English natural note name (no accidentals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NoteName {
    /// C natural.
    C,
    /// D natural.
    D,
    /// E natural.
    E,
    /// F natural.
    F,
    /// G natural.
    G,
    /// A natural.
    A,
    /// B natural.
    B,
}

impl NoteName {
    /// Pitch-class for the natural form of this note name (`C = 0`,
    /// `D = 2`, ..., `B = 11`).
    #[must_use]
    pub fn pitch_class(self) -> i32 {
        match self {
            Self::C => 0,
            Self::D => 2,
            Self::E => 4,
            Self::F => 5,
            Self::G => 7,
            Self::A => 9,
            Self::B => 11,
        }
    }
}

/// Accidental for a spelled note. `Natural` carries no glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Accidental {
    /// No accidental.
    Natural,
    /// Sharp (`#`).
    Sharp,
    /// Flat (`b`).
    Flat,
}

/// La-vs-do based minor solfege convention.
///
/// `DoBased` (the default — preserves prior behaviour) anchors `Do`
/// at the minor tonic. `LaBased` is the older Kodály / European
/// convention where the minor scale is the relative-Aeolian rotation
/// of the major scale, so the minor tonic is `La` and the relative
/// major's tonic is `Do`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MinorMode {
    /// Tonic of the minor key is `Do` (modal-do convention).
    DoBased,
    /// Tonic of the minor key is `La` (Kodály / European convention).
    LaBased,
}

impl Default for MinorMode {
    fn default() -> Self {
        Self::DoBased
    }
}

/// Major / minor key context for solfege resolution. The `Minor`
/// variant carries an explicit [`MinorMode`] so callers can opt into
/// la-based minor (Kodály) without changing the ergonomic
/// `from_midi_in_key` constructor for major keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyMode {
    /// Major key.
    Major,
    /// Minor key. The variant carries the solfege convention so the
    /// previous default (do-based) can coexist with la-based callers.
    Minor(MinorMode),
}

impl KeyMode {
    /// Build a do-based minor [`KeyMode`] — the legacy default.
    #[must_use]
    pub const fn minor_do_based() -> Self {
        Self::Minor(MinorMode::DoBased)
    }

    /// Build a la-based minor [`KeyMode`] — the Kodály convention.
    #[must_use]
    pub const fn minor_la_based() -> Self {
        Self::Minor(MinorMode::LaBased)
    }
}

/// Direction hint for chromatic movable-do (`Ri` vs `Me` for raised 2 /
/// lowered 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    /// Ascending — prefer raised-degree spellings (`Di`, `Ri`, `Fi`, `Si`, `Li`).
    Ascending,
    /// Descending — prefer lowered-degree spellings (`Ra`, `Me`, `Se`, `Le`, `Te`).
    Descending,
}

impl Default for Direction {
    fn default() -> Self {
        Self::Ascending
    }
}

/// Canonical note shape used across drill specs, prompts, and results.
///
/// The MIDI number is the source of truth; spelled fields (accidental,
/// octave, name, solfege) are derived from `(midi, key_root, mode)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Note {
    /// MIDI note number, 0..=127.
    pub midi: i32,
    /// Accidental of the spelled note.
    pub accidental: Accidental,
    /// Scientific-pitch-notation octave (A4 is octave 4).
    pub octave: i32,
    /// English natural note name.
    pub name: NoteName,
    /// Solfege syllable in movable-do for the supplied key.
    pub solfege_movable: SolfegeSyllable,
    /// Solfege syllable in fixed-do (pitch-class only).
    pub solfege_fixed: SolfegeSyllable,
}

impl Note {
    /// Build a `Note` from a MIDI number in the supplied key context,
    /// defaulting to ascending direction for chromatic movable-do.
    #[must_use]
    pub fn from_midi_in_key(midi: i32, key_root: NoteName, mode: KeyMode) -> Self {
        Self::from_midi_in_key_dir(midi, key_root, mode, Direction::Ascending)
    }

    /// Build a `Note` from a MIDI number in the supplied key context
    /// with an explicit direction hint for chromatic syllables.
    #[must_use]
    pub fn from_midi_in_key_dir(
        midi: i32,
        key_root: NoteName,
        mode: KeyMode,
        direction: Direction,
    ) -> Self {
        let pc = midi.rem_euclid(12);
        // SPN octave: MIDI 60 = C4. So octave = (midi / 12) - 1, using
        // floored division so negatives behave correctly.
        let octave = midi.div_euclid(12) - 1;

        let prefer_flats = key_prefers_flats(key_root, mode);
        let (name, accidental) = spell_pitch_class(pc, prefer_flats);

        // Fixed-do anchors at C and uses ONLY the seven natural
        // syllables — accidentals show up as a glyph on `accidental`,
        // not as a chromatic syllable. Direction has no effect.
        let solfege_fixed = fixed_do_natural_for_name(name);

        // Movable-do reference depends on the minor convention. La-
        // based minor anchors `Do` at the relative-major root (a
        // minor third above the minor tonic).
        let movable_root_pc = match mode {
            KeyMode::Major | KeyMode::Minor(MinorMode::DoBased) => key_root.pitch_class(),
            KeyMode::Minor(MinorMode::LaBased) => (key_root.pitch_class() + 3).rem_euclid(12),
        };
        let degree = (pc - movable_root_pc).rem_euclid(12);
        let solfege_movable = movable_do_for_degree(degree, direction);

        Self {
            midi,
            accidental,
            octave,
            name,
            solfege_movable,
            solfege_fixed,
        }
    }
}

/// Whether the given key signature prefers flat accidentals over sharp.
///
/// Major keys with flats: F, Bb, Eb, Ab, Db, Gb. Of those, only `F`
/// has a natural-letter root in [`NoteName`]. Minor keys with flats:
/// D, G, C, F, Bb, Eb. C major / A minor pick sharp by default (no
/// flats; either is fine — sharp matches the rest of the crate).
fn key_prefers_flats(root: NoteName, mode: KeyMode) -> bool {
    matches!(
        (root, mode),
        (NoteName::F, KeyMode::Major | KeyMode::Minor(_))
            | (NoteName::D | NoteName::G | NoteName::C, KeyMode::Minor(_))
    )
}

/// Spell a pitch-class as `(NoteName, Accidental)` honouring the
/// caller's flat-vs-sharp preference. Pitch class is normalised to
/// `0..=11` before lookup so out-of-range MIDI inputs do not panic.
fn spell_pitch_class(pc: i32, prefer_flats: bool) -> (NoteName, Accidental) {
    match pc.rem_euclid(12) {
        0 => (NoteName::C, Accidental::Natural),
        1 if prefer_flats => (NoteName::D, Accidental::Flat),
        1 => (NoteName::C, Accidental::Sharp),
        2 => (NoteName::D, Accidental::Natural),
        3 if prefer_flats => (NoteName::E, Accidental::Flat),
        3 => (NoteName::D, Accidental::Sharp),
        4 => (NoteName::E, Accidental::Natural),
        5 => (NoteName::F, Accidental::Natural),
        6 if prefer_flats => (NoteName::G, Accidental::Flat),
        6 => (NoteName::F, Accidental::Sharp),
        7 => (NoteName::G, Accidental::Natural),
        8 if prefer_flats => (NoteName::A, Accidental::Flat),
        8 => (NoteName::G, Accidental::Sharp),
        9 => (NoteName::A, Accidental::Natural),
        10 if prefer_flats => (NoteName::B, Accidental::Flat),
        10 => (NoteName::A, Accidental::Sharp),
        // Pitch class is `rem_euclid(12)`-normalised so the only
        // remaining value is 11; the unreachable arm below keeps the
        // match exhaustive without a duplicate Do-flavoured fallback.
        _ => (NoteName::B, Accidental::Natural),
    }
}

/// Movable-do syllable for a scale degree (`0..=11`) in pitch classes
/// from the tonic, honouring the direction hint for chromatic
/// syllables.
fn movable_do_for_degree(degree: i32, direction: Direction) -> SolfegeSyllable {
    let asc = matches!(direction, Direction::Ascending);
    match degree.rem_euclid(12) {
        0 => SolfegeSyllable::Do,
        1 if asc => SolfegeSyllable::Di,
        1 => SolfegeSyllable::Ra,
        2 => SolfegeSyllable::Re,
        3 if asc => SolfegeSyllable::Ri,
        3 => SolfegeSyllable::Me,
        4 => SolfegeSyllable::Mi,
        5 => SolfegeSyllable::Fa,
        6 if asc => SolfegeSyllable::Fi,
        6 => SolfegeSyllable::Se,
        7 => SolfegeSyllable::Sol,
        8 if asc => SolfegeSyllable::Si,
        8 => SolfegeSyllable::Le,
        9 => SolfegeSyllable::La,
        10 if asc => SolfegeSyllable::Li,
        10 => SolfegeSyllable::Te,
        // Degree is `rem_euclid(12)`-normalised; the only remaining
        // value is 11. The fallback arm keeps the match exhaustive
        // without a duplicate `Do` body.
        _ => SolfegeSyllable::Ti,
    }
}

/// Fixed-do syllable for a natural [`NoteName`]. Standard
/// Romance / French Conservatoire / Italian fixed-do uses the seven
/// natural syllables only; accidentals are inflections of the same
/// syllable (`Db` is spoken `Do bémol`, still `Do`). The accidental
/// glyph already lives on [`Note::accidental`] so consumers compose
/// the spoken form by concatenating syllable + accidental themselves.
fn fixed_do_natural_for_name(name: NoteName) -> SolfegeSyllable {
    match name {
        NoteName::C => SolfegeSyllable::Do,
        NoteName::D => SolfegeSyllable::Re,
        NoteName::E => SolfegeSyllable::Mi,
        NoteName::F => SolfegeSyllable::Fa,
        NoteName::G => SolfegeSyllable::Sol,
        NoteName::A => SolfegeSyllable::La,
        NoteName::B => SolfegeSyllable::Ti,
    }
}
