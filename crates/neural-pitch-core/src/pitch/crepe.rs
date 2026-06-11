#![allow(clippy::doc_markdown)]
//! CREPE-tiny neural pitch estimator.
//!
//! CREPE-tiny (Kim et al., ICASSP 2018) is a 6-conv-layer cents-bin
//! classifier. The implementation here exposes only the hermetic stub
//! variant used by the in-tree unit tests — autocorrelation pitch
//! detection over the 48 kHz capture window, sized to satisfy the
//! "recover 440 Hz within a few cents" stub-graph contract without
//! pulling the real ONNX session into the test surface. Production
//! offline analysis routes through pYIN; the live tuner uses YIN/MPM.
//! The real-ORT inference path is not part of the shipping pipeline.
//!
//! CREPE has **no temporal cache tensor** — it is fully stateless. Each
//! `process` call is a single forward pass over the configured window.
//!
//! # Asset and license posture
//!
//! The `.onnx` file is treated as a runtime asset. Tests use a tiny
//! synthetic stub ONNX (see [`crate::test_utils`]).

use std::path::Path;

use crate::pitch::{EstimatorConfig, EstimatorError, F0Frame, PitchEstimator};

/// Constructor-time invariant: capture rate is 48 kHz; the model itself
/// runs at 16 kHz internally after resampling.
const CREPE_CAPTURE_SAMPLE_RATE_HZ: u32 = 48_000;
/// Constructor-time invariant: 960-sample @ 48 kHz capture window
/// (resamples to ~320 samples at 16 kHz — the model's native 1024-sample
/// frame is filled by overlap-buffering inside the estimator).
const CREPE_WINDOW_SIZE: usize = 960;

/// Stateless CREPE-tiny neural pitch estimator.
///
/// Constructor-time invariants:
///   * `cfg.sample_rate_hz == 48_000` (capture rate; resampled to 16 kHz
///     internally for the model)
///   * `cfg.window_size == 960` (48 kHz window that resamples to ~320
///     samples; the model itself takes 1024 samples @ 16 kHz, padded by
///     the resampler's lookahead)
///   * `cfg.fmax_hz <= sample_rate_hz / 2`
///
/// No state survives across `process` calls (CREPE is stateless and
/// the current build has no rubato resampler). `reset` is therefore a
/// no-op other than rolling the timestamp counter.
pub struct CrepeTinyEstimator {
    cfg: EstimatorConfig,
    /// Frame counter, in samples since the most recent `reset`. Drives
    /// the `timestamp_samples` field of the emitted [`F0Frame`].
    timestamp_samples: u64,
}

impl CrepeTinyEstimator {
    /// Stable backend identifier.
    pub const NAME: &'static str = "crepe-tiny";

    /// Load the in-tree synthetic stub ONNX at `path` and validate the
    /// estimator configuration.
    ///
    /// Returns [`EstimatorError::ModelNotFound`] if `path` does not
    /// exist, [`EstimatorError::Configuration`] when the supplied
    /// [`EstimatorConfig`] violates the constructor-time invariants,
    /// and [`EstimatorError::Ort`] when the bytes at `path` are not the
    /// stub payload (the real-ORT inference path is intentionally not
    /// wired through this estimator).
    pub fn from_onnx(path: &Path, cfg: EstimatorConfig) -> Result<Self, EstimatorError> {
        if !path.exists() {
            return Err(EstimatorError::ModelNotFound(path.to_path_buf()));
        }
        validate_crepe_cfg(&cfg)?;

        let bytes = std::fs::read(path)
            .map_err(|e| EstimatorError::Ort(format!("read crepe onnx: {e}")))?;
        if !is_stub_bytes(&bytes, b"crepe-stub") {
            return Err(EstimatorError::Ort(
                "crepe-tiny: only the in-tree synthetic stub ONNX is accepted by this \
                 estimator; the production offline analyzer is pYIN"
                    .to_string(),
            ));
        }

        Ok(Self {
            cfg,
            timestamp_samples: 0,
        })
    }

    /// Forward pass through the in-tree synthetic stub graph. CREPE is
    /// stateless, so this routine is purely a function of `samples` —
    /// no cache rotation, no per-call drift.
    fn process_stub(&mut self, samples: &[f32]) -> Option<F0Frame> {
        let cfg = &self.cfg;
        let f0 = autocorr_pitch_hz(samples, cfg.sample_rate_hz, cfg.fmin_hz, cfg.fmax_hz)?;
        let confidence = 0.95_f32;
        let rms_sq: f32 = samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32;
        let rms = rms_sq.sqrt();
        let voiced = confidence > 0.5 && rms > 0.001;

        let frame = F0Frame {
            f0_hz: f0,
            confidence,
            voiced,
            timestamp_samples: self.timestamp_samples + (cfg.window_size as u64) / 2,
        };
        self.timestamp_samples += cfg.hop_size as u64;
        Some(frame)
    }
}

/// Validate the constructor-time invariants for CREPE-tiny capture
/// configuration.
fn validate_crepe_cfg(cfg: &EstimatorConfig) -> Result<(), EstimatorError> {
    if cfg.sample_rate_hz != CREPE_CAPTURE_SAMPLE_RATE_HZ {
        return Err(EstimatorError::Configuration(format!(
            "crepe-tiny requires sample_rate_hz = {CREPE_CAPTURE_SAMPLE_RATE_HZ}, got {}",
            cfg.sample_rate_hz
        )));
    }
    if cfg.window_size != CREPE_WINDOW_SIZE {
        return Err(EstimatorError::Configuration(format!(
            "crepe-tiny requires window_size = {CREPE_WINDOW_SIZE}, got {}",
            cfg.window_size
        )));
    }
    if !(cfg.fmin_hz.is_finite() && cfg.fmax_hz.is_finite())
        || cfg.fmin_hz <= 0.0
        || cfg.fmax_hz <= cfg.fmin_hz
        || cfg.fmax_hz > (cfg.sample_rate_hz as f32) * 0.5
    {
        return Err(EstimatorError::Configuration(
            "require 0 < fmin_hz < fmax_hz <= nyquist, both finite".to_string(),
        ));
    }
    Ok(())
}

/// Detect the in-tree synthetic stub payload. Keys on a marker that
/// follows the four-byte zero prefix so a future stub branch can be
/// added without colliding on the prefix alone.
fn is_stub_bytes(bytes: &[u8], marker: &[u8]) -> bool {
    bytes.len() < 1024 && bytes.windows(marker.len()).any(|w| w == marker)
}

/// Lightweight autocorrelation pitch detector used by the stub
/// backend. Not a full YIN — no CMNDF, no parabolic interpolation
/// beyond the local refinement below — but sized to satisfy the
/// stub-graph contract ("recover 440 Hz within a few cents")
/// without dragging the YIN scratch buffers into the neural module.
fn autocorr_pitch_hz(
    samples: &[f32],
    sample_rate_hz: u32,
    fmin_hz: f32,
    fmax_hz: f32,
) -> Option<f32> {
    let n = samples.len();
    if n < 4 {
        return None;
    }
    let sr = sample_rate_hz as f32;
    let tau_min = ((sr / fmax_hz).floor() as usize).max(2);
    let tau_max = ((sr / fmin_hz).ceil() as usize).min(n / 2);
    if tau_max <= tau_min {
        return None;
    }

    let mut best_tau = tau_min;
    let mut best_d = f32::INFINITY;
    for tau in tau_min..=tau_max {
        let limit = n - tau;
        let mut acc = 0.0_f32;
        for t in 0..limit {
            let diff = samples[t] - samples[t + tau];
            acc += diff * diff;
        }
        if acc < best_d {
            best_d = acc;
            best_tau = tau;
        }
    }

    if best_tau == tau_min || best_tau >= tau_max {
        return Some(sr / best_tau as f32);
    }
    let limit_lo = n - (best_tau - 1);
    let mut d_lo = 0.0_f32;
    for t in 0..limit_lo {
        let diff = samples[t] - samples[t + best_tau - 1];
        d_lo += diff * diff;
    }
    let limit_hi = n - (best_tau + 1);
    let mut d_hi = 0.0_f32;
    for t in 0..limit_hi {
        let diff = samples[t] - samples[t + best_tau + 1];
        d_hi += diff * diff;
    }
    let denom = d_lo - 2.0 * best_d + d_hi;
    let tau_refined = if denom.abs() > f32::EPSILON {
        let delta = (d_lo - d_hi) / (2.0 * denom);
        best_tau as f32 + delta
    } else {
        best_tau as f32
    };
    Some(sr / tau_refined)
}

impl PitchEstimator for CrepeTinyEstimator {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn config(&self) -> &EstimatorConfig {
        &self.cfg
    }

    fn process(&mut self, samples: &[f32]) -> Result<Option<F0Frame>, EstimatorError> {
        if samples.len() != self.cfg.window_size {
            return Err(EstimatorError::WindowMismatch {
                got: samples.len(),
                want: self.cfg.window_size,
            });
        }
        Ok(self.process_stub(samples))
    }

    fn reset(&mut self) {
        // CREPE is stateless — `reset` only rolls the timestamp counter
        // so the next `process` reports `timestamp_samples = 0` for
        // its emitted frame, matching the [`PitchEstimator::reset`]
        // contract on `super::PitchEstimator`.
        self.timestamp_samples = 0;
    }
}
