//! Drill enum, drill specs, hit-windows, and per-session aggregate
//! results.

use serde::{Deserialize, Serialize};

use super::chords::ChordQuality;
use super::scales::ScaleMode;

/// One drill kind. Flat enum (no `Box<dyn>`) so it serialises cleanly
/// across the Tauri IPC boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Drill {
    /// Recognise a played interval (ascending / descending / harmonic).
    IntervalRecognition {
        /// Include ascending interval prompts.
        ascending: bool,
        /// Include descending interval prompts.
        descending: bool,
        /// Include harmonic (simultaneous) interval prompts.
        harmonic: bool,
    },
    /// Identify a chord's quality from a fixed pool.
    ChordQualityId {
        /// Pool of qualities the drill is allowed to draw from.
        qualities: Vec<ChordQuality>,
    },
    /// Identify a scale / mode from a fixed pool.
    ScaleId {
        /// Pool of modes the drill is allowed to draw from.
        modes: Vec<ScaleMode>,
    },
    /// Sight-singing exercise referenced by id.
    Sightreading {
        /// Catalogue id of the exercise.
        exercise_id: String,
    },
    /// Sustain-a-pitch tuning practice.
    TuningPractice {
        /// Target MIDI pitch.
        target_midi: i32,
    },
}

/// User-configurable drill parameters (length, tolerance, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillSpec {
    /// The drill kind + per-kind parameters.
    pub drill: Drill,
    /// Number of attempts in the session.
    pub attempts: u32,
    /// Cents tolerance applied to per-attempt scoring (TuningPractice
    /// and Sightreading hit-windows).
    pub tolerance_cents: f32,
}

/// Karaoke-ribbon hit-window. Single-note targets have
/// `start_midi == end_midi`; the matcher's slide-along-a-line
/// interpolation is out of scope — the field pair is treated as a
/// `[min_midi, max_midi]` envelope.
///
/// Concretely, [`crate::training::target_match::TargetMatcher`] flags a
/// frame in-tune iff:
///
/// 1. The frame's `nearest_midi` (from continuous f0) lies in
///    `[start_midi, end_midi]`, AND
/// 2. The frame's signed cents error against the caller-supplied
///    `target_midi` is within `tolerance_cents`.
///
/// True slide scoring (cents tolerance around the line connecting the
/// two endpoints, parameterised by `t_normalised`) is out of scope;
/// the matcher treats `[start_midi, end_midi]` as an envelope.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HitWindow {
    /// Lower MIDI bound, inclusive. For single-pitch targets,
    /// `start_midi == end_midi == target`.
    pub start_midi: i32,
    /// Upper MIDI bound, inclusive.
    pub end_midi: i32,
    /// Signed cents tolerance against the caller-supplied
    /// `target_midi`. NOT a tolerance around the slide line — the
    /// matcher does not interpolate.
    pub tolerance_cents: f32,
}

/// Per-attempt outcome captured during a drill session.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DrillAttempt {
    /// Whether the attempt was scored as correct.
    pub correct: bool,
    /// Absolute cents error for the attempt (mean of voiced frames).
    pub cents_error_abs: f32,
}

/// Aggregate result for a completed drill session.
///
/// Cached by the analysisStore for the recordings library + UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrillResult {
    /// The spec the session was generated from.
    pub spec: DrillSpec,
    /// Per-attempt outcomes (length == `spec.attempts`).
    pub attempts: Vec<DrillAttempt>,
    /// Mean of `attempts[i].cents_error_abs`.
    pub mean_cents_error_abs: f32,
}

impl DrillResult {
    /// Build a `DrillResult` from a spec and a slice of per-attempt
    /// outcomes.
    #[must_use]
    pub fn from_attempts(spec: DrillSpec, attempts: &[DrillAttempt]) -> Self {
        let mean_cents_error_abs = if attempts.is_empty() {
            0.0
        } else {
            let sum: f32 = attempts.iter().map(|a| a.cents_error_abs).sum();
            sum / attempts.len() as f32
        };
        Self {
            spec,
            attempts: attempts.to_vec(),
            mean_cents_error_abs,
        }
    }

    /// Fraction of attempts marked `correct` — `count(correct) / attempts.len()`.
    /// Returns `0.0` for an empty attempts vector.
    #[must_use]
    pub fn accuracy(&self) -> f32 {
        if self.attempts.is_empty() {
            return 0.0;
        }
        let correct = self.attempts.iter().filter(|a| a.correct).count();
        correct as f32 / self.attempts.len() as f32
    }
}
