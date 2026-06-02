//! Backend factory for [`PitchEstimator`] instances.
//!
//! Day 1 only ships the YIN/MPM backend (`Backend::YinMpm`). Future variants
//! (`PYin`, `OnnxPesto`, `OnnxCrepeTiny`) will be added behind the `neural`
//! and `pyin` Cargo features in Phase 2.

use std::path::Path;

use crate::pitch::{EstimatorConfig, EstimatorError, PitchEstimator, yin::YinMpmEstimator};

/// Stable identifier for the available pitch detection backends.
///
/// Day 1 only contains `YinMpm`. The enum is non-exhaustive in spirit; new
/// variants will be added as Phase 2 brings up neural backends behind the
/// `neural` feature gate. Callers MUST handle the case where a feature is
/// not enabled, which surfaces as [`EstimatorError::FeatureDisabled`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Classical YIN/MPM time-domain estimator. Always available.
    YinMpm,
}

/// Construct a boxed estimator for the requested backend.
///
/// `model_root` is the resolved directory containing ONNX weights. `None` is
/// the correct value for classical backends (YIN/MPM, pYIN). Neural backends
/// require `Some(...)` and return [`EstimatorError::ModelNotFound`] if the
/// path is missing the requested weights.
pub fn make_estimator(
    backend: Backend,
    cfg: EstimatorConfig,
    _model_root: Option<&Path>,
) -> Result<Box<dyn PitchEstimator>, EstimatorError> {
    match backend {
        Backend::YinMpm => Ok(Box::new(YinMpmEstimator::new(cfg)?)),
    }
}
