//! Pitch detection trait surface and supporting types.
//!
//! The [`PitchEstimator`] trait is the single backend-agnostic interface for
//! all pitch detection algorithms shipped or planned in `neural-pitch-core`
//! (YIN/MPM in Phase 1; pYIN, PESTO, CREPE-tiny in Phase 2). Pipelines own
//! exactly one boxed estimator at a time and call [`PitchEstimator::process`]
//! on hop-aligned sample chunks.
//!
//! # Contract
//!
//! - **Octave-error responsibility**: each backend is responsible for its own
//!   octave-error correction internally. Callers do not post-process f0
//!   estimates to fix octave jumps; that is a property of the backend
//!   implementation.
//! - **Voicing semantics**: [`F0Frame::voiced`] is the conjunction of the
//!   estimator's internal voicing decision and any caller-side gate
//!   ([`crate::voicing::VoiceActivityGate`]). When `voiced` is `false`, the
//!   `f0_hz` field is meaningless and SHOULD be ignored by consumers.
//! - **Stateful backends**: [`PitchEstimator::process`] takes `&mut self`
//!   because backends maintain internal buffers (overlap-add, Viterbi state,
//!   etc.). The trait does not require `Sync`; pipelines own estimators
//!   exclusively and SHOULD NOT introduce internal `Mutex`es.
//! - **Hot-path discipline**: implementations MUST NOT allocate, log, or
//!   perform I/O after the first call to [`PitchEstimator::process`]. Model
//!   loading happens in `new()`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod auto_prior;
pub mod factory;
pub mod yin;

/// One frame of fundamental-frequency analysis.
///
/// `F0Frame` does **not** derive `PartialEq` or `Eq` because `f32` fields can
/// be `NaN` and exact equality is rarely the right comparison. Tests should
/// use cents-based or absolute-tolerance helpers instead.
#[derive(Clone, Copy, Debug)]
pub struct F0Frame {
    /// Estimated fundamental in Hertz. Always greater than zero when
    /// [`F0Frame::voiced`] is `true`.
    pub f0_hz: f32,

    /// Estimator-reported confidence, normalised to `[0.0, 1.0]`.
    pub confidence: f32,

    /// Conjunction of the estimator's internal voicing decision and any
    /// caller-side voice-activity gate. When `false`, `f0_hz` is meaningless.
    pub voiced: bool,

    /// Sample-accurate timestamp of the analysis frame's centre, measured in
    /// samples since the most recent [`PitchEstimator::reset`].
    pub timestamp_samples: u64,
}

/// Configuration for a pitch estimator instance.
///
/// `EstimatorConfig` is owned by the estimator after construction. The
/// [`PitchEstimator::config`] accessor returns a borrow so pipelines can read
/// the active window/hop size without taking ownership.
#[derive(Clone, Debug)]
pub struct EstimatorConfig {
    /// Sample rate of incoming audio, in Hertz.
    pub sample_rate_hz: u32,

    /// Analysis window size, in samples.
    pub window_size: usize,

    /// Hop size between consecutive analysis frames, in samples.
    pub hop_size: usize,

    /// Lower bound of the search range, in Hertz.
    pub fmin_hz: f32,

    /// Upper bound of the search range, in Hertz.
    pub fmax_hz: f32,

    /// Optional instrument hint; backends MAY use it to bias priors or
    /// adjust the search range. `None` means no hint.
    pub instrument_hint: Option<InstrumentHint>,
}

/// Coarse instrument category used to bias backend priors.
///
/// Backends are not required to implement instrument-specific logic; the hint
/// is advisory. `Generic` is the safe default when no information is known.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstrumentHint {
    /// Singing or speaking voice.
    Voice,
    /// Six-string electric or acoustic guitar.
    Guitar,
    /// Four- or five-string electric/acoustic bass.
    Bass,
    /// Acoustic or electric piano.
    Piano,
    /// Bowed string instrument with violin-family range.
    Violin,
    /// No instrument-specific prior.
    Generic,
}

/// Errors returned by [`PitchEstimator::process`] and the factory.
#[derive(Debug, Error)]
pub enum EstimatorError {
    /// A required model weights file was not found at the resolved path.
    #[error("model file not found: {0}")]
    ModelNotFound(PathBuf),

    /// The ONNX runtime returned an error during model loading or inference.
    #[error("ort runtime error: {0}")]
    Ort(String),

    /// The input chunk size did not match the estimator's configured window.
    #[error("input frame size {got} != expected {want}")]
    WindowMismatch {
        /// The size that was actually supplied to `process`.
        got: usize,
        /// The size the estimator was configured to expect.
        want: usize,
    },

    /// The requested backend or capability is gated behind a Cargo feature
    /// that was not enabled at compile time.
    #[error("backend disabled at compile time: feature = \"{0}\"")]
    FeatureDisabled(&'static str),

    /// The supplied [`EstimatorConfig`] was internally inconsistent or
    /// otherwise unusable for the chosen backend.
    #[error("invalid configuration: {0}")]
    Configuration(String),
}

/// Backend-agnostic pitch estimator interface.
///
/// Estimator instances are not designed to be shared across threads; pipelines
/// own them exclusively. The `Send` bound is required because pipelines hand
/// estimators to dedicated DSP workers; no `Sync` bound is required and impls
/// SHOULD NOT introduce internal `Mutex`es.
///
/// See the module-level documentation for the contract on octave-error
/// handling, voicing semantics, and hot-path discipline.
pub trait PitchEstimator: Send {
    /// Stable identifier for this backend, e.g. `"yin-mpm"`.
    fn name(&self) -> &'static str;

    /// Borrow the configuration the estimator was constructed with.
    fn config(&self) -> &EstimatorConfig;

    /// Process one chunk of input samples and return zero or one
    /// [`F0Frame`]. Implementations that buffer internally MAY return `None`
    /// while still warming up.
    fn process(&mut self, samples: &[f32]) -> Result<Option<F0Frame>, EstimatorError>;

    /// Process one chunk with a per-call search range override.
    ///
    /// The default implementation ignores `fmin_hz`/`fmax_hz` and forwards
    /// to [`PitchEstimator::process`]. Backends MAY override this to
    /// recompute lag bounds without rebuilding their scratch buffers — the
    /// override MUST stay within the constructor-time allocated budget so
    /// no allocation occurs on the hot path.
    ///
    /// Callers (in particular [`crate::pipeline::DspWorker`] driving an
    /// [`crate::pitch::auto_prior::AutoPrior`]) pass the running auto-prior
    /// range every iteration. Backends that have not yet implemented
    /// per-call narrowing degrade cleanly to their constructor-time range.
    fn process_with_range(
        &mut self,
        samples: &[f32],
        fmin_hz: f32,
        fmax_hz: f32,
    ) -> Result<Option<F0Frame>, EstimatorError> {
        let _ = (fmin_hz, fmax_hz);
        self.process(samples)
    }

    /// Drop all internal state (buffers, Viterbi paths, timestamp counter).
    /// After `reset`, the next [`PitchEstimator::process`] call behaves as if
    /// the estimator had just been constructed.
    fn reset(&mut self);
}
