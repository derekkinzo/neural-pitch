//! Phase 2.3 — vibrato detector (4–7 Hz residual FFT).
//!
//! [`compute_vibrato`] consumes a
//! [`crate::analysis::contour::ContourResult`] and emits a
//! [`VibratoReport`] describing per-window vibrato rate and extent in the
//! 4–7 Hz band, plus whole-recording aggregate medians. See the Phase 2.3
//! algorithm memo for the full specification.
//!
//! Algorithm overview:
//!   1. Slide a 1-second window with 50% overlap across
//!      `contour.smoothed_cents`.
//!   2. Median-filter (kernel = 20% of window length) to derive the
//!      "intended" pitch baseline.
//!   3. FFT the residual (zero-padded to 1024 bins) via `realfft`.
//!   4. Find the peak magnitude in the 4–7 Hz band; emit a
//!      [`VibratoWindow`] when extent ≥ 5 cents.
//!
//! Per-window output is preserved on the wire so downstream UIs can
//! visualise vibrato rate over time without re-analysis.

use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use realfft::{RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

use crate::analysis::contour::ContourResult;

/// Cached `RealFftPlanner<f32>` shared across `compute_vibrato` calls.
///
/// `RealFftPlanner::new()` allocates twiddle-factor tables on
/// construction and `plan_fft_forward` keeps an internal cache of plans
/// keyed by length. `compute_vibrato` is called on every cache-hit (i.e.
/// every IPC `analyze_recording` / `get_vibrato_report` round-trip) so
/// minting a fresh planner per call burns allocator + setup time on the
/// hot path. We share one planner under a `Mutex` — the lock is held only
/// long enough to call `plan_fft_forward(FFT_LEN)`, which returns a cheap
/// `Arc<dyn RealToComplex<f32>>` clone, so contention between concurrent
/// IPC calls is bounded by the planner-lookup cost (microseconds).
fn shared_r2c() -> Arc<dyn RealToComplex<f32>> {
    static PLANNER: OnceLock<Mutex<RealFftPlanner<f32>>> = OnceLock::new();
    let planner_mutex = PLANNER.get_or_init(|| Mutex::new(RealFftPlanner::<f32>::new()));
    // `parking_lot::Mutex` does not poison on panic (ADR-0014), so a single
    // `lock()` call suffices.
    let mut planner = planner_mutex.lock();
    planner.plan_fft_forward(FFT_LEN)
}

/// Zero-padded FFT length (real input). Bin width at 93.75 fps is
/// ~0.0916 Hz — well below the 0.3 Hz separation needed inside the
/// 4–7 Hz vibrato band.
const FFT_LEN: usize = 1024;

/// Lower edge of the vibrato detection band, in Hertz.
const VIBRATO_BAND_LOW_HZ: f32 = 4.0;

/// Upper edge of the vibrato detection band, in Hertz.
const VIBRATO_BAND_HIGH_HZ: f32 = 7.0;

/// Lower edge of the confidence-floor band (peak / mean magnitude).
const CONFIDENCE_BAND_LOW_HZ: f32 = 1.0;

/// Upper edge of the confidence-floor band.
const CONFIDENCE_BAND_HIGH_HZ: f32 = 10.0;

/// Cents threshold below which a window is reported with confidence 0
/// and excluded from the per-window aggregate.
const EXTENT_FLOOR_CENTS: f32 = 5.0;

/// One detected vibrato window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VibratoWindow {
    /// Frame index (in the contour) where this window starts.
    pub start_frame: u32,
    /// Detected vibrato rate in Hertz, peak of the 4–7 Hz FFT bin band.
    pub rate_hz: f32,
    /// Vibrato extent in cents (peak-to-zero of the time-domain
    /// residual, derived from the FFT amplitude).
    pub extent_cents: f32,
    /// Confidence in `[0.0, 1.0]` — peak-to-floor ratio across the
    /// 1–10 Hz band. `0.0` means "no vibrato detected in this window".
    pub confidence_0_to_1: f32,
}

/// Whole-recording vibrato report.
///
/// Field order is fixed for postcard byte-equal round-trips — do not
/// reorder without bumping the analyzer-version cache key (ADR-0012).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VibratoReport {
    /// One entry per detected vibrato window. Skipped windows
    /// (extent < 5 cents) are omitted from this vector but still
    /// counted in the [`Self::vibrato_ratio`] denominator via
    /// `total_window_count` bookkeeping inside the analyzer.
    pub per_window: Vec<VibratoWindow>,
    /// Median rate across detected windows, in Hertz. `0.0` when
    /// `per_window` is empty.
    pub median_rate_hz: f32,
    /// Median extent across detected windows, in cents. `0.0` when
    /// `per_window` is empty.
    pub median_extent_cents: f32,
    /// `detected_window_count / total_window_count`. `0.0` when the
    /// recording is shorter than one window.
    pub vibrato_ratio: f32,
}

/// Compute the vibrato report for one contour.
///
/// `a4_hz` is the caller-supplied reference pitch (per ADR-0005). The
/// vibrato analysis itself operates on `smoothed_cents` (already
/// relative to `a4_hz`) but the parameter is kept on the public
/// signature for API symmetry with [`crate::analysis::range::compute_range`]
/// and to leave room for future work that needs the absolute pitch
/// reference (e.g. instrument-specific extent thresholds).
#[must_use]
pub fn compute_vibrato(contour: &ContourResult, a4_hz: f32) -> VibratoReport {
    // a4_hz is intentionally unused today (see doc-comment above); future
    // hooks may consume it for instrument-specific thresholds.
    let _ = a4_hz;

    let frame_rate_hz = contour.frame_rate_hz;
    if !(frame_rate_hz.is_finite() && frame_rate_hz > 0.0) {
        return empty_report();
    }
    let window_len = frame_rate_hz.round() as usize;
    if window_len == 0 || window_len > FFT_LEN {
        return empty_report();
    }
    let hop = (window_len / 2).max(1);

    let cents = &contour.smoothed_cents;
    if cents.len() < window_len {
        return empty_report();
    }

    let kernel = pick_kernel(window_len);

    let r2c = shared_r2c();
    let mut input_buf = r2c.make_input_vec();
    let mut output_buf = r2c.make_output_vec();
    let mut scratch = r2c.make_scratch_vec();

    let bin_width_hz = frame_rate_hz / FFT_LEN as f32;

    let mut per_window: Vec<VibratoWindow> = Vec::new();
    let mut total_windows: u32 = 0;

    let mut start = 0_usize;
    while start + window_len <= cents.len() {
        let raw = &cents[start..start + window_len];
        match analyse_window(
            raw,
            start,
            window_len,
            kernel,
            bin_width_hz,
            r2c.as_ref(),
            &mut input_buf,
            &mut output_buf,
            &mut scratch,
        ) {
            WindowOutcome::Detected(w) => per_window.push(w),
            WindowOutcome::BelowFloor | WindowOutcome::Skipped => {}
        }
        total_windows = total_windows.saturating_add(1);
        start = start.saturating_add(hop);
    }

    aggregate(per_window, total_windows)
}

/// Median-filter kernel: 20% of window length, snapped odd. Minimum 3 so
/// the median is well-defined; capped at the window length.
fn pick_kernel(window_len: usize) -> usize {
    let mut kernel = (window_len as f32 * 0.2).round() as usize;
    if kernel < 3 {
        kernel = 3;
    }
    if kernel.is_multiple_of(2) {
        kernel = kernel.saturating_add(1);
    }
    if kernel > window_len {
        kernel = if window_len.is_multiple_of(2) {
            window_len.saturating_sub(1).max(1)
        } else {
            window_len
        };
    }
    kernel
}

enum WindowOutcome {
    Detected(VibratoWindow),
    BelowFloor,
    Skipped,
}

#[allow(clippy::too_many_arguments)]
fn analyse_window(
    raw: &[f32],
    start: usize,
    window_len: usize,
    kernel: usize,
    bin_width_hz: f32,
    r2c: &dyn RealToComplex<f32>,
    input_buf: &mut [f32],
    output_buf: &mut [realfft::num_complex::Complex<f32>],
    scratch: &mut [realfft::num_complex::Complex<f32>],
) -> WindowOutcome {
    // Replace NaN unvoiced cents with the per-window mean of the finite
    // samples — keeps the FFT input finite without injecting a bias
    // toward zero. If every sample is NaN we skip the window entirely.
    let Some(mut window) = fill_window(raw) else {
        return WindowOutcome::Skipped;
    };

    // Median-filter to derive the "intended" pitch baseline; subtract
    // to get the vibrato residual. The median is robust to slow pitch
    // drift across the window without being thrown off by a vibrato
    // cycle inside the kernel (research §5.2).
    let baseline = median_filter(&window, kernel);
    for (slot, b) in window.iter_mut().zip(baseline.iter()) {
        *slot -= *b;
    }

    // Apply a Hann window before zero-padding. This kills DC + low-
    // frequency leakage from the rectangular truncation, which would
    // otherwise smear a small amount of residual carrier energy into
    // the 4–7 Hz vibrato band and cause the single-bin peak to
    // underestimate the true amplitude. The coherent-gain correction
    // (sum(hann)/N) factors back out below.
    let coherent_gain = apply_hann(&mut window);

    // Zero-pad into the FFT input buffer and FFT.
    for slot in input_buf.iter_mut() {
        *slot = 0.0;
    }
    for (slot, &v) in input_buf.iter_mut().zip(window.iter()) {
        *slot = v;
    }
    if r2c
        .process_with_scratch(input_buf, output_buf, scratch)
        .is_err()
    {
        return WindowOutcome::Skipped;
    }

    // Magnitude spectrum.
    let mags: Vec<f32> = output_buf
        .iter()
        .map(|c| (c.re * c.re + c.im * c.im).sqrt())
        .collect();

    // Peak in the 4–7 Hz band.
    let band_lo_idx = (VIBRATO_BAND_LOW_HZ / bin_width_hz).floor() as usize;
    let band_hi_idx = ((VIBRATO_BAND_HIGH_HZ / bin_width_hz).ceil() as usize).min(mags.len() - 1);
    let (peak_idx, peak_mag) = peak_in_band(&mags, band_lo_idx, band_hi_idx);

    // Convert peak magnitude to a cents extent (peak-to-zero of the
    // time-domain residual). For an N-point real-input FFT of a
    // windowed sinusoid with amplitude A, the magnitude at the matching
    // bin is approximately `A * N * coherent_gain / 2`; so
    // `A ≈ 2 * peak_mag / (N * coherent_gain)`. For a Hann window
    // `coherent_gain ≈ 0.5`, recovering the rectangular formula
    // (`A ≈ 2 * peak_mag / N`) when no window is applied.
    //
    // We additionally apply parabolic interpolation across the
    // (peak-1, peak, peak+1) triplet to recover inter-bin amplitude —
    // standard FFT peak-correction technique (Smith, "Spectral Audio
    // Signal Processing", §A.3).
    let interp_peak = parabolic_peak(&mags, peak_idx);
    let denom = window_len as f32 * coherent_gain;
    let extent_cents = if denom > 0.0 {
        2.0 * interp_peak / denom
    } else {
        0.0
    };

    if extent_cents < EXTENT_FLOOR_CENTS {
        return WindowOutcome::BelowFloor;
    }

    // Confidence: peak / mean of 1–10 Hz band, normalised then clamped.
    let conf_lo = (CONFIDENCE_BAND_LOW_HZ / bin_width_hz).floor() as usize;
    let conf_hi = ((CONFIDENCE_BAND_HIGH_HZ / bin_width_hz).ceil() as usize).min(mags.len() - 1);
    let confidence = confidence_score(&mags, conf_lo, conf_hi, peak_mag);
    let rate_hz = peak_idx as f32 * bin_width_hz;

    WindowOutcome::Detected(VibratoWindow {
        start_frame: u32::try_from(start).unwrap_or(u32::MAX),
        rate_hz,
        extent_cents,
        confidence_0_to_1: confidence,
    })
}

fn fill_window(raw: &[f32]) -> Option<Vec<f32>> {
    let mut sum = 0.0_f32;
    let mut finite_count: u32 = 0;
    for &v in raw {
        if v.is_finite() {
            sum += v;
            finite_count += 1;
        }
    }
    if finite_count == 0 {
        return None;
    }
    let fill = sum / finite_count as f32;
    Some(
        raw.iter()
            .map(|v| if v.is_finite() { *v } else { fill })
            .collect(),
    )
}

/// Apply a Hann window in place and return its coherent gain
/// (`sum(hann) / N`). For Hann this converges to ~0.5 for any reasonable
/// `n`. Single-pass — no auxiliary allocation.
fn apply_hann(samples: &mut [f32]) -> f32 {
    let n = samples.len();
    if n < 2 {
        return 1.0;
    }
    let denom = (n - 1) as f32;
    let mut sum = 0.0_f32;
    for (i, slot) in samples.iter_mut().enumerate() {
        let w = 0.5 - 0.5 * (core::f32::consts::TAU * i as f32 / denom).cos();
        sum += w;
        *slot *= w;
    }
    sum / n as f32
}

/// Parabolic peak interpolation across `mags[k-1], mags[k], mags[k+1]`.
///
/// For a sinusoid whose true frequency falls between FFT bins, the
/// single-bin peak underestimates amplitude by up to ~22%. Fitting a
/// parabola through the three-sample neighbourhood around the peak
/// recovers the inter-bin amplitude. Standard correction technique
/// (Smith, "Spectral Audio Signal Processing", §A.3).
///
/// Falls back to the raw peak when the neighbourhood is degenerate
/// (peak at edge or flat triplet).
fn parabolic_peak(mags: &[f32], k: usize) -> f32 {
    if k == 0 || k + 1 >= mags.len() {
        return mags.get(k).copied().unwrap_or(0.0);
    }
    let alpha = mags[k - 1];
    let beta = mags[k];
    let gamma = mags[k + 1];
    let denom = alpha - 2.0 * beta + gamma;
    if !denom.is_finite() || denom.abs() < f32::EPSILON {
        return beta;
    }
    let p = 0.5 * (alpha - gamma) / denom;
    if !p.is_finite() {
        return beta;
    }
    // Corrected peak amplitude (parabola apex value).
    let corrected = beta - 0.25 * (alpha - gamma) * p;
    if corrected.is_finite() && corrected >= beta {
        corrected
    } else {
        beta
    }
}

fn peak_in_band(mags: &[f32], lo: usize, hi: usize) -> (usize, f32) {
    let mut peak_idx = lo;
    let mut peak_mag = 0.0_f32;
    for (i, &m) in mags.iter().enumerate().take(hi + 1).skip(lo) {
        if m > peak_mag {
            peak_mag = m;
            peak_idx = i;
        }
    }
    (peak_idx, peak_mag)
}

fn confidence_score(mags: &[f32], lo: usize, hi: usize, peak_mag: f32) -> f32 {
    let mut conf_sum = 0.0_f32;
    let mut conf_count: u32 = 0;
    for &m in mags.iter().take(hi + 1).skip(lo) {
        conf_sum += m;
        conf_count += 1;
    }
    let conf_mean = if conf_count > 0 {
        conf_sum / conf_count as f32
    } else {
        0.0
    };
    let raw_confidence = if conf_mean > 0.0 {
        peak_mag / conf_mean
    } else {
        0.0
    };
    // Normalise by a representative ceiling: a clean sinusoid with
    // exactly one peak inside the confidence band yields a peak/mean
    // ratio approximately equal to the number of bins in the band.
    let normaliser = (hi - lo + 1).max(1) as f32;
    (raw_confidence / normaliser).clamp(0.0, 1.0)
}

fn aggregate(per_window: Vec<VibratoWindow>, total_windows: u32) -> VibratoReport {
    let detected = u32::try_from(per_window.len()).unwrap_or(u32::MAX);
    let vibrato_ratio = if total_windows == 0 {
        0.0
    } else {
        detected as f32 / total_windows as f32
    };
    let (median_rate_hz, median_extent_cents) = if per_window.is_empty() {
        (0.0, 0.0)
    } else {
        let mut rates: Vec<f32> = per_window.iter().map(|w| w.rate_hz).collect();
        let mut extents: Vec<f32> = per_window.iter().map(|w| w.extent_cents).collect();
        rates.sort_by(f32::total_cmp);
        extents.sort_by(f32::total_cmp);
        (median_of_sorted(&rates), median_of_sorted(&extents))
    };
    VibratoReport {
        per_window,
        median_rate_hz,
        median_extent_cents,
        vibrato_ratio,
    }
}

fn empty_report() -> VibratoReport {
    VibratoReport {
        per_window: Vec::new(),
        median_rate_hz: 0.0,
        median_extent_cents: 0.0,
        vibrato_ratio: 0.0,
    }
}

/// Centred median filter with reflection at the edges. Robust to short
/// vibrato cycles inside the kernel — preferred to a polynomial fit per
/// research §5.2.
fn median_filter(input: &[f32], kernel: usize) -> Vec<f32> {
    if input.is_empty() || kernel < 1 {
        return input.to_vec();
    }
    let half = kernel / 2;
    let n = input.len();
    let mut out = Vec::with_capacity(n);
    let mut buf: Vec<f32> = Vec::with_capacity(kernel);
    for i in 0..n {
        buf.clear();
        for k in 0..kernel {
            let idx = reflect_index(i, k, half, n);
            buf.push(input[idx]);
        }
        buf.sort_by(f32::total_cmp);
        out.push(buf[buf.len() / 2]);
    }
    out
}

/// Compute the reflected sample index for kernel position `k` over a
/// centred window of half-width `half` at sample `i`. Pure usize math —
/// avoids the `usize -> isize` casts clippy flags as wrap-prone.
fn reflect_index(i: usize, k: usize, half: usize, n: usize) -> usize {
    // Conceptually: `signed = i + k - half`. Compute it without going
    // through isize: the only branch where the negative reflection
    // triggers is when `i + k < half`.
    let raw = i + k;
    if raw < half {
        // Negative side reflection: |raw - half|.
        let diff = half - raw;
        diff.min(n.saturating_sub(1))
    } else {
        let signed = raw - half;
        if signed >= n {
            // Reflect around the upper boundary: mirror = (n-1) - over,
            // where over = signed - (n - 1).
            let last = n.saturating_sub(1);
            let over = signed.saturating_sub(last);
            last.saturating_sub(over)
        } else {
            signed
        }
    }
}

fn median_of_sorted(sorted: &[f32]) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let n = sorted.len();
    if n.is_multiple_of(2) {
        0.5 * (sorted[n / 2 - 1] + sorted[n / 2])
    } else {
        sorted[n / 2]
    }
}
