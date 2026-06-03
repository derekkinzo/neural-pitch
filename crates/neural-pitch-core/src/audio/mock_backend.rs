//! Deterministic in-process audio backend for tests.
//!
//! [`MockAudioBackend`] is the always-on counterpart to
//! [`crate::audio::CpalAudioBackend`]. It does **not** spawn a real-time
//! capture thread; tests drive it manually via [`MockAudioBackend::feed`],
//! which pushes a fixed number of `f32` samples into the SPSC producer with
//! the same back-pressure semantics as the cpal callback (drop on full,
//! incrementing an `AtomicU64` counter).
//!
//! This is a Tier-2 test fixture per ADR-0016: deterministic, single-threaded,
//! and free of any platform-specific I/O.

use core::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::audio::backend::{AudioBackend, AudioBackendConfig, AudioBackendError};

/// Sample-source descriptor for [`MockAudioBackend`].
#[non_exhaustive]
pub enum SampleSource {
    /// A pure sine wave at `hz`, peak amplitude `0.95`. Phase advances
    /// continuously across `feed` calls.
    Sine {
        /// Frequency of the generated sine wave, in Hertz.
        hz: f32,
    },
    /// Pure silence (constant zero).
    Silence,
    /// A vibrato signal: a sine whose instantaneous frequency oscillates
    /// around `center` at `rate_hz`, with peak deviation `depth_cents`.
    Vibrato {
        /// Centre frequency, in Hertz.
        center: f32,
        /// Vibrato modulation rate, in Hertz.
        rate_hz: f32,
        /// Peak frequency deviation from `center`, in cents.
        depth_cents: f32,
    },
    /// User-supplied waveform: the closure receives the absolute sample
    /// index since `start()` and returns the sample value.
    Custom(Box<dyn FnMut(u64) -> f32 + Send>),
}

impl core::fmt::Debug for SampleSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Sine { hz } => f.debug_struct("Sine").field("hz", hz).finish(),
            Self::Silence => f.debug_struct("Silence").finish(),
            Self::Vibrato {
                center,
                rate_hz,
                depth_cents,
            } => f
                .debug_struct("Vibrato")
                .field("center", center)
                .field("rate_hz", rate_hz)
                .field("depth_cents", depth_cents)
                .finish(),
            Self::Custom(_) => f.debug_struct("Custom").finish_non_exhaustive(),
        }
    }
}

/// Pacing policy for the mock backend.
///
/// `MockAudioBackend` itself does not sleep — the variants exist for symmetry
/// with future real-time test harnesses and are not consulted by the current
/// implementation. They are kept on the public surface so tests can express
/// intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pacing {
    /// Produce samples as fast as the consumer drains them. This is the only
    /// mode used by the Tier-2 deterministic tests today.
    #[default]
    AsFastAsPossible,
    /// Reserved for future use: simulate real-time pacing in long-running
    /// integration tests. **Currently a no-op marker** — `MockAudioBackend`
    /// does not consult this variant; `feed` always runs as fast as the ring
    /// drains.
    Realtime,
}

/// Always-on, deterministic audio backend driven by [`MockAudioBackend::feed`].
///
/// `MockAudioBackend` never spawns a thread. Tests construct it, hand the
/// associated [`rtrb::Producer<f32>`] to the worker, and call
/// [`MockAudioBackend::feed`] to push exactly the samples they want analysed.
pub struct MockAudioBackend {
    cfg: AudioBackendConfig,
    source: SampleSource,
    pacing: Pacing,
    producer: Option<rtrb::Producer<f32>>,
    samples_emitted: u64,
    dropped_samples: Arc<AtomicU64>,
    started: bool,
    /// Phase accumulator (radians) used by the [`SampleSource::Vibrato`] and
    /// [`SampleSource::Sine`] generators so successive `feed` calls produce
    /// continuous waveforms.
    phase: f32,
}

impl core::fmt::Debug for MockAudioBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MockAudioBackend")
            .field("cfg", &self.cfg)
            .field("source", &self.source)
            .field("pacing", &self.pacing)
            .field("samples_emitted", &self.samples_emitted)
            .field("started", &self.started)
            .finish_non_exhaustive()
    }
}

impl MockAudioBackend {
    /// Construct a new mock backend with the given configuration and source.
    ///
    /// The pacing policy defaults to [`Pacing::AsFastAsPossible`].
    pub fn new(cfg: AudioBackendConfig, source: SampleSource) -> Self {
        Self::with_pacing(cfg, source, Pacing::AsFastAsPossible)
    }

    /// Construct a new mock backend with explicit pacing.
    pub fn with_pacing(cfg: AudioBackendConfig, source: SampleSource, pacing: Pacing) -> Self {
        Self {
            cfg,
            source,
            pacing,
            producer: None,
            samples_emitted: 0,
            dropped_samples: Arc::new(AtomicU64::new(0)),
            started: false,
            phase: 0.0,
        }
    }

    /// Feed `count` samples generated from the active [`SampleSource`] into
    /// the producer. Returns the number of samples actually accepted by the
    /// ring; samples that did not fit increment the underrun counter.
    ///
    /// Returns `0` immediately if [`AudioBackend::start`] has not been
    /// called yet (the producer half is not yet owned by the backend);
    /// callers that hit this branch are misconfigured.
    pub fn feed(&mut self, count: usize) -> usize {
        let Some(producer) = self.producer.as_mut() else {
            return 0;
        };
        let sr = self.cfg.sample_rate as f32;
        let mut accepted = 0_usize;
        let mut dropped = 0_u64;
        for _ in 0..count {
            let sample = match &mut self.source {
                SampleSource::Silence => 0.0_f32,
                SampleSource::Sine { hz } => {
                    let s = self.phase.sin() * 0.95;
                    self.phase += TAU * *hz / sr;
                    if self.phase > TAU {
                        self.phase -= TAU;
                    }
                    s
                }
                SampleSource::Vibrato {
                    center,
                    rate_hz,
                    depth_cents,
                } => {
                    let t = self.samples_emitted as f32 / sr;
                    let log2_ratio = *depth_cents / 1200.0;
                    let mod_octaves = log2_ratio * (TAU * *rate_hz * t).sin();
                    let f_inst = *center * mod_octaves.exp2();
                    let s = self.phase.sin() * 0.95;
                    self.phase += TAU * f_inst / sr;
                    if self.phase > TAU {
                        self.phase -= TAU;
                    }
                    s
                }
                SampleSource::Custom(f) => f(self.samples_emitted),
            };
            self.samples_emitted = self.samples_emitted.saturating_add(1);
            if producer.push(sample).is_ok() {
                accepted += 1;
            } else {
                dropped = dropped.saturating_add(1);
            }
        }
        if dropped > 0 {
            self.dropped_samples.fetch_add(dropped, Ordering::Relaxed);
        }
        accepted
    }

    /// Borrow a clone-able handle to the underrun counter.
    ///
    /// The counter is incremented inside [`MockAudioBackend::feed`] (and
    /// inside the cpal callback in production) every time a sample is
    /// dropped because the SPSC ring was full. Phase 1.1 exposes the
    /// counter as a poll-only handle: callers (tests and the future
    /// Phase 1.2 Tauri shell) read it as needed. The DSP worker does
    /// **not** currently poll it — wiring of the structured underrun
    /// warning is Phase 1.2 work.
    pub fn dropped_samples(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.dropped_samples)
    }

    /// Total samples generated by [`MockAudioBackend::feed`] since
    /// [`AudioBackend::start`]. Includes samples that were dropped on the
    /// floor because the ring was full.
    pub fn samples_emitted(&self) -> u64 {
        self.samples_emitted
    }

    /// Active pacing policy.
    pub fn pacing(&self) -> Pacing {
        self.pacing
    }
}

impl AudioBackend for MockAudioBackend {
    fn start(&mut self, producer: rtrb::Producer<f32>) -> Result<(), AudioBackendError> {
        if self.started {
            return Err(AudioBackendError::AlreadyStarted);
        }
        self.producer = Some(producer);
        self.started = true;
        Ok(())
    }

    fn stop(&mut self) {
        self.producer = None;
        self.started = false;
    }

    fn config(&self) -> &AudioBackendConfig {
        &self.cfg
    }
}
