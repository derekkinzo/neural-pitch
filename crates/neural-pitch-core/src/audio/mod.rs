//! Audio I/O abstractions.
//!
//! Defines the [`AudioBlock`] data shape, the [`AudioDecoder`] trait, and
//! the live-capture surface: [`AudioBackend`], [`AudioBackendConfig`],
//! [`AudioBackendEvent`], and the always-on [`MockAudioBackend`] used by
//! deterministic tests. The cpal-backed [`CpalAudioBackend`] is gated
//! behind `#[cfg(feature = "cpal")]`. Concrete file decoder/encoder
//! implementations (FLAC) sit behind their own feature gates.

use std::io;

use thiserror::Error;

pub mod backend;
pub mod mock_backend;

#[cfg(feature = "cpal")]
pub mod cpal_backend;

pub use backend::{
    AudioBackend, AudioBackendConfig, AudioBackendError, AudioBackendEvent, AudioEventEmitter,
};
pub use mock_backend::{MockAudioBackend, Pacing, SampleSource};

#[cfg(feature = "cpal")]
pub use cpal_backend::{CpalAudioBackend, pick_buffer_size};

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
/// Surfaces only the static `supported_extensions()` method. Streaming
/// decode is intentionally out of scope; a `decode_block(&mut self) ->
/// Result<Option<AudioBlock>, AudioError>` extension belongs in a
/// follow-on trait so existing implementors stay source-compatible.
pub trait AudioDecoder: Send {
    /// File extensions this decoder claims to support, lowercase, without
    /// the leading dot. Used by the format-detection dispatcher to pick a
    /// concrete decoder for a given file.
    fn supported_extensions(&self) -> &'static [&'static str];
}
