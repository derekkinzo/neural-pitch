#![allow(clippy::doc_markdown)]
//! Backend factory for [`PitchEstimator`] instances.
//!
//! Exposes the classical time-domain backends `Backend::YinMpm` (YIN
//! with parabolic interpolation) and `Backend::Mpm` (McLeod Pitch
//! Method, sharing the same [`yin::YinMpmEstimator`] struct via
//! [`yin::YinAlgorithm`]). The pYIN and neural backends
//! ([`crate::pitch::pyin::PYinEstimator`],
//! [`crate::pitch::crepe::CrepeTinyEstimator`]) are constructed
//! directly by callers; this factory exposes only the time-domain
//! backends.

use std::path::Path;

use crate::pitch::{
    EstimatorConfig, EstimatorError, PitchEstimator,
    yin::{self, YinMpmEstimator},
};

/// Stable identifier for the available pitch detection backends.
///
/// Currently contains `YinMpm` and `Mpm`, both implemented by
/// [`YinMpmEstimator`] under different [`yin::YinAlgorithm`] choices.
/// Callers MUST handle the case where a feature is not enabled, which
/// surfaces as [`EstimatorError::FeatureDisabled`].
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
