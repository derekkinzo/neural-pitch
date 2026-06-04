//! Crate-wide error type. Forwards to per-module error enums via `#[from]` so
//! callers can use `?` uniformly across pitch, music, audio, and pipeline
//! operations.

use thiserror::Error;

use crate::audio::{AudioBackendError, AudioError};
use crate::music::MusicError;
use crate::pipeline::FrameSinkError;
use crate::pitch::EstimatorError;
use crate::store::StoreError;

/// Cross-cutting error type for `neural-pitch-core`.
///
/// Each variant wraps the per-module error type. Downstream callers that want
/// fine-grained handling can match on the inner enums directly; callers that
/// only need a unified error surface can use `CoreError` with `?`.
#[derive(Debug, Error)]
pub enum CoreError {
    /// A pitch estimator returned an error.
    #[error(transparent)]
    Pitch(#[from] EstimatorError),

    /// Music-theory math (frequency-to-note, MIDI conversion) failed.
    #[error(transparent)]
    Music(#[from] MusicError),

    /// Audio I/O or decoding failed.
    #[error(transparent)]
    Audio(#[from] AudioError),

    /// Live audio capture (cpal, mock) failed.
    #[error(transparent)]
    AudioBackend(#[from] AudioBackendError),

    /// A pipeline frame sink failed to deliver an update.
    #[error(transparent)]
    Sink(#[from] FrameSinkError),

    /// A persistence-layer (`store::RecordingsLibrary`) operation failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}
