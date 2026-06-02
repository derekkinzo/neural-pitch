#![allow(clippy::doc_markdown)]
//! Backend factory for [`PitchEstimator`] instances.
//!
//! Day 1 ships the classical time-domain backends `Backend::YinMpm` (YIN with
//! parabolic interpolation) and `Backend::Mpm` (McLeod Pitch Method, sharing
//! the same [`yin::YinMpmEstimator`] struct via [`yin::YinAlgorithm`]).
//!
//! Future variants (`PYin`, `OnnxPesto`, `OnnxCrepeTiny`) will be added behind
//! the `neural` and `pyin` Cargo features in Phase 2.

use std::path::Path;

use crate::pitch::{
    EstimatorConfig, EstimatorError, PitchEstimator,
    yin::{self, YinMpmEstimator},
};

/// Stable identifier for the available pitch detection backends.
///
/// Day 1 contains `YinMpm` and `Mpm`, both implemented by
/// [`YinMpmEstimator`] under different [`yin::YinAlgorithm`] choices. The
/// enum is non-exhaustive in spirit; new variants will be added as Phase 2
/// brings up neural backends behind the `neural` feature gate. Callers MUST
/// handle the case where a feature is not enabled, which surfaces as
/// [`EstimatorError::FeatureDisabled`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Classical YIN time-domain estimator (de Cheveigne & Kawahara 2002).
    /// Always available.
    YinMpm,
    /// McLeod Pitch Method (McLeod & Wyvill 2005). Always available.
    Mpm,
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
        Backend::YinMpm => Ok(Box::new(YinMpmEstimator::with_algorithm(
            cfg,
            yin::YinAlgorithm::Yin,
        )?)),
        Backend::Mpm => Ok(Box::new(YinMpmEstimator::with_algorithm(
            cfg,
            yin::YinAlgorithm::Mpm,
        )?)),
    }
}
