//! YIN/MPM pitch estimator — Phase 0 skeleton.
//!
//! This file is intentionally a stub: the [`PitchEstimator::process`] impl
//! returns [`EstimatorError::FeatureDisabled`] on every call. Phase 1 will
//! replace the stub with a real YIN/MPM implementation, at which point the
//! `#[ignore]`'d smoke tests in `tests/yin_smoke.rs` will be un-ignored.

use crate::pitch::{EstimatorConfig, EstimatorError, F0Frame, PitchEstimator};

/// Skeleton YIN/MPM estimator.
///
/// Holds an [`EstimatorConfig`] and nothing else. Phase 1 will add internal
/// difference-function buffers, parabolic interpolation state, and the MPM
/// peak-picking history that lets YIN reach the required ±5 cent accuracy on
/// clean signals.
#[derive(Debug)]
pub struct YinMpmEstimator {
    cfg: EstimatorConfig,
}

impl YinMpmEstimator {
    /// Construct a new estimator from an [`EstimatorConfig`].
    ///
    /// Phase 0 performs no validation; Phase 1 will reject configurations
    /// where `window_size < 2 * sample_rate / fmin_hz` (the classical YIN
    /// minimum-period requirement).
    pub fn new(cfg: EstimatorConfig) -> Result<Self, EstimatorError> {
        tracing::trace!(
            target: "neural_pitch_core::pitch::yin",
            window_size = cfg.window_size,
            hop_size = cfg.hop_size,
            sample_rate_hz = cfg.sample_rate_hz,
            "YinMpmEstimator::new (Phase 0 skeleton)"
        );
        Ok(Self { cfg })
    }
}

impl PitchEstimator for YinMpmEstimator {
    fn name(&self) -> &'static str {
        "yin-mpm"
    }

    fn config(&self) -> &EstimatorConfig {
        &self.cfg
    }

    fn process(&mut self, _samples: &[f32]) -> Result<Option<F0Frame>, EstimatorError> {
        Err(EstimatorError::FeatureDisabled("yin not yet implemented"))
    }

    fn reset(&mut self) {
        // No internal state to clear in the Phase 0 skeleton.
    }
}
