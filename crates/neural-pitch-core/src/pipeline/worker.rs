//! [`DspWorker`]: the std::thread-hosted analysis loop.
//!
//! The worker drains exactly `hop` samples from a [`rtrb::Consumer<f32>`] per
//! iteration, slides them into a window-sized scratch buffer, runs the
//! configured [`PitchEstimator`] (with [`VoiceActivityGate`] +
//! [`ContourSmoother`]), and pushes a [`PitchUpdate`] through the supplied
//! [`FrameSink`]. No allocation occurs after `new()`.
//!
//! The cancellation contract (ADR-0014): on each iteration the worker
//! checks the [`CancellationToken`] *before* doing any work. Cancellation
//! is the only ordered shutdown path; tests MUST `cancel.cancel()` and then
//! `join()` to confirm the worker exits within one packet boundary
//! (~`hop` samples of wall time).

use std::thread::JoinHandle;
use std::time::Duration;

use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::audio::backend::AudioBackendConfig;
use crate::music::frequency_to_note;
use crate::pipeline::sink::{FrameSink, FrameSinkError, PitchUpdate};
use crate::pitch::{EstimatorError, PitchEstimator};
use crate::smoothing::ContourSmoother;
use crate::voicing::VoiceActivityGate;

/// Errors raised by [`DspWorker::run`] and the convenience
/// [`DspWorker::spawn`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DspError {
    /// The active pitch estimator returned an error.
    #[error(transparent)]
    Estimator(#[from] EstimatorError),

    /// The [`FrameSink`] reported a delivery failure (typically
    /// [`FrameSinkError::Disconnected`]).
    #[error(transparent)]
    Sink(#[from] FrameSinkError),

    /// The worker thread could not be spawned.
    #[error("failed to spawn DSP worker thread: {0}")]
    Spawn(String),
}

/// Reference pitch (Hz) for note math. Tunable per-instance via
/// [`DspWorker::with_a4`].
const DEFAULT_A4_HZ: f32 = 440.0;

/// Audio analysis worker.
///
/// Owns its inputs (consumer + estimator + smoother + VAD) and its output
/// (the [`FrameSink`]). The loop is RT-safe-adjacent: no allocation after
/// construction, no locking, no syscalls beyond an optional brief
/// `thread::sleep` when the ring is starved.
pub struct DspWorker {
    estimator: Box<dyn PitchEstimator>,
    smoother: ContourSmoother,
    vad: VoiceActivityGate,
    consumer: rtrb::Consumer<f32>,
    sink: Box<dyn FrameSink>,
    cancel: CancellationToken,
    cfg: AudioBackendConfig,
    /// Pre-allocated sliding window buffer; reused every iteration.
    window: Box<[f32]>,
    /// Number of valid samples currently held in `window`. Once this reaches
    /// `cfg.window`, the buffer is full and analysis can run; subsequent
    /// hops shift the buffer left by `hop` samples and append a fresh hop
    /// from the consumer.
    window_filled: usize,
    /// Total samples drained from the consumer since `new()`.
    samples_seen: u64,
    /// Reference pitch for cents math, default `440.0`.
    a4_hz: f32,
}

impl core::fmt::Debug for DspWorker {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DspWorker")
            .field("cfg", &self.cfg)
            .field("samples_seen", &self.samples_seen)
            .field("window_filled", &self.window_filled)
            .field("a4_hz", &self.a4_hz)
            .finish_non_exhaustive()
    }
}

impl DspWorker {
    /// Construct a new worker. Pure constructor — performs no I/O.
    ///
    /// `cancel` is shared with the caller; calling `cancel.cancel()` from
    /// any thread drives the next loop iteration to exit cleanly.
    pub fn new(
        cfg: AudioBackendConfig,
        estimator: Box<dyn PitchEstimator>,
        smoother: ContourSmoother,
        vad: VoiceActivityGate,
        consumer: rtrb::Consumer<f32>,
        sink: Box<dyn FrameSink>,
        cancel: CancellationToken,
    ) -> Self {
        let window = vec![0.0_f32; cfg.window].into_boxed_slice();
        Self {
            estimator,
            smoother,
            vad,
            consumer,
            sink,
            cancel,
            cfg,
            window,
            window_filled: 0,
            samples_seen: 0,
            a4_hz: DEFAULT_A4_HZ,
        }
    }

    /// Override the reference A4 frequency used to compute
    /// [`PitchUpdate::smoothed_cents`] and [`PitchUpdate::target_hz`].
    /// Defaults to `440.0` Hz.
    #[must_use]
    pub fn with_a4(mut self, a4_hz: f32) -> Self {
        if a4_hz.is_finite() && a4_hz > 0.0 {
            self.a4_hz = a4_hz;
        }
        self
    }

    /// Number of samples drained from the consumer since `new()`.
    pub fn samples_seen(&self) -> u64 {
        self.samples_seen
    }

    /// Run the analysis loop until [`CancellationToken::cancel`] is invoked
    /// or the [`FrameSink`] disconnects.
    pub fn run(mut self) -> Result<(), DspError> {
        // Clamp `hop` to `[1, window]`. A hop greater than the window
        // would underflow `window - hop` and panic on `copy_within`, so
        // refuse to start the analysis loop. (The DSP geometry contract
        // is `hop <= window`.)
        if self.cfg.hop == 0 || self.cfg.window == 0 {
            return Err(DspError::Spawn(format!(
                "invalid AudioBackendConfig: hop={} window={} (both must be > 0)",
                self.cfg.hop, self.cfg.window
            )));
        }
        if self.cfg.hop > self.cfg.window {
            return Err(DspError::Spawn(format!(
                "invalid AudioBackendConfig: hop ({}) exceeds window ({})",
                self.cfg.hop, self.cfg.window
            )));
        }
        let hop = self.cfg.hop;
        let window = self.cfg.window;
        debug_assert!(hop <= window, "hop must be <= window");
        let park = Duration::from_micros(
            // Sleep for hop / sample_rate / 4 seconds when the ring is
            // starved. For 48 kHz / 512-sample hop that is ~2.6 ms.
            ((hop as u64) * 1_000_000 / (4 * u64::from(self.cfg.sample_rate.max(1)))).max(1),
        );

        loop {
            // 1) Cancellation contract: first instruction every iteration.
            if self.cancel.is_cancelled() {
                return Ok(());
            }

            // 2) Park if the ring does not have a full hop available.
            if self.consumer.slots() < hop {
                std::thread::sleep(park);
                // Re-check cancellation immediately after the sleep so the
                // worst-case shutdown latency on a starved ring is bounded
                // by `park` rather than `2 * park`.
                if self.cancel.is_cancelled() {
                    return Ok(());
                }
                continue;
            }

            // 3) Drain exactly `hop` samples and slide the window left by
            //    `hop`. The first `window/hop` iterations only fill the
            //    buffer without emitting.
            if self.window_filled < window {
                // Append up to `hop` samples to the back of the partial
                // buffer (clamped so we never overrun the window).
                let target = (self.window_filled + hop).min(window);
                let to_pop = target - self.window_filled;
                let mut popped = 0_usize;
                for slot in &mut self.window[self.window_filled..target] {
                    match self.consumer.pop() {
                        Ok(s) => {
                            *slot = s;
                            popped += 1;
                        }
                        Err(_) => break,
                    }
                }
                self.window_filled += popped;
                self.samples_seen = self.samples_seen.saturating_add(popped as u64);
                if popped < to_pop {
                    // Did not drain the full slice. The gating
                    // `slots() < hop` check normally prevents this, but
                    // rtrb's relaxed counters make a benign false-positive
                    // theoretically possible. Try again next iteration.
                    continue;
                }
                if self.window_filled < window {
                    continue;
                }
            } else {
                // Slide left by `hop`, then append a fresh hop at the tail.
                self.window.copy_within(hop..window, 0);
                let tail_start = window - hop;
                let mut popped = 0_usize;
                for slot in &mut self.window[tail_start..window] {
                    match self.consumer.pop() {
                        Ok(s) => {
                            *slot = s;
                            popped += 1;
                        }
                        Err(_) => break,
                    }
                }
                self.samples_seen = self.samples_seen.saturating_add(popped as u64);
                if popped < hop {
                    // Partial pop in the slide branch leaves stale data in
                    // the tail; do not run the estimator on a corrupt frame.
                    continue;
                }
            }

            // 4) Run the estimator on the full window.
            let Some(frame) = self.estimator.process(&self.window)? else {
                continue;
            };

            // 5) Apply caller-side VAD over the freshest hop. The hop
            //    represents the most recent slice of audio.
            let hop_slice = &self.window[window - hop..window];
            let vad_voiced = self.vad.is_voiced(hop_slice);
            let voiced = frame.voiced && vad_voiced;

            // 6) Smooth.
            let smoothed = self
                .smoother
                .push(crate::pitch::F0Frame { voiced, ..frame });

            // 7) Compute musical cents/MIDI/target. When unvoiced or f0 is
            //    not finite/positive, fall back to a zero-cents zero-MIDI
            //    reading; consumers must gate on `voiced` anyway.
            let f0 = smoothed.f0_hz;
            let (cents, target_midi, target_hz) = if smoothed.voiced && f0.is_finite() && f0 > 0.0 {
                let r = frequency_to_note(f0, self.a4_hz);
                (r.cents, r.midi, r.expected_hz)
            } else {
                (0.0, 0, 0.0)
            };

            let update = PitchUpdate {
                timestamp_samples: smoothed.timestamp_samples,
                f0_hz: f0,
                confidence: smoothed.confidence,
                voiced: smoothed.voiced,
                smoothed_cents: cents,
                target_midi,
                target_hz,
            };

            // 8) Deliver. Disconnected sink is a terminal condition.
            self.sink.send(update)?;
        }
    }

    /// Convenience wrapper around [`DspWorker::run`] that spawns it on a
    /// named [`std::thread`] (ADR-0014).
    pub fn spawn(self) -> Result<JoinHandle<Result<(), DspError>>, DspError> {
        std::thread::Builder::new()
            .name("neuralpitch-dsp".to_string())
            .spawn(move || self.run())
            .map_err(|e| DspError::Spawn(e.to_string()))
    }
}
