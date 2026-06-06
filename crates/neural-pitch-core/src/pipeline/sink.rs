//! Backend-agnostic frame-sink trait and the [`PitchUpdate`] data shape.
//!
//! The [`FrameSink`] trait keeps `tauri::*` out of `neural-pitch-core` (P2,
//! the day-1 implementation [`ChannelFrameSink`] wraps a
//! `std::sync::mpsc::Sender<PitchUpdate>`, which every Tier-2 test uses. The
//! Tauri-side `TauriChannelFrameSink` lives in `src-tauri/src/ipc/` (Phase
//! 1.2 work) and adapts `tauri::ipc::Channel<PitchUpdate>` against this same
//! trait.

use std::sync::mpsc::{SendError, Sender};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One pitch-detection update emitted by [`crate::pipeline::DspWorker`].
///
/// `smoothed_cents`, `target_midi`, and `target_hz` are computed by the
/// worker after the [`crate::smoothing::ContourSmoother`]. When `voiced` is
/// `false`, `f0_hz` and `smoothed_cents` carry the most recent valid values;
/// consumers MUST gate on `voiced` before treating them as meaningful.
///
/// `Copy` so [`FrameSink::send`] is a pointer-free pass-by-register on the
/// IPC hot path — no heap traffic per analysis frame.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct PitchUpdate {
    /// Absolute timestamp of the analysis frame's centre, in samples since
    /// the worker started.
    pub timestamp_samples: u64,
    /// Estimated fundamental frequency, in Hertz.
    pub f0_hz: f32,
    /// Estimator confidence, normalised to `[0.0, 1.0]`.
    pub confidence: f32,
    /// `true` if both the estimator's internal voicing decision and the
    /// caller-side [`crate::voicing::VoiceActivityGate`] reported voiced.
    pub voiced: bool,
    /// Signed deviation in cents from the nearest equal-tempered note at
    /// the configured `a4_hz`. Range `(-50.0, 50.0]`.
    pub smoothed_cents: f32,
    /// MIDI number of the nearest equal-tempered note.
    pub target_midi: i32,
    /// Equal-tempered Hertz value of `target_midi` at the configured
    /// `a4_hz`.
    pub target_hz: f32,
}

/// Errors raised by [`FrameSink::send`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FrameSinkError {
    /// The downstream consumer dropped the receiver.
    #[error("frame sink disconnected")]
    Disconnected,
}

impl<T> From<SendError<T>> for FrameSinkError {
    fn from(_: SendError<T>) -> Self {
        Self::Disconnected
    }
}

/// Backend-agnostic delivery surface for [`PitchUpdate`] frames.
///
/// Implementations MUST be cheap to call from the DSP worker thread —
/// the worker calls `send` once per analysis frame on the hot path.
pub trait FrameSink: Send {
    /// Deliver one update. Returns [`FrameSinkError::Disconnected`] when
    /// the downstream receiver has gone away; the worker treats that as a
    /// terminal condition and exits cleanly.
    fn send(&mut self, update: PitchUpdate) -> Result<(), FrameSinkError>;
}

/// Day-1 [`FrameSink`] implementation that wraps a
/// `std::sync::mpsc::Sender<PitchUpdate>`.
///
/// Used by every Tier-2 deterministic test. The Tauri-side adapter
/// (`TauriChannelFrameSink`) lives in the shell crate (Phase 1.2) and is
/// out of scope for this crate.
#[derive(Debug)]
pub struct ChannelFrameSink {
    tx: Sender<PitchUpdate>,
}

impl ChannelFrameSink {
    /// Wrap an existing `Sender<PitchUpdate>`.
    pub fn new(tx: Sender<PitchUpdate>) -> Self {
        Self { tx }
    }
}

impl FrameSink for ChannelFrameSink {
    fn send(&mut self, update: PitchUpdate) -> Result<(), FrameSinkError> {
        self.tx.send(update).map_err(FrameSinkError::from)
    }
}
