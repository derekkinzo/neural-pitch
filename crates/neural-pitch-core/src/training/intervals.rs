//! Named musical intervals + cents/MIDI helpers.
//!
//! [`Interval`] is the canonical enum used by drill specs and by the
//! karaoke ribbon target generator. All helpers are total functions; no
//! allocations.

/// Named musical interval (within one octave) used by drill prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Interval {
    /// Unison (0 semitones).
    Unison,
    /// Minor 2nd (1 semitone).
    MinorSecond,
    /// Major 2nd (2 semitones).
    MajorSecond,
    /// Minor 3rd (3 semitones).
    MinorThird,
    /// Major 3rd (4 semitones).
    MajorThird,
    /// Perfect 4th (5 semitones).
    PerfectFourth,
    /// Tritone / aug 4 / dim 5 (6 semitones).
    Tritone,
    /// Perfect 5th (7 semitones).
    PerfectFifth,
    /// Minor 6th (8 semitones).
    MinorSixth,
    /// Major 6th (9 semitones).
    MajorSixth,
    /// Minor 7th (10 semitones).
    MinorSeventh,
    /// Major 7th (11 semitones).
    MajorSeventh,
    /// Perfect octave (12 semitones).
    Octave,
}

impl Interval {
    /// Number of semitones spanned by this interval.
    #[must_use]
    pub fn semitones(self) -> i32 {
        match self {
            Self::Unison => 0,
            Self::MinorSecond => 1,
            Self::MajorSecond => 2,
            Self::MinorThird => 3,
            Self::MajorThird => 4,
            Self::PerfectFourth => 5,
            Self::Tritone => 6,
            Self::PerfectFifth => 7,
            Self::MinorSixth => 8,
            Self::MajorSixth => 9,
            Self::MinorSeventh => 10,
            Self::MajorSeventh => 11,
            Self::Octave => 12,
        }
    }

    /// Cents value for this interval in equal temperament (`semitones * 100.0`).
    #[must_use]
    pub fn cents(self) -> f32 {
        (self.semitones() as f32) * 100.0
    }

    /// Compute the MIDI note `self` semitones above `root_midi`.
    #[must_use]
    pub fn up_from_midi(self, root_midi: i32) -> i32 {
        root_midi + self.semitones()
    }

    /// Compute the MIDI note `self` semitones below `root_midi`.
    #[must_use]
    pub fn down_from_midi(self, root_midi: i32) -> i32 {
        root_midi - self.semitones()
    }
}
