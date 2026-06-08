//! Scale / mode classifier.
//!
//! Identifies a pitch-class set as one of the supported scale modes. The
//! seven diatonic modes share a single pitch-class set (rotations of
//! `{0, 2, 4, 5, 7, 9, 11}`), so a tonic-free classifier cannot
//! distinguish them. The public surface therefore takes an explicit
//! `tonic_pc` argument and `enumerate_candidates` returns every
//! `(mode, tonic)` pair consistent with the pitch-class set so a caller
//! can disambiguate from melodic context.

/// Scale / mode names recognised by drill prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ScaleMode {
    /// Ionian (major).
    Ionian,
    /// Dorian.
    Dorian,
    /// Phrygian.
    Phrygian,
    /// Lydian.
    Lydian,
    /// Mixolydian.
    Mixolydian,
    /// Aeolian (natural minor).
    Aeolian,
    /// Locrian.
    Locrian,
    /// Major pentatonic (5-note).
    PentatonicMajor,
    /// Minor pentatonic (5-note).
    PentatonicMinor,
    /// Harmonic minor.
    HarmonicMinor,
    /// Melodic minor (ascending form).
    MelodicMinor,
    /// Blues scale.
    Blues,
}

impl ScaleMode {
    /// Classify a pitch-class set against an explicit `tonic_pc`. Pitch
    /// classes are normalised via `rem_euclid(12)` before matching so
    /// MIDI numbers, signed offsets, and pitch classes all work.
    ///
    /// Returns `None` when no mode matches the resulting interval
    /// signature *from* `tonic_pc`. This is the strict contract used by
    /// scoring code; for free-form classification (no tonic supplied)
    /// see [`Self::enumerate_candidates`].
    #[must_use]
    pub fn classify(midi_or_pcs: &[i32], tonic_pc: i32) -> Option<Self> {
        let mut pcs: Vec<i32> = midi_or_pcs.iter().map(|m| m.rem_euclid(12)).collect();
        pcs.sort_unstable();
        pcs.dedup();
        if pcs.is_empty() {
            return None;
        }

        let root = tonic_pc.rem_euclid(12);
        let mut intervals: Vec<i32> = pcs.iter().map(|p| (p - root).rem_euclid(12)).collect();
        intervals.sort_unstable();
        intervals.dedup();
        // The intervals slice MUST start with 0 — otherwise `tonic_pc`
        // is not a member of the input pitch-class set and the mode
        // is undefined for that pairing.
        if intervals.first() != Some(&0) {
            return None;
        }
        match_mode(&intervals)
    }

    /// Enumerate every `(mode, tonic_pc)` pair consistent with the
    /// supplied pitch-class set. The diatonic modes all share a single
    /// pitch-class set so this returns up to seven hits for a 7-note
    /// scale; the caller disambiguates from melodic context (typically
    /// the lowest sounding pitch on the down-beat).
    #[must_use]
    pub fn enumerate_candidates(midi_or_pcs: &[i32]) -> Vec<(Self, i32)> {
        let mut pcs: Vec<i32> = midi_or_pcs.iter().map(|m| m.rem_euclid(12)).collect();
        pcs.sort_unstable();
        pcs.dedup();
        if pcs.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<(Self, i32)> = Vec::new();
        for &root in &pcs {
            let mut intervals: Vec<i32> = pcs.iter().map(|p| (p - root).rem_euclid(12)).collect();
            intervals.sort_unstable();
            intervals.dedup();
            if let Some(mode) = match_mode(&intervals) {
                out.push((mode, root));
            }
        }
        out
    }
}

/// Match a sorted, deduped, root-anchored pitch-class signature
/// (always starting with 0) against the canonical mode shapes.
fn match_mode(intervals: &[i32]) -> Option<ScaleMode> {
    match intervals {
        // Diatonic 7-note modes.
        [0, 2, 4, 5, 7, 9, 11] => Some(ScaleMode::Ionian),
        [0, 2, 3, 5, 7, 9, 10] => Some(ScaleMode::Dorian),
        [0, 1, 3, 5, 7, 8, 10] => Some(ScaleMode::Phrygian),
        [0, 2, 4, 6, 7, 9, 11] => Some(ScaleMode::Lydian),
        [0, 2, 4, 5, 7, 9, 10] => Some(ScaleMode::Mixolydian),
        [0, 2, 3, 5, 7, 8, 10] => Some(ScaleMode::Aeolian),
        [0, 1, 3, 5, 6, 8, 10] => Some(ScaleMode::Locrian),
        // 7-note minor variants.
        [0, 2, 3, 5, 7, 8, 11] => Some(ScaleMode::HarmonicMinor),
        [0, 2, 3, 5, 7, 9, 11] => Some(ScaleMode::MelodicMinor),
        // 5-note pentatonics.
        [0, 2, 4, 7, 9] => Some(ScaleMode::PentatonicMajor),
        [0, 3, 5, 7, 10] => Some(ScaleMode::PentatonicMinor),
        // 6-note blues.
        [0, 3, 5, 6, 7, 10] => Some(ScaleMode::Blues),
        _ => None,
    }
}
