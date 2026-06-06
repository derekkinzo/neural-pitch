//! Backend-agnostic audio capture trait surface.
//!
//! [`AudioBackend`] is the single boundary between live capture (cpal in
//! production, [`crate::audio::MockAudioBackend`] in tests) and the DSP
//! worker. Concrete backends push interleaved-mono `f32` samples into an
//! [`rtrb::Producer<f32>`]; the worker drains them on a dedicated `std::thread`
//! (ADR-0014).
//!
//! Out-of-band device events ([`AudioBackendEvent`]) flow through an
//! [`AudioEventEmitter`] supplied by the Tauri shell at construction time.
//! Phase 1.3 wires the emitter as a `Arc<dyn Fn(AudioBackendEvent) + Send + Sync>`
//! so the core crate keeps zero `tauri::*` imports (P2). The shell wraps a
//! `tauri::ipc::Channel<AudioBackendEvent>` in a matching closure. Events
//! MUST NOT be funnelled through the sample producer (P3, DESIGN §2.4); the
//! producer is real-time-safe and must not carry structured data.

use std::io;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Audio capture configuration negotiated between the backend and the DSP
/// worker.
///
/// `AudioBackendConfig` is the single source of truth for the analyzer's
/// expected geometry: sample rate, channel count, hop size, and analysis
/// window length. The DSP worker uses these to size its sliding-window buffer
/// and to compute the ring-buffer capacity (`next_pow2(3 * window)` per
/// DESIGN §6.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
        // `checked_next_power_of_two` returns `None` only on overflow at the
        // very top of the `usize` range, which we never approach in practice.
        // Fall back to the largest representable power of two to keep the
        // function total without unwrap.
        self.window
            .saturating_mul(3)
            .max(1)
            .checked_next_power_of_two()
            .unwrap_or(usize::MAX / 2 + 1)
    }
}

/// Out-of-band events emitted by an audio backend.
///
/// These flow through an [`AudioEventEmitter`] supplied by the Tauri shell at
/// backend construction time. The shell wraps a
/// `tauri::ipc::Channel<AudioBackendEvent>` in a matching closure so the core
/// crate keeps zero `tauri::*` imports (P2, ADR-0002). Events are intentionally
/// out-of-band: they MUST NOT be funnelled through the sample producer, which
/// is real-time-safe and must not carry structured data.
///
/// The variants are tagged with `#[serde(tag = "kind")]` so the front-end can
/// branch on a stable string discriminator instead of regex-matching free-form
/// strings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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

/// Type alias for the out-of-band event-emitter closure passed to backends.
///
/// The Tauri shell wraps a `tauri::ipc::Channel<AudioBackendEvent>` in a
/// closure satisfying this signature; the core crate stays free of any
/// `tauri::*` imports (P2). Calls to the emitter are non-blocking: the cpal
/// `err_fn` runs on the platform audio thread, and the closure MUST NOT
/// allocate or block per ADR-0014. `tauri::ipc::Channel::send` does perform
/// JSON serialisation on the calling thread, but only on the rare device-
/// event path (not the per-sample hot path), so the cost is acceptable.
pub type AudioEventEmitter = Arc<dyn Fn(AudioBackendEvent) + Send + Sync>;

/// Errors raised by [`AudioBackend::start`] and related lifecycle calls.
///
/// Variants are intentionally coarse-grained for Phase 1.1; most cpal-specific
/// branches surface as [`AudioBackendError::BuildStream`] with the underlying
/// error message preserved.
///
/// [`AudioBackendError::PermissionDenied`] is the dedicated typed channel for
/// the macOS TCC microphone-denial case (and any future platform-equivalent
/// permission rejection): consumers branch on the *variant*, not on a
/// stringified message, so that locale-dependent backend-specific text
/// changes cannot regress the user-facing recovery flow. ADR-0017 requires
/// no telemetry on this path.
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

    /// The OS denied the microphone capture permission (macOS TCC denial,
    /// Windows mic-privacy off, Linux PipeWire / Pulse policy block). The
    /// wrapped string preserves the platform-specific message for logs;
    /// consumers should pattern-match on the variant for the user-facing
    /// recovery flow rather than the message body.
    #[error("microphone permission denied: {0}")]
    PermissionDenied(String),

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
