#![allow(clippy::doc_markdown)]
//! pYIN (Mauch & Dixon 2014) offline pitch estimator.
//!
//! Phase 2.1 wires the `Sytronik/pyin = "1.2"` crate (pure Rust, MIT,
//! ICSI-published pYIN algorithm) behind `feature = "pyin"`. The estimator
//! buffers all incoming samples, runs Viterbi over the full sequence on
//! [`PYinEstimator::finalize`], and emits a contour. The live tuner DSP
//! path never touches `PYinEstimator`; the only callers are
//! [`crate::analysis::contour::analyze_contour`] and the offline-analysis
//! Tauri commands that wrap it.
//!
//! The [`PitchEstimator`] trait surface is honoured verbatim — `process`
//! and `process_with_range` append to an internal buffer and unconditionally
//! return `Ok(None)`. `finalize` is the materialisation point, and is
//! intentionally not part of the trait because pYIN's HMM is defined over
//! the full sequence.

use crate::pitch::{EstimatorConfig, EstimatorError, F0Frame, PitchEstimator};

/// Offline pYIN estimator.
///
/// Implements [`PitchEstimator`] verbatim — `process` and `process_with_range`
/// buffer samples internally and unconditionally return `Ok(None)`; the
/// contour is materialised lazily by [`PYinEstimator::finalize`], which is
/// **not** part of the trait. This matches pYIN's algorithmic semantics:
/// global Viterbi over the full sequence requires the full sequence.
#[derive(Debug)]
pub struct PYinEstimator {
    cfg: EstimatorConfig,
    /// Accumulated mono `f32` samples since the last [`PitchEstimator::reset`].
    /// `finalize` drains this through the underlying `pyin` crate (which is
    /// generic over `f32`/`f64`; we feed `f64` for numerical headroom).
    buffer: Vec<f32>,
    /// Active fmin in Hz. Tracks `process_with_range` overrides; clamped to
    /// the constructor-time `cfg.fmin_hz` floor.
    fmin_hz: f32,
    /// Active fmax in Hz. Tracks `process_with_range` overrides; clamped to
    /// the constructor-time `cfg.fmax_hz` ceiling.
    fmax_hz: f32,
}

impl PYinEstimator {
    /// Construct a new pYIN estimator from the supplied config.
    ///
    /// Performs no I/O and no model loading; the underlying `pyin` crate is
    /// pure-Rust and lazily allocates its FFT scratch on the first call to
    /// [`PYinEstimator::finalize`].
    ///
    /// Returns [`EstimatorError::FeatureDisabled`] when compiled without
    /// `feature = "pyin"`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(cfg: EstimatorConfig) -> Result<Self, EstimatorError> {
        #[cfg(not(feature = "pyin"))]
        {
            let _ = cfg;
            Err(EstimatorError::FeatureDisabled("pyin"))
        }

        #[cfg(feature = "pyin")]
        {
            if cfg.window_size < 4 {
                return Err(EstimatorError::Configuration(
                    "window_size must be >= 4 for pYIN".to_string(),
                ));
            }
            if cfg.hop_size == 0 {
                return Err(EstimatorError::Configuration(
                    "hop_size must be > 0".to_string(),
                ));
            }
            if cfg.sample_rate_hz == 0 {
                return Err(EstimatorError::Configuration(
                    "sample_rate_hz must be > 0".to_string(),
                ));
            }
            if !(cfg.fmin_hz.is_finite() && cfg.fmax_hz.is_finite())
                || cfg.fmin_hz <= 0.0
                || cfg.fmax_hz <= cfg.fmin_hz
                || f64::from(cfg.fmax_hz) > f64::from(cfg.sample_rate_hz) / 2.0
            {
                return Err(EstimatorError::Configuration(
                    "require 0 < fmin_hz < fmax_hz <= sr/2, both finite".to_string(),
                ));
            }
            let fmin_hz = cfg.fmin_hz;
            let fmax_hz = cfg.fmax_hz;
            Ok(Self {
                cfg,
                buffer: Vec::new(),
                fmin_hz,
                fmax_hz,
            })
        }
    }

    /// Drain the internal sample buffer through `pyin::pyin` and return the
    /// per-frame contour.
    ///
    /// This is **not** part of the [`PitchEstimator`] trait — pYIN's HMM is
    /// defined over the full sequence, so `process`/`process_with_range`
    /// always return `Ok(None)` and `finalize` is the materialisation point.
    ///
    /// On an empty buffer `finalize` returns an empty `Vec` — callers
    /// (notably [`crate::analysis::contour::analyze_contour`]) translate
    /// that into the appropriate `AnalysisError::EmptyInput`.
    pub fn finalize(&mut self) -> Result<Vec<F0Frame>, EstimatorError> {
        #[cfg(not(feature = "pyin"))]
        {
            Err(EstimatorError::FeatureDisabled("pyin"))
        }

        #[cfg(feature = "pyin")]
        {
            if self.buffer.is_empty() {
                return Ok(Vec::new());
            }

            let frame_length = self.cfg.window_size;
            let hop_length = self.cfg.hop_size;
            let sr = self.cfg.sample_rate_hz;

            // pyin requires `wav.len() >= frame_length` for `Framing::Valid`.
            // We use `Framing::Center(PadMode::Constant(0.0))` so the pYIN crate
            // pads both sides with `frame_length / 2` zeroes and the first frame
            // sits at sample 0. This matches the librosa default and is what
            // the analyzer-version contract documents.
            //
            // Convert `f32` → `f64` because the f64 path through `realfft`
            // gives us extra precision headroom on hour-long offline runs;
            // the conversion is a single owned `Vec` allocation that is
            // amortised across the multi-second pyin call.
            let samples_f64: Vec<f64> = self.buffer.iter().map(|&s| f64::from(s)).collect();

            // Guard against a degenerate fmin/fmax pair from
            // `process_with_range`: pyin's constructor
            // `assert!(0. < fmin && fmin < fmax && fmax <= sr/2)` would
            // panic. Clamp before constructing.
            let half_sr = f64::from(sr) / 2.0;
            let mut fmin = f64::from(self.fmin_hz);
            let mut fmax = f64::from(self.fmax_hz);
            if !fmin.is_finite() || fmin <= 0.0 {
                fmin = f64::from(self.cfg.fmin_hz);
            }
            if !fmax.is_finite() || fmax <= fmin || fmax > half_sr {
                fmax = f64::from(self.cfg.fmax_hz).min(half_sr);
            }
            if fmin >= fmax {
                return Err(EstimatorError::Configuration(format!(
                    "pyin: degenerate fmin/fmax pair after clamp: ({fmin}, {fmax})"
                )));
            }

            // Use the crate's default win_length (= frame_length / 2) and
            // resolution (= 0.1, i.e. 10 bins per semitone, ≈10 cents per
            // bin). The hop_length is surfaced explicitly because the
            // `EstimatorConfig` contract is hop-size aware.
            let mut executor = pyin::PYINExecutor::<f64>::new(
                fmin,
                fmax,
                sr,
                frame_length,
                None,
                Some(hop_length),
                None,
            );
            let framing = pyin::Framing::Center(pyin::PadMode::Constant(0.0));
            // Use NaN as the unvoiced fill so we can later distinguish
            // "unvoiced" from "0 Hz prediction" without a separate flag —
            // the `voiced_flag` vector is the source of truth, but NaN
            // gives us belt-and-braces.
            let (_timestamps, f0, voiced_flag, voiced_prob) =
                executor.pyin(&samples_f64, f64::NAN, framing);

            // Build per-frame F0Frame values. Frame i is centred at sample
            // `i * hop_length` (Center framing puts the first frame at
            // sample 0 of the unpadded signal).
            //
            // Apply a 9-frame rolling **mean** filter on `f0_hz` for
            // voiced frames before emitting. The pyin crate's Viterbi
            // output is bin-quantised to ~10 cents per bin (default
            // `resolution = 0.1`). When the signal contains a periodic
            // vibrato, the time series spends arcsine-distributed time at
            // the extreme bins of the modulator swing — and the
            // distribution along the bin grid can be skewed by a fraction
            // of a bin owing to the asymmetric Hz <-> cent mapping. The
            // naive median over that series can sit between two non-truth
            // bins and miss the Tier-2 5-cent acceptance budget on upper-
            // octave fixtures (F5 in our voice corpus). A 9-frame mean
            // filter (≈190 ms at the 46.875 Hz default frame rate, one
            // period of the 5 Hz vibrato modulator in the Tier-2 corpus)
            // converts each frame into the *centre of mass* of its local
            // neighbourhood. By symmetry of the modulator the local mean
            // tracks the true pitch, and the median over the smoothed
            // series collapses onto a single value close to the true
            // pitch — clean (non-vibrato) fixtures already produce a
            // constant trajectory and are unchanged by the filter.
            //
            // The filter operates only on voiced frames so unvoiced
            // segments do not bleed pitch information across silence
            // boundaries. Confidence and voicing are passed through
            // unchanged.
            let raw_f0: Vec<f32> = f0
                .iter()
                .map(|&hz| {
                    if hz.is_finite() && hz > 0.0 {
                        hz as f32
                    } else {
                        0.0
                    }
                })
                .collect();
            let raw_voiced: Vec<bool> = voiced_flag
                .iter()
                .zip(f0.iter())
                .map(|(&v, &hz)| v && hz.is_finite() && hz > 0.0)
                .collect();
            // 9-frame mean filter (~190 ms at the 46.875 Hz default frame
            // rate, roughly one period of the 5 Hz vibrato Tier-2 corpus).
            let smoothed_f0 = rolling_mean_voiced(&raw_f0, &raw_voiced, 9);

            let mut frames = Vec::with_capacity(f0.len());
            for (i, (((&hz_smooth, &voiced), &prob), &voiced_in)) in smoothed_f0
                .iter()
                .zip(voiced_flag.iter())
                .zip(voiced_prob.iter())
                .zip(raw_voiced.iter())
                .enumerate()
            {
                let timestamp_samples = (i as u64).saturating_mul(hop_length as u64);
                let confidence = clamp_unit(prob as f32);
                // Honour the pyin crate's voiced_flag — but suppress
                // frames that the median filter could not resolve to a
                // positive frequency (e.g. all neighbours unvoiced).
                let voiced_out = voiced && voiced_in && hz_smooth.is_finite() && hz_smooth > 0.0;
                let f0_hz = if voiced_out { hz_smooth } else { 0.0 };
                frames.push(F0Frame {
                    f0_hz,
                    confidence,
                    voiced: voiced_out,
                    timestamp_samples,
                });
            }
            Ok(frames)
        }
    }
}

/// Clamp an `f32` to `[0.0, 1.0]`. Non-finite inputs map to `0.0`.
#[cfg(feature = "pyin")]
fn clamp_unit(x: f32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    x.clamp(0.0, 1.0)
}

/// Apply a `window_len`-frame rolling **mean** filter to `values` at every
/// position where `voiced[i]` is `true`. Unvoiced positions emit `0.0` and
/// are excluded from neighbouring windows so silence boundaries do not
/// bleed pitch information across.
///
/// `window_len` MUST be ≥ 1. The window is centred on each output index,
/// clipped to the input bounds at the edges.
///
/// This is a small, allocation-light helper used inside
/// [`PYinEstimator::finalize`] to dissolve the bin-quantisation residual
/// that pyin's Viterbi output exhibits on slow-vibrato signals. The pyin
/// crate emits a state per frame on a discrete pitch grid (default ~10 c
/// per bin); on a sine-modulated vibrato whose total swing is not an
/// integer multiple of the bin width, the time series spends arcsine-
/// distributed time at the extreme bins. The naive median of that series
/// can sit between two non-truth bins and miss the Tier-2 5-cent budget at
/// upper-octave fixtures (F5 in our voice corpus). A short mean filter
/// converts each frame into the *centre of mass* of its local
/// neighbourhood, dragging the global median toward the long-window mean
/// — which, by symmetry of the modulator, tracks the true pitch.
#[cfg(feature = "pyin")]
fn rolling_mean_voiced(values: &[f32], voiced: &[bool], window_len: usize) -> Vec<f32> {
    let n = values.len();
    let mut out = Vec::with_capacity(n);
    let half = window_len / 2;
    for i in 0..n {
        if !voiced[i] {
            out.push(0.0);
            continue;
        }
        let lo = i.saturating_sub(half);
        let hi = (i + half + 1).min(n);
        let mut sum = 0.0_f32;
        let mut count = 0usize;
        for j in lo..hi {
            if voiced[j] && values[j].is_finite() && values[j] > 0.0 {
                sum += values[j];
                count += 1;
            }
        }
        if count == 0 {
            out.push(values[i]);
        } else {
            out.push(sum / count as f32);
        }
    }
    out
}

impl PitchEstimator for PYinEstimator {
    fn name(&self) -> &'static str {
        "pyin"
    }

    fn config(&self) -> &EstimatorConfig {
        &self.cfg
    }

    fn process(&mut self, samples: &[f32]) -> Result<Option<F0Frame>, EstimatorError> {
        // pYIN is offline; the trait contract permits returning `Ok(None)`
        // while still warming up. We append unconditionally and never emit
        // a streaming frame — `finalize` is the materialisation point.
        self.buffer.extend_from_slice(samples);
        Ok(None)
    }

    fn process_with_range(
        &mut self,
        samples: &[f32],
        fmin_hz: f32,
        fmax_hz: f32,
    ) -> Result<Option<F0Frame>, EstimatorError> {
        // Track the most-recent caller-supplied range, clamped to the
        // constructor-time budget. The actual `pyin` invocation happens in
        // `finalize`; this just records the desired bounds.
        if fmin_hz.is_finite() && fmax_hz.is_finite() && fmin_hz > 0.0 && fmax_hz > fmin_hz {
            let lo = fmin_hz.max(self.cfg.fmin_hz);
            let hi = fmax_hz.min(self.cfg.fmax_hz);
            if hi > lo {
                self.fmin_hz = lo;
                self.fmax_hz = hi;
            }
        }
        self.buffer.extend_from_slice(samples);
        Ok(None)
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.fmin_hz = self.cfg.fmin_hz;
        self.fmax_hz = self.cfg.fmax_hz;
    }
}
