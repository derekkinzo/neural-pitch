//! Phase 1.1 capture-to-update pipeline.
//!
//! The pipeline crate-internally couples three pieces:
//!
//! - [`crate::audio::AudioBackend`] (cpal in production, [`crate::audio::MockAudioBackend`]
//!   in tests) — pushes interleaved-mono `f32` samples into an SPSC ring.
//! - [`DspWorker`] — drains the ring, runs the configured pitch estimator,
//!   smoother, and VAD, and emits [`PitchUpdate`] frames.
//! - [`FrameSink`] — backend-agnostic delivery surface for updates. The
//!   default [`ChannelFrameSink`] wraps an `mpsc::Sender<PitchUpdate>`.

pub mod recording;
pub mod sink;
pub mod target_match;
pub mod worker;

#[cfg(feature = "flac")]
pub use recording::FlacRecordingSink;
pub use recording::{
    MockRecordingSink, RecordingArtifact, RecordingError, RecordingHandle, RecordingId,
    RecordingProgress, RecordingSink, RecordingSinkError, RecordingWorker,
};
pub use sink::{ChannelFrameSink, FrameSink, FrameSinkError, PitchUpdate};
pub use target_match::{DEFAULT_IN_WINDOW_CENTS, MatchEmitter, MatchUpdate, TargetMatcher};
pub use worker::{DspError, DspWorker, RecordingFanout};
