#![allow(clippy::doc_markdown)]
//! YIN (de Cheveigne & Kawahara 2002) and MPM (McLeod & Wyvill 2005)
//! pitch estimators.
//!
//! Two time-domain algorithms share one struct, [`YinMpmEstimator`], and one
//! set of pre-allocated buffers. The active algorithm is selected at
//! construction time via [`YinAlgorithm`]; both branches go through the same
//! parabolic-interpolation, clarity-gate, and RMS-gate machinery.
//!
//! # YIN
//!
//! Reference: A. de Cheveigne, H. Kawahara, *YIN, a fundamental frequency
//! estimator for speech and music*, J. Acoust. Soc. Am. 111(4), April 2002,
//! Section IV.B (the cumulative-mean-normalized difference function and the
//! absolute-threshold tau-picking rule).
//!
//! Steps implemented:
//! 1. Difference function `d(tau) = sum_t (x[t] - x[t+tau])^2` for
//!    `tau in [tau_min, tau_max]`. Naive O(W * (tau_max - tau_min)) form;
//!    for `window_size = 2048` this is roughly 4M FLOPs per frame, well
//!    within the latency budget.
//! 2. Cumulative-mean-normalized difference
//!    `d'(tau) = d(tau) * tau / cumsum_{i=1..=tau} d(i)`, with
//!    `d'(0) = 1.0` per the paper.
//! 3. Pick the smallest `tau` where `d'(tau) < threshold` AND `d'(tau)` is
//!    a local minimum.
//! 4. Refine via parabolic interpolation across `(tau-1, tau, tau+1)`.
//! 5. `clarity = 1.0 - d'(tau_refined)` is the confidence in `[0, 1]`.
//! 6. `f0_hz = sample_rate_hz / tau_refined`.
//!
//! # MPM
//!
//! Reference: P. McLeod, G. Wyvill, *A Smarter Way to Find Pitch*, ICMC 2005,
//! Section 4.5 (the normalized squared difference function and the
//! "largest peak above k * global_max" picking rule, with `k = 0.93`).
//!
//! Steps implemented:
//! 1. NSDF `m(tau) = 2 * sum_t x[t] * x[t + tau] /
//!    (sum_t x[t]^2 + sum_t x[t + tau]^2)`.
//! 2. Find local maxima after the first positive zero crossing.
//! 3. Pick the first local maximum whose value is above `k * global_max`.
//! 4. Refine via parabolic interpolation around the chosen lag.
//! 5. `clarity = m(tau_refined)`.
//! 6. `f0_hz = sample_rate_hz / tau_refined`.
//!
//! # Hot-path discipline
//!
//! All scratch buffers are allocated once in [`YinMpmEstimator::new`].
//! [`YinMpmEstimator::process`] writes into them in place and never allocates.
//!
//! # Voicing gate
//!
//! A frame is reported as `voiced = true` only if both:
//! - Clarity is above [`CLARITY_THRESHOLD`] (0.5), AND
//! - RMS of the input window is above [`RMS_GATE`] (0.001).
//!
//! When either gate fails, the estimator still emits a frame so the smoother
//! sees a continuous timestamp stream — but `voiced = false` and `f0_hz = 0.0`.

use crate::pitch::{EstimatorConfig, EstimatorError, F0Frame, PitchEstimator};

/// Minimum clarity (`1 - d'(tau)` for YIN; `m(tau)` for MPM) required to
/// flag a frame as voiced.
const CLARITY_THRESHOLD: f32 = 0.5;

/// Minimum RMS amplitude required to flag a frame as voiced. Gates the
/// near-silence corner case where the difference function is dominated by
/// numerical noise.
const RMS_GATE: f32 = 0.001;

/// Absolute threshold on `d'(tau)` for the YIN tau-picking rule. The original
/// paper recommends 0.10..=0.15 for clean signals; we use 0.10 as the default
/// and emit `voiced = false` when no tau falls below it.
const YIN_ABS_THRESHOLD: f32 = 0.10;

/// MPM peak-picking factor: pick the first local maximum of the NSDF whose
/// value is above `MPM_K * global_max`. McLeod & Wyvill recommend 0.8..=0.95;
/// we use 0.93 as the canonical default cited in the paper.
const MPM_K: f32 = 0.93;

/// Selects which time-domain algorithm [`YinMpmEstimator`] runs. The two
/// algorithms share buffers and surface the same [`F0Frame`] output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum YinAlgorithm {
    /// Classical YIN with cumulative-mean-normalized difference function and
    /// parabolic interpolation. Default.
    #[default]
    Yin,
    /// McLeod Pitch Method (MPM) with normalized squared difference function
    /// and parabolic interpolation.
    Mpm,
}

/// YIN/MPM pitch estimator.
///
/// Holds an [`EstimatorConfig`] and the pre-allocated scratch buffers needed
/// by the chosen [`YinAlgorithm`]. [`PitchEstimator::process`] does no heap
/// allocation after construction.
#[derive(Debug)]
pub struct YinMpmEstimator {
    cfg: EstimatorConfig,
    algorithm: YinAlgorithm,

    /// Smallest lag (in samples) considered, derived from `fmax_hz`.
    tau_min: usize,
    /// Largest lag (in samples) considered, derived from `fmin_hz` and
    /// `window_size / 2`.
    tau_max: usize,

    /// Constructor-time minimum lag floor (in samples). `process_with_range`
    /// MUST never widen `tau_min` below this value. The constructor floor
    /// is set to 2 so parabolic interpolation has a left neighbour at
    /// index `tau - 1 >= 1`.
    tau_min_floor: usize,
    /// Constructor-time maximum lag ceiling (in samples). `process_with_range`
    /// MUST never widen `tau_max` above this — the scratch buffers were
    /// sized for exactly this lag.
    tau_max_ceiling: usize,

    /// Scratch buffer holding either `d(tau)` (YIN) or `m(tau)` (MPM) for
    /// `tau in 0..=tau_max`. Length is `tau_max_ceiling + 1`.
    scratch: Vec<f32>,
    /// Scratch buffer holding `d'(tau)` (YIN only). Length is
    /// `tau_max_ceiling + 1`.
    cmnd: Vec<f32>,
    /// Scratch buffer of MPM local-maximum lag indices, owned by the
    /// estimator so [`Self::pick_mpm_tau`] is allocation-free on the hot
    /// path. Pre-sized to `tau_max_ceiling + 1`; `clear()` preserves the
    /// allocated capacity at the top of every call.
    mpm_maxima_idx: Vec<usize>,
    /// Parallel scratch buffer of MPM local-maximum NSDF values. Same
    /// invariant as [`Self::mpm_maxima_idx`].
    mpm_maxima_val: Vec<f32>,

    /// Monotonic timestamp counter, in samples since the last
    /// [`PitchEstimator::reset`]. Advances by `hop_size` on each `process`
    /// call. The trait does not surface this to callers; they read it back
    /// off the emitted [`F0Frame::timestamp_samples`].
    timestamp_samples: u64,
}

impl YinMpmEstimator {
    /// Construct a new estimator with the YIN algorithm (default).
    ///
    /// Equivalent to `Self::with_algorithm(cfg, YinAlgorithm::Yin)`.
    pub fn new(cfg: EstimatorConfig) -> Result<Self, EstimatorError> {
        Self::with_algorithm(cfg, YinAlgorithm::Yin)
    }

    /// Construct a new estimator with an explicit algorithm choice.
    ///
    /// Pre-allocates all scratch buffers; `process` is allocation-free after
    /// this call returns.
    ///
    /// Returns [`EstimatorError::Configuration`] if `fmin`/`fmax` produce an
    /// empty lag range, if the window is too small to span `tau_max`, or if
    /// the configuration is otherwise inconsistent.
    pub fn with_algorithm(
        cfg: EstimatorConfig,
        algorithm: YinAlgorithm,
    ) -> Result<Self, EstimatorError> {
        if cfg.window_size < 2 {
            return Err(EstimatorError::Configuration(
                "window_size must be >= 2".to_string(),
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
        {
            return Err(EstimatorError::Configuration(
                "require 0 < fmin_hz < fmax_hz, both finite".to_string(),
            ));
        }

        let sr = cfg.sample_rate_hz as f32;
        let tau_min_f = (sr / cfg.fmax_hz).floor();
        let tau_max_f = (sr / cfg.fmin_hz).ceil();

        // tau must be at least 2 so parabolic interpolation has a left
        // neighbour at index `tau - 1 >= 1`.
        let tau_min = (tau_min_f as usize).max(2);
        let half_window = cfg.window_size / 2;
        let tau_max = (tau_max_f as usize).min(half_window);

        if tau_max <= tau_min {
            return Err(EstimatorError::Configuration(
                "fmin/fmax produces empty tau range".to_string(),
            ));
        }

        let scratch_len = tau_max + 1;
        let scratch = vec![0.0_f32; scratch_len];
        let cmnd = vec![0.0_f32; scratch_len];
        // MPM local-maxima scratch: at most one maximum per lag, so
        // `scratch_len` is a safe upper bound. Reserved here so the
        // hot-path `clear()` in `pick_mpm_tau` cannot reallocate.
        let mpm_maxima_idx = Vec::with_capacity(scratch_len);
        let mpm_maxima_val = Vec::with_capacity(scratch_len);

        tracing::trace!(
            target: "neural_pitch_core::pitch::yin",
            window_size = cfg.window_size,
            hop_size = cfg.hop_size,
            sample_rate_hz = cfg.sample_rate_hz,
            tau_min,
            tau_max,
            ?algorithm,
            "YinMpmEstimator::with_algorithm"
        );

        Ok(Self {
            cfg,
            algorithm,
            tau_min,
            tau_max,
            tau_min_floor: tau_min,
            tau_max_ceiling: tau_max,
            scratch,
            cmnd,
            mpm_maxima_idx,
            mpm_maxima_val,
            timestamp_samples: 0,
        })
    }

    /// Recompute `tau_min`/`tau_max` from a caller-supplied search range,
    /// clamped to the constructor-time `[tau_min_floor, tau_max_ceiling]`
    /// budget so the pre-allocated scratch buffers always cover the
    /// resulting lag range. Returns `false` when the requested range is
    /// degenerate (non-finite, non-positive, or empty after clamping); in
    /// that case the caller's lag bounds are left unchanged and the
    /// estimator falls back to its constructor-time range.
    fn apply_range(&mut self, fmin_hz: f32, fmax_hz: f32) -> bool {
        if !(fmin_hz.is_finite() && fmax_hz.is_finite()) || fmin_hz <= 0.0 || fmax_hz <= fmin_hz {
            return false;
        }
        let sr = self.cfg.sample_rate_hz as f32;
        let tau_min_f = (sr / fmax_hz).floor();
        let tau_max_f = (sr / fmin_hz).ceil();
        let mut tau_min = tau_min_f as usize;
        if tau_min < self.tau_min_floor {
            tau_min = self.tau_min_floor;
        }
        let mut tau_max = tau_max_f as usize;
        if tau_max > self.tau_max_ceiling {
            tau_max = self.tau_max_ceiling;
        }
        if tau_max <= tau_min {
            return false;
        }
        self.tau_min = tau_min;
        self.tau_max = tau_max;
        true
    }

    /// Returns the active algorithm.
    pub fn algorithm(&self) -> YinAlgorithm {
        self.algorithm
    }

    /// Compute the YIN difference function `d(tau)` in-place into
    /// `self.scratch[0..=tau_max]`.
    fn fill_yin_difference(&mut self, samples: &[f32]) {
        let w = self.cfg.window_size;
        // d(0) is conventionally 0 — we never use it for picking, but write
        // it explicitly so cmnd[0] = 1.0 falls out clean.
        self.scratch[0] = 0.0;
        for tau in 1..=self.tau_max {
            // Sum runs over t in [0, w - tau).
            let limit = w - tau;
            let mut acc = 0.0_f32;
            for t in 0..limit {
                let diff = samples[t] - samples[t + tau];
                acc += diff * diff;
            }
            self.scratch[tau] = acc;
        }
    }

    /// Compute the cumulative-mean-normalized difference `d'(tau)` in-place
    /// from `self.scratch` into `self.cmnd`.
    fn fill_cmnd(&mut self) {
        // Per YIN paper Eq. (8): d'(0) = 1, d'(tau) = d(tau) * tau / cumsum.
        self.cmnd[0] = 1.0;
        let mut running = 0.0_f32;
        for tau in 1..=self.tau_max {
            let d_tau = self.scratch[tau];
            running += d_tau;
            if running > 0.0 {
                self.cmnd[tau] = d_tau * (tau as f32) / running;
            } else {
                // All-zero or numerically tiny window: report worst-case 1.0
                // so the picker never selects it.
                self.cmnd[tau] = 1.0;
            }
        }
    }

    /// Run the YIN tau-picking rule on `self.cmnd`. Returns the integer lag
    /// (within `[tau_min, tau_max - 1]`) of the chosen point, or `None`.
    fn pick_yin_tau(&self) -> Option<usize> {
        // Smallest tau >= tau_min with d'(tau) < threshold AND d'(tau) is a
        // local minimum (strictly less than both neighbours). Cap at
        // `tau_max - 1` so `tau + 1` is in range for parabolic interpolation.
        let tau_min = self.tau_min;
        let tau_hi = self.tau_max.saturating_sub(1);
        if tau_min + 1 > tau_hi {
            return None;
        }

        let mut chosen: Option<usize> = None;
        let mut tau = tau_min;
        while tau <= tau_hi {
            let v = self.cmnd[tau];
            if v < YIN_ABS_THRESHOLD {
                // Walk while values keep decreasing (find the bottom of this
                // dip) — the standard YIN refinement.
                let mut t = tau;
                while t < tau_hi && self.cmnd[t + 1] < self.cmnd[t] {
                    t += 1;
                }
                chosen = Some(t);
                break;
            }
            tau += 1;
        }
        chosen
    }

    /// Compute the MPM normalized squared difference `m(tau)` in-place into
    /// `self.scratch[0..=tau_max]`.
    fn fill_mpm_nsdf(&mut self, samples: &[f32]) {
        let w = self.cfg.window_size;
        for tau in 0..=self.tau_max {
            let limit = w - tau;
            let mut auto = 0.0_f32; // sum x[t] * x[t+tau]
            let mut energy = 0.0_f32; // sum x[t]^2 + x[t+tau]^2
            for t in 0..limit {
                let a = samples[t];
                let b = samples[t + tau];
                auto += a * b;
                energy += a * a + b * b;
            }
            self.scratch[tau] = if energy > 0.0 {
                2.0 * auto / energy
            } else {
                0.0
            };
        }
    }

    /// Run the MPM peak-picking rule on `self.scratch`. Returns the integer
    /// lag of the chosen local maximum, or `None`.
    ///
    /// Takes `&mut self` because the local-maxima scratch buffers
    /// ([`Self::mpm_maxima_idx`] / [`Self::mpm_maxima_val`]) are owned by
    /// the struct and reused across calls; `clear()` preserves their
    /// allocated capacity so this method is allocation-free on the hot path
    /// (mod-doc lines 47-50).
    fn pick_mpm_tau(&mut self) -> Option<usize> {
        // Step 1: walk until first positive zero crossing — i.e. find the
        // first index >= tau_min where m(tau) becomes >= 0 after having been
        // negative. Per McLeod & Wyvill, m(0) = 1 so we skip past the initial
        // positive lobe by waiting for the first crossing into negative.
        let tau_hi = self.tau_max.saturating_sub(1);
        if self.tau_min + 1 > tau_hi {
            return None;
        }

        // Find first index where NSDF goes from >= 0 to < 0 (descent) and
        // then back up — i.e. the start of the first negative-or-zero
        // valley. We then look for local maxima after that point.
        let mut start = self.tau_min;
        // Skip past the initial decay: advance while the NSDF is still
        // descending without a local maximum we'd want.
        // A simpler robust approach: collect all local maxima in
        // `[tau_min, tau_hi]`, drop those that occur before the first
        // negative value, then apply the k * global_max rule.
        let mut first_negative: Option<usize> = None;
        for tau in self.tau_min..=tau_hi {
            if self.scratch[tau] < 0.0 {
                first_negative = Some(tau);
                break;
            }
        }
        if let Some(neg) = first_negative {
            start = neg;
        }

        // Reuse owned scratch buffers; `clear()` preserves capacity so no
        // allocation happens on this hot path.
        self.mpm_maxima_idx.clear();
        self.mpm_maxima_val.clear();
        let mut tau = start.max(self.tau_min + 1);
        while tau < tau_hi {
            let v = self.scratch[tau];
            let l = self.scratch[tau - 1];
            let r = self.scratch[tau + 1];
            if v > 0.0 && v >= l && v >= r {
                self.mpm_maxima_idx.push(tau);
                self.mpm_maxima_val.push(v);
            }
            tau += 1;
        }
        if self.mpm_maxima_idx.is_empty() {
            return None;
        }

        // global_max is the largest local-maximum value (NOT the global max
        // of the whole NSDF, which is 1.0 at lag 0). McLeod & Wyvill: pick
        // the first peak whose value is above k * highest_peak_value.
        let global_max = self.mpm_maxima_val.iter().copied().fold(0.0_f32, f32::max);
        let threshold = MPM_K * global_max;
        for (idx, &v) in self
            .mpm_maxima_idx
            .iter()
            .copied()
            .zip(self.mpm_maxima_val.iter())
        {
            if v >= threshold {
                return Some(idx);
            }
        }
        None
    }

    /// Parabolic interpolation around `tau` using values from `buf`. Returns
    /// the refined fractional lag and the interpolated value.
    fn parabolic_refine(buf: &[f32], tau: usize) -> (f32, f32) {
        if tau == 0 || tau + 1 >= buf.len() {
            return (tau as f32, buf[tau]);
        }
        let y0 = buf[tau - 1];
        let y1 = buf[tau];
        let y2 = buf[tau + 1];
        let denom = y0 - 2.0 * y1 + y2;
        if denom.abs() < 1e-12 {
            return (tau as f32, y1);
        }
        let shift = 0.5 * (y0 - y2) / denom;
        // Clamp shift to [-1, 1] to defend against numerical pathologies.
        let shift = shift.clamp(-1.0, 1.0);
        let refined_tau = tau as f32 + shift;
        let refined_val = y1 - 0.25 * (y0 - y2) * shift;
        (refined_tau, refined_val)
    }

    /// Compute the RMS amplitude of a sample window.
    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let mut acc = 0.0_f64;
        for &s in samples {
            let s64 = f64::from(s);
            acc += s64 * s64;
        }
        let mean = acc / samples.len() as f64;
        mean.sqrt() as f32
    }

    /// Sanitize input: any NaN/infinity in the window produces a failed
    /// voicing decision rather than a panic.
    fn input_is_finite(samples: &[f32]) -> bool {
        samples.iter().all(|s| s.is_finite())
    }

    /// Build an unvoiced frame at the current timestamp with the given
    /// confidence reading.
    fn unvoiced_frame(&self, confidence: f32) -> F0Frame {
        F0Frame {
            f0_hz: 0.0,
            confidence: confidence.clamp(0.0, 1.0),
            voiced: false,
            timestamp_samples: self.timestamp_samples,
        }
    }
}

impl PitchEstimator for YinMpmEstimator {
    fn name(&self) -> &'static str {
        match self.algorithm {
            YinAlgorithm::Yin => "yin-mpm",
            YinAlgorithm::Mpm => "mpm",
        }
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

        // Defend against NaN / infinity at the module entry: produce an
        // unvoiced frame and advance the timestamp rather than panicking.
        if !Self::input_is_finite(samples) {
            let frame = self.unvoiced_frame(0.0);
            self.timestamp_samples = self
                .timestamp_samples
                .saturating_add(self.cfg.hop_size as u64);
            return Ok(Some(frame));
        }

        // Cheap RMS gate up front. Still emit a frame so callers see a
        // continuous timestamp stream.
        let rms = Self::rms(samples);

        let (tau_int_opt, picker_buf): (Option<usize>, &[f32]) = match self.algorithm {
            YinAlgorithm::Yin => {
                self.fill_yin_difference(samples);
                self.fill_cmnd();
                (self.pick_yin_tau(), self.cmnd.as_slice())
            }
            YinAlgorithm::Mpm => {
                self.fill_mpm_nsdf(samples);
                // `pick_mpm_tau` takes `&mut self` because it writes to the
                // owned local-maxima scratch slices, but it does not mutate
                // `self.scratch` itself. Drop the mut borrow before re-
                // borrowing `self.scratch` immutably for `picker_buf`.
                let tau_int_opt = self.pick_mpm_tau();
                (tau_int_opt, self.scratch.as_slice())
            }
        };

        let frame = match tau_int_opt {
            None => self.unvoiced_frame(0.0),
            Some(tau_int) => {
                let (tau_refined, picker_val) = Self::parabolic_refine(picker_buf, tau_int);
                // Clarity is "how confident is the picker"; computed
                // differently for YIN vs MPM but always lives in [0, 1].
                let clarity = match self.algorithm {
                    YinAlgorithm::Yin => (1.0 - picker_val).clamp(0.0, 1.0),
                    YinAlgorithm::Mpm => picker_val.clamp(0.0, 1.0),
                };

                if tau_refined <= 0.0 {
                    // Numerical pathology — refuse to divide.
                    self.unvoiced_frame(clarity)
                } else {
                    let f0_hz = self.cfg.sample_rate_hz as f32 / tau_refined;
                    let voiced = clarity > CLARITY_THRESHOLD && rms > RMS_GATE;
                    if voiced {
                        F0Frame {
                            f0_hz,
                            confidence: clarity,
                            voiced: true,
                            timestamp_samples: self.timestamp_samples,
                        }
                    } else {
                        self.unvoiced_frame(clarity)
                    }
                }
            }
        };

        self.timestamp_samples = self
            .timestamp_samples
            .saturating_add(self.cfg.hop_size as u64);

        Ok(Some(frame))
    }

    fn process_with_range(
        &mut self,
        samples: &[f32],
        fmin_hz: f32,
        fmax_hz: f32,
    ) -> Result<Option<F0Frame>, EstimatorError> {
        // Narrow tau bounds in place. On a degenerate request (non-finite,
        // non-positive, or empty intersection with the constructor budget)
        // restore the constructor-time bounds so the trait contract at
        // `pitch/mod.rs:170-178` ("degrade cleanly to their constructor-time
        // range") holds — without this restore, a single bad frame would
        // sticky-pin the lag bounds at the most-recent-good range. The
        // restore stays inside the constructor allocation so no heap work
        // can occur on this hot path.
        if !self.apply_range(fmin_hz, fmax_hz) {
            self.tau_min = self.tau_min_floor;
            self.tau_max = self.tau_max_ceiling;
        }
        self.process(samples)
    }

    fn reset(&mut self) {
        self.scratch.fill(0.0);
        self.cmnd.fill(0.0);
        self.timestamp_samples = 0;
        // Restore the lag range to the constructor-time budget so
        // `process` after `reset` behaves as a freshly-constructed
        // estimator, per the trait contract.
        self.tau_min = self.tau_min_floor;
        self.tau_max = self.tau_max_ceiling;
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp
)]
mod tests {
    use super::*;
    use crate::pitch::InstrumentHint;
    use crate::test_utils::signals::{silence, sine_wave};

    fn cfg_voice() -> EstimatorConfig {
        EstimatorConfig {
            sample_rate_hz: 48_000,
            window_size: 2048,
            hop_size: 512,
            fmin_hz: 50.0,
            fmax_hz: 1500.0,
            instrument_hint: Some(InstrumentHint::Voice),
        }
    }

    #[test]
    fn empty_tau_range_errors() {
        let cfg = EstimatorConfig {
            sample_rate_hz: 48_000,
            window_size: 2048,
            hop_size: 512,
            // fmin == fmax: rejected (we require fmax > fmin).
            fmin_hz: 200.0,
            fmax_hz: 200.0,
            instrument_hint: None,
        };
        let result = YinMpmEstimator::new(cfg);
        assert!(matches!(result, Err(EstimatorError::Configuration(_))));
    }

    #[test]
    fn timestamp_advances_by_hop() {
        let cfg = cfg_voice();
        let hop = cfg.hop_size as u64;
        let mut est = YinMpmEstimator::new(cfg.clone()).expect("ctor");
        let buf = sine_wave(440.0, cfg.sample_rate_hz, cfg.window_size);
        let f0 = est.process(&buf).expect("ok").expect("frame");
        assert_eq!(f0.timestamp_samples, 0);
        let f1 = est.process(&buf).expect("ok").expect("frame");
        assert_eq!(f1.timestamp_samples, hop);
        est.reset();
        let f2 = est.process(&buf).expect("ok").expect("frame");
        assert_eq!(f2.timestamp_samples, 0);
    }

    #[test]
    fn silence_is_unvoiced() {
        let cfg = cfg_voice();
        let mut est = YinMpmEstimator::new(cfg.clone()).expect("ctor");
        let buf = silence(cfg.window_size);
        let frame = est.process(&buf).expect("ok").expect("frame");
        assert!(!frame.voiced);
        assert_eq!(frame.f0_hz, 0.0);
    }

    #[test]
    fn nan_input_does_not_panic() {
        let cfg = cfg_voice();
        let mut est = YinMpmEstimator::new(cfg.clone()).expect("ctor");
        let mut buf = sine_wave(440.0, cfg.sample_rate_hz, cfg.window_size);
        buf[0] = f32::NAN;
        let frame = est.process(&buf).expect("ok").expect("frame");
        assert!(!frame.voiced);
    }

    #[test]
    fn window_mismatch_errors() {
        let cfg = cfg_voice();
        let mut est = YinMpmEstimator::new(cfg).expect("ctor");
        let buf = vec![0.0_f32; 1024];
        let result = est.process(&buf);
        assert!(matches!(result, Err(EstimatorError::WindowMismatch { .. })));
    }
}
