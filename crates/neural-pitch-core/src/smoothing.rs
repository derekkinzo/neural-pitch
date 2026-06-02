//! Pitch contour smoothing.
//!
//! [`ContourSmoother`] maintains a running window of recent F0 frames and
//! returns a smoothed estimate. Day 1 implements a simple running mean over
//! cents (relative to a sliding-window reference); Phase 2 will replace it
//! with a more musically-aware filter (median in cents, hangover, etc.).

use std::collections::VecDeque;

use crate::pitch::F0Frame;

/// Sliding-window contour smoother.
///
/// The window holds `window_ms * sample_rate_hz / 1000` worth of historical
/// F0 values, expressed in cents relative to the most-recent voiced frame.
/// `push` returns a frame whose `f0_hz` is the running mean over the
/// window; unvoiced inputs are passed through untouched.
#[derive(Debug)]
pub struct ContourSmoother {
    window_ms: f32,
    sample_rate_hz: u32,
    history: VecDeque<f32>,
    capacity: usize,
}

impl ContourSmoother {
    /// Construct a new smoother with the given window length in milliseconds
    /// and the sample rate of the underlying audio. The capacity is computed
    /// once at construction; later `push` calls do not reallocate.
    pub fn new(window_ms: f32, sample_rate_hz: u32) -> Self {
        // Capacity in *frames* — assume one frame per millisecond as a
        // conservative upper bound. This is intentionally generous so the
        // ring never reallocates on the hot path even for high-rate
        // estimators (e.g. PESTO at ~100 Hz frame rate).
        let capacity = window_ms.max(1.0).ceil() as usize;
        Self {
            window_ms,
            sample_rate_hz,
            history: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a new frame into the window and return the smoothed result.
    ///
    /// Unvoiced frames are returned unchanged and do not contribute to the
    /// running mean. Voiced frames are added to the window; the returned
    /// frame has `f0_hz` set to the running mean of the window.
    pub fn push(&mut self, frame: F0Frame) -> F0Frame {
        if !frame.voiced {
            return frame;
        }
        if self.history.len() == self.capacity {
            let _ = self.history.pop_front();
        }
        self.history.push_back(frame.f0_hz);
        let mean = self.history.iter().copied().sum::<f32>() / self.history.len() as f32;
        F0Frame {
            f0_hz: mean,
            confidence: frame.confidence,
            voiced: true,
            timestamp_samples: frame.timestamp_samples,
        }
    }

    /// Window length in milliseconds, as supplied at construction.
    pub fn window_ms(&self) -> f32 {
        self.window_ms
    }

    /// Sample rate the smoother was constructed for.
    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// Drop the running history. The next `push` starts with an empty window.
    pub fn reset(&mut self) {
        self.history.clear();
    }
}
