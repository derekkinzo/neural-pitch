//! Convenience re-exports for downstream callers.
//!
//! `use neural_pitch_core::prelude::*;` brings in the most commonly used types
//! across the pitch detection and music theory modules.

pub use crate::music::{NoteReading, frequency_to_note, midi_to_hz};
pub use crate::pitch::{EstimatorConfig, EstimatorError, F0Frame, InstrumentHint, PitchEstimator};
