//! Audio I/O abstractions.
//!
//! Phase 1.0 defined only the [`AudioBlock`] data shape and the
//! [`AudioDecoder`] trait. Phase 1.1 adds the live-capture surface:
//! [`AudioBackend`], [`AudioBackendConfig`], [`AudioBackendEvent`], and the
//! always-on [`MockAudioBackend`] used by Tier-2 deterministic tests. The
//! cpal-backed [`CpalAudioBackend`] is gated behind `#[cfg(feature = "cpal")]`.
//! Concrete file decoder/encoder implementations (Symphonia, FLAC, WAV) land
//! in Phase 2 behind feature gates; see `docs/design/DESIGN.md` §5.2.

use std::io;

use thiserror::Error;

pub mod backend;
pub mod mock_backend;

#[cfg(feature = "cpal")]
pub mod cpal_backend;

pub use backend::{AudioBackend, AudioBackendConfig, AudioBackendError, AudioBackendEvent};
pub use mock_backend::{MockAudioBackend, Pacing, SampleSource};

#[cfg(feature = "cpal")]
pub use cpal_backend::CpalAudioBackend;

/// One block of contiguous PCM audio samples, interleaved if multi-channel.
///
/// `timestamp_samples` is the absolute sample index of the first frame in
/// the block, measured from the start of the source. It is used by the
/// pipeline to align analysis frames with the underlying audio.
#[derive(Debug, Clone)]
pub struct AudioBlock {
    /// Interleaved PCM samples in `f32` (range `[-1.0, 1.0]`).
    pub samples: Vec<f32>,
    /// Sample rate of `samples`, in Hertz.
    pub sample_rate_hz: u32,
    /// Number of interleaved channels.
    pub channels: u16,
    /// Sample-accurate timestamp of the first frame in the block.
    pub timestamp_samples: u64,
}

/// Errors raised by audio decoders, encoders, and I/O glue.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AudioError {
    /// An OS-level I/O error occurred while reading or writing audio.
    #[error("audio I/O error: {0}")]
    Io(#[from] io::Error),

    /// The container or codec format was unsupported, malformed, or
    /// otherwise unreadable.
    #[error("audio format error: {0}")]
    Format(String),
}

/// File-format-agnostic decoder trait.
///
/// Phase 1 defines only the static `supported_extensions()` method. Streaming
/// decode is intentionally deferred — see the design doc for the planned
/// `decode_block(&mut self) -> Result<Option<AudioBlock>, AudioError>` API.
pub trait AudioDecoder: Send {
    /// File extensions this decoder claims to support, lowercase, without
    /// the leading dot. Used by the format-detection dispatcher to pick a
    /// concrete decoder for a given file.
    fn supported_extensions(&self) -> &'static [&'static str];

    // TODO(phase-2): add `decode_block(&mut self) -> Result<Option<AudioBlock>, AudioError>`
    // for streaming decode. Implementations will hold an internal Symphonia
    // reader and yield ~1024-sample blocks.
}
