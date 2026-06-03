//! Backend-agnostic audio capture trait surface.
//!
//! [`AudioBackend`] is the single boundary between live capture (cpal in
//! production, [`crate::audio::MockAudioBackend`] in tests) and the DSP
//! worker. Concrete backends push interleaved-mono `f32` samples into an
//! [`rtrb::Producer<f32>`]; the worker drains them on a dedicated `std::thread`
//! (ADR-0014).
//!
//! Out-of-band device events ([`AudioBackendEvent`]) will flow through a
//! `std::sync::mpsc::Sender<AudioBackendEvent>` wired in by the Phase 1.2
//! Tauri shell. Phase 1.1 only defines the data shape — the trait surface
//! does not yet thread an event sender, and concrete backends do not emit
//! events. When the wiring lands, events MUST NOT be funnelled through the
//! sample producer (P3, DESIGN §2.4); the producer is real-time-safe and
//! must not carry structured data.

use std::io;

use thiserror::Error;

/// Audio capture configuration negotiated between the backend and the DSP
/// worker.
///
/// `AudioBackendConfig` is the single source of truth for the analyzer's
/// expected geometry: sample rate, channel count, hop size, and analysis
/// window length. The DSP worker uses these to size its sliding-window buffer
/// and to compute the ring-buffer capacity (`next_pow2(3 * window)` per
/// DESIGN §6.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioBackendConfig {
    /// Sample rate of incoming audio, in Hertz.
    pub sample_rate: u32,
    /// Number of interleaved channels in the device callback. The backend
    /// downmixes to mono inline before pushing samples; the worker only ever
    /// sees mono `f32`.
    pub channels: u16,
    /// Hop size, in samples. The DSP worker advances its sliding window by
    /// this many samples per analysis frame.
    pub hop: usize,
    /// Analysis window length, in samples. Matches
    /// [`crate::pitch::EstimatorConfig::window_size`] of the active estimator.
    pub window: usize,
}

impl AudioBackendConfig {
    /// Recommended ring-buffer capacity for the SPSC channel between the
    /// audio callback and the DSP worker.
    ///
    /// Returns `next_pow2(3 * window)` so the producer never has to wrap
    /// inside a single analysis frame. This matches DESIGN §6.4.
    pub fn ring_capacity(&self) -> usize {
        let target = self.window.saturating_mul(3).max(1);
        let mut cap: usize = 1;
        while cap < target {
            // `next_pow2` saturates at `usize::MAX / 2 + 1`; in practice we
            // never come close to that, but defend the math anyway.
            match cap.checked_mul(2) {
                Some(v) => cap = v,
                None => return cap,
            }
        }
        cap
    }
}

/// Out-of-band events emitted by an audio backend.
///
/// These flow through a `std::sync::mpsc::Sender<AudioBackendEvent>` that
/// will be wired in Phase 1.2 (Tauri shell). Phase 1.1 only defines the
/// data shape — the trait surface does not yet thread an event sender, so
/// concrete backends do not currently emit events. Events are intentionally
/// out-of-band: they MUST NOT be funnelled through the sample producer,
/// which is real-time-safe and must not carry structured data.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AudioBackendEvent {
    /// The capture device disappeared (USB unplug, hot-swap, OS audio reset).
    Disconnected,
    /// The negotiated stream format changed mid-capture. Consumers SHOULD
    /// stop the worker, rebuild it with the new config, and restart capture.
    FormatChanged {
        /// The newly negotiated configuration.
        new: AudioBackendConfig,
    },
    /// The audio callback dropped one or more samples because the SPSC ring
    /// was full. The DSP worker reads this counter once per loop iteration
    /// and emits a structured `tracing::warn!` when it advances.
    Underrun {
        /// Cumulative dropped-sample count since the backend started.
        count: u64,
    },
}

/// Errors raised by [`AudioBackend::start`] and related lifecycle calls.
///
/// Variants are intentionally coarse-grained for Phase 1.1; the cpal-specific
/// branches surface as [`AudioBackendError::BuildStream`] with the underlying
/// error message preserved.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AudioBackendError {
    /// No capture device is available on the current platform.
    #[error("no audio capture device available")]
    DeviceUnavailable,

    /// The negotiated stream format is not supported by this backend.
    #[error("unsupported stream format: {0}")]
    UnsupportedFormat(String),

    /// The platform refused to build the audio stream. The wrapped string is
    /// the platform error message verbatim.
    #[error("failed to build audio stream: {0}")]
    BuildStream(String),

    /// `start()` was called twice without an intervening `stop()`. This is a
    /// state-machine violation rather than a stream-construction failure.
    #[error("audio backend already started")]
    AlreadyStarted,

    /// An OS-level I/O error occurred while interacting with the audio
    /// subsystem.
    #[error("audio I/O error: {0}")]
    Io(#[from] io::Error),
}

/// Backend-agnostic audio capture trait.
///
/// Implementations own a real-time-safe capture path. They MUST:
///
/// - Push interleaved-mono `f32` samples through the supplied
///   [`rtrb::Producer<f32>`].
/// - Drop new samples on the floor (incrementing an `AtomicU64` underrun
///   counter) when the producer is full; **never** block, allocate, or log
///   from the audio callback.
/// - Stop and free the underlying stream in [`AudioBackend::stop`] so
///   `Drop` is a no-op.
pub trait AudioBackend: Send {
    /// Begin capture and route samples through `producer`.
    ///
    /// May be called at most once per backend instance for Phase 1.1.
    /// Subsequent calls SHOULD return [`AudioBackendError::BuildStream`].
    fn start(&mut self, producer: rtrb::Producer<f32>) -> Result<(), AudioBackendError>;

    /// Stop capture and tear down the underlying stream. Idempotent.
    fn stop(&mut self);

    /// Borrow the active configuration.
    fn config(&self) -> &AudioBackendConfig;
}
