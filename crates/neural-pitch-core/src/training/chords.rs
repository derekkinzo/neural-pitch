//! Chord quality classifier.
//!
//! Takes a slice of MIDI numbers (or pitch-classes) and returns the
//! detected [`ChordQuality`]. The classifier collapses to pitch-class
//! before matching, so inversions of the same chord normalise to the
//! same root-position quality.
//!
//! Sus2 and Sus4 share a single pitch-class set (`Csus2 = {0,2,7}` and
//! `Gsus4 = {0,2,7}` rotated to G), so for those qualities the caller
//! must supply the chord's root via [`ChordQuality::classify_with_root`]
//! to disambiguate. The legacy `classify` path does not consult a root
//! and therefore returns `None` for sus chords on purpose; callers that
//! do not know the root should treat sus as untyped.

/// Chord qualities recognised by drill prompts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ChordQuality {
    /// Major triad (1, 3, 5).
    Major,
    /// Minor triad (1, b3, 5).
    Minor,
    /// Diminished triad (1, b3, b5).
    Diminished,
    /// Augmented triad (1, 3, #5).
    Augmented,
    /// Suspended-2 triad (1, 2, 5).
    Sus2,
    /// Suspended-4 triad (1, 4, 5).
    Sus4,
    /// Dominant 7 (1, 3, 5, b7).
    Dominant7,
    /// Major 7 (1, 3, 5, 7).
    Major7,
    /// Minor 7 (1, b3, 5, b7).
    Minor7,
    /// Half-diminished 7 (1, b3, b5, b7).
    HalfDiminished7,
    /// Fully-diminished 7 (1, b3, b5, bb7).
    Diminished7,
}

impl ChordQuality {
    /// Classify a slice of MIDI numbers (or pitch classes) into a known
    /// quality. Inversions normalise to root-position before matching;
    /// duplicates and octave-doublings are ignored.
    ///
    /// Returns `None` when the pitch-class set does not match any
    /// supported quality OR when the set is ambiguous between Sus2 and
    /// Sus4 (`Csus2` and `Gsus4` share `{0, 2, 7}`); call
    /// [`Self::classify_with_root`] when the root is known.
    #[must_use]
    pub fn classify(midi_or_pcs: &[i32]) -> Option<Self> {
        // Collapse to a sorted pitch-class set.
        let mut pcs: Vec<i32> = midi_or_pcs.iter().map(|m| m.rem_euclid(12)).collect();
        pcs.sort_unstable();
        pcs.dedup();
        if pcs.is_empty() {
            return None;
        }

        // Try every rotation as the candidate root; on the first hit
        // for an *unambiguous* quality, return. Sus2/Sus4 share the
        // same pcs so we never resolve them through this path — only
        // `classify_with_root` can disambiguate.
        let n = pcs.len();
        for root_idx in 0..n {
            let root = pcs[root_idx];
            let mut intervals: Vec<i32> = pcs.iter().map(|p| (p - root).rem_euclid(12)).collect();
            intervals.sort_unstable();
            intervals.dedup();

            if let Some(quality) = match_intervals_unambiguous(&intervals) {
                return Some(quality);
            }
        }
        None
    }

    /// Classify against an explicit root pitch-class. Required to
    /// disambiguate Sus2 vs Sus4 (which share `{0, 2, 7}` mod-12) and
    /// preferred whenever the caller knows the chord's bass / lowest
    /// sounding pitch.
    #[must_use]
    pub fn classify_with_root(midi_or_pcs: &[i32], root_pc: i32) -> Option<Self> {
        let mut pcs: Vec<i32> = midi_or_pcs.iter().map(|m| m.rem_euclid(12)).collect();
        pcs.sort_unstable();
        pcs.dedup();
        if pcs.is_empty() {
            return None;
        }
        let root = root_pc.rem_euclid(12);
        let mut intervals: Vec<i32> = pcs.iter().map(|p| (p - root).rem_euclid(12)).collect();
        intervals.sort_unstable();
        intervals.dedup();
        if intervals.first() != Some(&0) {
            return None;
        }
        match_intervals(&intervals)
    }
}

/// Match a sorted, deduped, root-anchored pitch-class signature
/// (always starting with 0) against the canonical chord shapes,
/// including the sus-pair which is only resolvable with a known root.
fn match_intervals(intervals: &[i32]) -> Option<ChordQuality> {
    match intervals {
        [0, 4, 7] => Some(ChordQuality::Major),
        [0, 3, 7] => Some(ChordQuality::Minor),
        [0, 3, 6] => Some(ChordQuality::Diminished),
        [0, 4, 8] => Some(ChordQuality::Augmented),
        [0, 2, 7] => Some(ChordQuality::Sus2),
        [0, 5, 7] => Some(ChordQuality::Sus4),
        [0, 4, 7, 10] => Some(ChordQuality::Dominant7),
        [0, 4, 7, 11] => Some(ChordQuality::Major7),
        [0, 3, 7, 10] => Some(ChordQuality::Minor7),
        [0, 3, 6, 10] => Some(ChordQuality::HalfDiminished7),
        [0, 3, 6, 9] => Some(ChordQuality::Diminished7),
        _ => None,
    }
}

/// Match without sus chords. Used by the rotation-search path because
/// `Csus2` rotated through G also matches `Gsus4` and the rotation
/// search has no way to know which spelling the caller intended.
fn match_intervals_unambiguous(intervals: &[i32]) -> Option<ChordQuality> {
    match intervals {
        [0, 4, 7] => Some(ChordQuality::Major),
        [0, 3, 7] => Some(ChordQuality::Minor),
        [0, 3, 6] => Some(ChordQuality::Diminished),
        [0, 4, 8] => Some(ChordQuality::Augmented),
        [0, 4, 7, 10] => Some(ChordQuality::Dominant7),
        [0, 4, 7, 11] => Some(ChordQuality::Major7),
        [0, 3, 7, 10] => Some(ChordQuality::Minor7),
        [0, 3, 6, 10] => Some(ChordQuality::HalfDiminished7),
        [0, 3, 6, 9] => Some(ChordQuality::Diminished7),
        _ => None,
    }
}
