//! Polyphonic transcription surface (Phase 3 — TDD-RED stubs).
//!
//! This module sits parallel to [`crate::pitch`] so the existing monophonic
//! [`crate::pitch::PitchEstimator`] surface stays untouched. Polyphonic
//! transcription has a fundamentally different output shape — multi-pitch
//! note events with onsets, durations, velocities, and optional pitch-bend
//! curves — so it gets its own [`PolyEstimator`] trait instead of squeezing
//! into the per-frame F0 contract.
//!
//! # TDD-RED status
//!
//! Every concrete entry point below currently panics with `todo!()`. The
//! Phase 3 GREEN step wires Basic Pitch v1 (Spotify, Apache-2.0) through
//! `ort` plus a `rubato` resampler, runs the per-bin Viterbi smoother from
//! [`crate::analysis::viterbi`], assembles note events from the onset /
//! note / contour heads, and emits SMF type-1 MIDI via `midly`.

use crate::pitch::EstimatorError;

pub mod basic_pitch;
pub mod midi;

/// One note event recovered from a polyphonic transcription pass.
///
/// `start_ms` and `end_ms` are millisecond offsets from the beginning of
/// the analysed audio buffer. `velocity` is in the standard MIDI range
/// `[1, 127]` (a recovered note never has velocity 0 — that would be a
/// note-off masquerading as a note-on). `pitch_bend_curve`, when present,
/// has exactly `end_frame - start_frame` samples in signed cents per
/// analysis frame; the frame rate is reported in [`PolyResult::frame_rate_hz`].
#[derive(Clone, Debug)]
pub struct NoteEvent {
    /// MIDI note number (`0..=127`). Basic Pitch v1's effective range is
    /// `21..=108` (88 piano keys).
    pub midi: u8,

    /// Onset timestamp, in milliseconds since the start of the analysed
    /// audio buffer.
    pub start_ms: u64,

    /// Offset timestamp, in milliseconds since the start of the analysed
    /// audio buffer. Always strictly greater than `start_ms`.
    pub end_ms: u64,

    /// MIDI velocity in the range `1..=127`.
    pub velocity: u8,

    /// Optional pitch-bend curve sampled at the contour frame rate
    /// (≈ 86.13 Hz for Basic Pitch v1). Each sample is a signed cents
    /// offset from the nominal MIDI pitch. `None` means the note had no
    /// detectable pitch deviation.
    pub pitch_bend_curve: Option<Vec<i16>>,
}

/// Output of a polyphonic transcription pass.
///
/// `notes` is unsorted — callers that need a deterministic order
/// (e.g. for snapshot tests) MUST sort by `(start_ms, midi)` themselves.
#[derive(Clone, Debug)]
pub struct PolyResult {
    /// Note events recovered from the input audio buffer.
    pub notes: Vec<NoteEvent>,

    /// Native frame rate of the underlying model's outputs, in Hertz.
    /// Basic Pitch v1 reports `22_050.0 / 256.0 ≈ 86.1328` Hz.
    pub frame_rate_hz: f32,

    /// Stable identifier for the model that produced this result, e.g.
    /// `"basic-pitch-1.0"`. Used for cache invalidation and provenance
    /// tracking.
    pub model_version: String,

    /// Total duration of the analysed audio buffer, in milliseconds.
    pub duration_ms: u64,
}

/// Backend-agnostic polyphonic transcription interface.
///
/// Distinct from [`crate::pitch::PitchEstimator`] because the output shape
/// (multi-pitch note events) does not fit the per-frame F0 contract. The
/// `Send` bound mirrors the monophonic trait so pipelines can hand a boxed
/// estimator to a dedicated worker thread.
pub trait PolyEstimator: Send {
    /// Stable identifier for this backend, e.g. `"basic-pitch-v1"`.
    fn name(&self) -> &str;

    /// Run polyphonic transcription on a mono audio buffer.
    ///
    /// `audio` is interpreted as mono PCM at `sample_rate_hz`. If the
    /// caller has stereo audio, it MUST mono-sum before calling — Phase 3
    /// keeps `analyze` mono-only by contract.
    fn analyze(&mut self, audio: &[f32], sample_rate_hz: u32)
    -> Result<PolyResult, EstimatorError>;
}
