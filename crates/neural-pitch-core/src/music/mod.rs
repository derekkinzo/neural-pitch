//! Music theory math: frequency ↔ MIDI ↔ note-name conversions, cents.
//!
//! All public functions take `a4_hz` as an explicit parameter. There is no
//! module-level state (ADR-0005). The reference formula for cents-relative-
//! to-equal-temperament is:
//!
//! ```text
//! midi  = 69 + 12 * log2(f / a4_hz)
//! cents = 1200 * log2(f / expected_hz_for_nearest_midi)
//! ```
//!
//! where `expected_hz_for_nearest_midi = a4_hz * 2^((round(midi) - 69) / 12)`.

use thiserror::Error;

/// Lowest valid MIDI note number (C-1).
pub const MIDI_MIN: i32 = 0;
/// Highest valid MIDI note number (G9).
pub const MIDI_MAX: i32 = 127;
/// MIDI number of A4, the reference pitch.
pub const MIDI_A4: i32 = 69;

/// Errors returned by music-theory math.
#[derive(Debug, Error)]
pub enum MusicError {
    /// MIDI number outside the standard 0..=127 range.
    #[error("midi number {0} is outside the valid range 0..=127")]
    MidiOutOfRange(i32),

    /// Reference pitch `a4_hz` was not strictly positive or otherwise unusable.
    #[error("invalid A4 reference: {0} Hz (must be > 0 and finite)")]
    InvalidA4(f32),

    /// Frequency was not strictly positive or otherwise unusable.
    #[error("invalid frequency: {0} Hz (must be > 0 and finite)")]
    InvalidFrequency(f32),
}

/// One frequency-to-note reading.
///
/// `cents` is signed: negative means flat relative to the nearest equal-
/// temperament note, positive means sharp. For an in-tune frequency, the
/// magnitude SHOULD be small (well under 50 cents).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteReading {
    /// Nearest MIDI note number to the input frequency.
    pub midi: i32,
    /// Equal-temperament Hertz value of `midi` given the supplied `a4_hz`.
    pub expected_hz: f32,
    /// Signed deviation in cents from `expected_hz`. Range: `(-50.0, 50.0]`.
    pub cents: f32,
}

/// Convert a MIDI note number to its equal-temperament frequency.
///
/// Returns `a4_hz * 2^((midi - 69) / 12)`. This function does not validate
/// `midi` — out-of-range values produce mathematically valid (if musically
/// unusual) frequencies. Callers wanting validation should use the
/// `_checked` flavour or pre-validate.
pub fn midi_to_hz(midi: i32, a4_hz: f32) -> f32 {
    let semitones_from_a4 = (midi - MIDI_A4) as f32;
    a4_hz * (semitones_from_a4 / 12.0).exp2()
}

/// Convert a frequency in Hertz to its nearest equal-temperament note.
///
/// The returned [`NoteReading::cents`] is in the range `(-50.0, 50.0]`.
///
/// # Behaviour for invalid inputs
///
/// This function is total: it returns a `NoteReading` for any non-NaN finite
/// positive frequency. Callers that need to reject invalid inputs (zero,
/// negative, NaN, infinite) SHOULD pre-validate. For consistency with the
/// rest of the public API, NaN/zero/negative inputs collapse to MIDI 0 with
/// zero cents — this is documented, not a panic, and is unlikely to be
/// reached in practice because pitch estimators only emit positive `f0_hz`
/// when `voiced` is true.
pub fn frequency_to_note(f_hz: f32, a4_hz: f32) -> NoteReading {
    if !f_hz.is_finite() || f_hz <= 0.0 || !a4_hz.is_finite() || a4_hz <= 0.0 {
        return NoteReading {
            midi: 0,
            expected_hz: midi_to_hz(0, if a4_hz > 0.0 { a4_hz } else { 440.0 }),
            cents: 0.0,
        };
    }
    let semitones = 12.0 * (f_hz / a4_hz).log2();
    let midi_f = semitones + MIDI_A4 as f32;
    let midi = midi_f.round() as i32;
    let expected_hz = midi_to_hz(midi, a4_hz);
    let cents = 1200.0 * (f_hz / expected_hz).log2();
    NoteReading {
        midi,
        expected_hz,
        cents,
    }
}

/// English note name decomposition for a MIDI number.
///
/// Returns `(letter, accidental, octave)` where:
/// - `letter` is one of `'A'..='G'` (uppercase),
/// - `accidental` is `Some('#')` for sharp notes, `None` for naturals,
/// - `octave` is the scientific-pitch-notation octave number
///   (C4 is middle C; A4 is MIDI 69).
///
/// MIDI numbers outside `0..=127` are clamped for the letter/accidental
/// computation; the octave is computed without clamping so callers can
/// recognise out-of-range inputs.
pub fn note_name_english(midi: i32) -> (char, Option<char>, i32) {
    // pitch-class index in chromatic order starting at C
    let pc = midi.rem_euclid(12) as usize;
    // scientific-pitch-notation octave: MIDI 0 is C-1, MIDI 12 is C0
    let octave = midi.div_euclid(12) - 1;
    let names: [(char, Option<char>); 12] = [
        ('C', None),
        ('C', Some('#')),
        ('D', None),
        ('D', Some('#')),
        ('E', None),
        ('F', None),
        ('F', Some('#')),
        ('G', None),
        ('G', Some('#')),
        ('A', None),
        ('A', Some('#')),
        ('B', None),
    ];
    let (letter, accidental) = names[pc];
    (letter, accidental, octave)
}
