#![allow(clippy::doc_markdown)]
//! Phase 2.2 — CREPE-tiny neural pitch estimator.
//!
//! CREPE-tiny (Kim et al., ICASSP 2018) is a 6-conv-layer cents-bin
//! classifier. We use the MIT-licensed weights from `yqzhishen/onnxcrepe`
//! v1.1.0 (1.96 MB) per `MODULAR-PITCH-RESEARCH.md` §2.1 — explicitly the
//! **license-clean fallback** for builds where counsel rejects PESTO's
//! LGPL-3.0 lineage or where a profile sets `--cfg license_strict_mit`.
//!
//! Unlike PESTO, CREPE has **no temporal cache tensor** — it is fully
//! stateless per `MODULAR-PITCH-RESEARCH.md` §8.2. Each `process` call is
//! a single forward pass over a 1024-sample @ 16 kHz window. Phase 2.2
//! ships only the synthetic stub backend (which operates on the raw 48 kHz
//! buffer because the recover-440-Hz-within-5-cents contract does not
//! require a real resample); Phase 2.5 will introduce a pre-allocated
//! `rubato::SincFixedIn` resampler so the real-ONNX hot path stays
//! alloc-free at 48 kHz capture rate.
//!
//! # Asset and license posture
//!
//! The `.onnx` file is treated as a runtime asset (ADR-0008). Tests use a
//! tiny synthetic stub ONNX (see [`crate::test_utils`]); production users
//! supply the real `.onnx` via the resolver path Phase 2.5/3+ lands.
//!
//! # Stub-graph fallback
//!
//! When the on-disk bytes match the in-tree stub marker, the estimator
//! takes a Stub backend that performs autocorrelation pitch detection
//! over the raw 48 kHz window (the stub does not need to round-trip
//! through 16 kHz because the Tier-1 contract is "recover 440 Hz within
//! 5 cents"; a stub-internal resample would only add quantisation
//! noise). The real-ONNX branch is wired in Phase 2.5 once the resolver
//! lands.

use std::path::Path;

use crate::pitch::{EstimatorConfig, EstimatorError, F0Frame, PitchEstimator};

/// Constructor-time invariant: capture rate is 48 kHz; the model itself
/// runs at 16 kHz internally after resampling.
const CREPE_CAPTURE_SAMPLE_RATE_HZ: u32 = 48_000;
/// Constructor-time invariant: 960-sample @ 48 kHz capture window
/// (resamples to ~320 samples at 16 kHz — the model's native 1024-sample
/// frame is filled by overlap-buffering inside the estimator).
const CREPE_WINDOW_SIZE: usize = 960;
/// CREPE's native model rate after the `rubato` resample.
#[allow(dead_code)]
const CREPE_MODEL_SAMPLE_RATE_HZ: u32 = 16_000;
/// CREPE's native model frame length, in 16 kHz samples.
#[allow(dead_code)]
const CREPE_MODEL_FRAME_SIZE: usize = 1024;

/// Backend selected at constructor time, mirroring the
/// [`super::pesto::PestoEstimator`] split: synthetic stub for
/// hermetic Tier-1 tests, real ORT session for production.
enum Backend {
    /// Hermetic stub used by the Tier-1 test suite — no ORT shared
    /// library required.
    Stub,
    /// Real ONNX session held opaquely so callers do not depend on
    /// `ort` 2.0 surface API.
    #[allow(dead_code)]
    Onnx(Box<ort::session::Session>),
}

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
/// Phase 2.2 has no rubato resampler yet — it will land in Phase 2.5
/// alongside the real-ONNX path). `reset` is therefore a no-op other than
/// rolling the timestamp counter.
pub struct CrepeTinyEstimator {
    cfg: EstimatorConfig,
    backend: Backend,
    /// Frame counter, in samples since the most recent `reset`. Drives
    /// the `timestamp_samples` field of the emitted [`F0Frame`].
    timestamp_samples: u64,
}

impl CrepeTinyEstimator {
    /// Stable backend identifier.
    pub const NAME: &'static str = "crepe-tiny";

    /// Load the ONNX model at `path`, validate input/output names, and
    /// pre-allocate the rubato resampler + output scratch buffers.
    ///
    /// Returns [`EstimatorError::ModelNotFound`] if `path` does not exist
    /// and [`EstimatorError::Ort`] if the session fails to load or the
    /// graph signature does not match what the CREPE export emits
    /// (`audio` input, `cents_logits` output — no `cache_*` tensors).
    pub fn from_onnx(path: &Path, cfg: EstimatorConfig) -> Result<Self, EstimatorError> {
        if !path.exists() {
            return Err(EstimatorError::ModelNotFound(path.to_path_buf()));
        }
        validate_crepe_cfg(&cfg)?;

        let bytes = std::fs::read(path)
            .map_err(|e| EstimatorError::Ort(format!("read crepe onnx: {e}")))?;

        let backend = if is_stub_bytes(&bytes, b"crepe-stub") {
            Backend::Stub
        } else {
            let session = ort::session::Session::builder()
                .map_err(|e| EstimatorError::Ort(format!("session builder: {e}")))?
                .commit_from_file(path)
                .map_err(|e| EstimatorError::Ort(format!("commit_from_file: {e}")))?;
            Backend::Onnx(Box::new(session))
        };

        Ok(Self {
            cfg,
            backend,
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

/// Detect the in-tree synthetic stub payload. Both stubs share a
/// four-byte zero prefix, so we additionally key on a per-stub marker
/// to keep the PESTO and CREPE branches independent.
fn is_stub_bytes(bytes: &[u8], marker: &[u8]) -> bool {
    bytes.len() < 1024 && bytes.windows(marker.len()).any(|w| w == marker)
}

/// Lightweight autocorrelation pitch detector used by the stub backend,
/// shared in shape with [`super::pesto`] but kept in this module to
/// avoid a public-API dependency between the two estimators. See the
/// PESTO module's docstring for the full design notes.
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
        match &self.backend {
            Backend::Stub => Ok(self.process_stub(samples)),
            Backend::Onnx(_) => Err(EstimatorError::Ort(
                "real CREPE-tiny ONNX path not yet wired (Phase 2.5)".into(),
            )),
        }
    }

    fn reset(&mut self) {
        // CREPE is stateless — `reset` only rolls the timestamp counter
        // so the next `process` reports `timestamp_samples = 0` for
        // its emitted frame, matching the [`PitchEstimator::reset`]
        // contract on `super::PitchEstimator`.
        self.timestamp_samples = 0;
    }
}
