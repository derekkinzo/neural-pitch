//! Phase 2.3 — vocal-range histogram report.
//!
//! [`compute_range`] consumes a [`crate::analysis::contour::ContourResult`]
//! and emits a [`RangeReport`] describing the median, comfortable, and full
//! pitch range of the voiced frames in the contour. See the Phase 2.3
//! algorithm memo for the full specification.
//!
//! Public contract:
//!   * `a4_hz` is a parameter (per ADR-0005 — no module-level A4 state).
//!   * Voicing is taken at face value from `contour.frames[i].voiced`; the
//!     analyser does not re-gate against confidence (that would
//!     double-gate with [`crate::voicing::VoiceActivityGate`] upstream).
//!   * Returns [`RangeReport::insufficient`] when fewer than 50 voiced
//!     frames are present (~0.5 s at 93.75 fps).
//!   * `voice_type_hint` is *informational only* (ADR-0008) and may report
//!     multiple overlapping types (e.g. `Some(vec![Tenor, Baritone])`).

use serde::{Deserialize, Serialize};

use crate::analysis::contour::ContourResult;

/// Minimum voiced-frame count below which the report is the
/// [`RangeReport::insufficient`] sentinel. ~0.5 s at the 93.75 fps default
/// frame rate (research §11.1).
const MIN_VOICED_FRAMES: usize = 50;

/// Number of histogram bins. One bin per MIDI semitone, 0..127 inclusive
/// (covers the full GM/MIDI pitch range plus tail).
const NUM_BINS: usize = 128;

/// Coarse Western-classical voice-type buckets used by
/// [`RangeReport::voice_type_hint`].
///
/// Ranges follow New Grove Dictionary of Music with ±1 semitone tolerance.
/// Multiple variants may be returned when the comfortable range overlaps
/// adjacent types — by design we never auto-assign one (ADR-0008).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceType {
    /// Soprano — roughly C4–C6.
    Soprano,
    /// Mezzo-soprano — roughly A3–A5.
    MezzoSoprano,
    /// Alto / contralto — roughly F3–F5.
    Alto,
    /// Tenor — roughly C3–C5.
    Tenor,
    /// Baritone — roughly G2–G4.
    Baritone,
    /// Bass — roughly E2–E4.
    Bass,
}

/// Whole-recording vocal-range histogram report.
///
/// Field order matters for postcard byte-equal round-trips — do not
/// reorder without bumping the analyzer-version cache key (ADR-0012).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RangeReport {
    /// Number of voiced frames considered. Below 50 the report is the
    /// [`RangeReport::insufficient`] sentinel.
    pub voiced_frame_count: u32,
    /// Median MIDI number of the voiced histogram. Zero for the
    /// insufficient-data sentinel.
    pub median_midi: i32,
    /// Frequency in Hertz corresponding to [`Self::median_midi`] under
    /// the caller's `a4_hz` reference.
    pub median_hz: f32,
    /// Lower bound of the comfortable range (1% trim) as a MIDI number.
    pub comfortable_min_midi: i32,
    /// Upper bound of the comfortable range (1% trim) as a MIDI number.
    pub comfortable_max_midi: i32,
    /// Lower bound of the full range (0.1% trim) as a MIDI number.
    pub full_min_midi: i32,
    /// Upper bound of the full range (0.1% trim) as a MIDI number.
    pub full_max_midi: i32,
    /// Voice-type hints (informational; ADR-0008). `None` when the
    /// recording does not contain enough voiced data.
    pub voice_type_hint: Option<Vec<VoiceType>>,
}

impl RangeReport {
    /// Sentinel value returned when fewer than 50 voiced frames are
    /// present in the contour.
    #[must_use]
    pub fn insufficient() -> Self {
        Self {
            voiced_frame_count: 0,
            median_midi: 0,
            median_hz: 0.0,
            comfortable_min_midi: 0,
            comfortable_max_midi: 0,
            full_min_midi: 0,
            full_max_midi: 0,
            voice_type_hint: None,
        }
    }
}

/// Compute the vocal-range report for one contour.
///
/// `a4_hz` is the caller-supplied reference pitch (per ADR-0005 — no
/// module-level state).
#[must_use]
pub fn compute_range(contour: &ContourResult, a4_hz: f32) -> RangeReport {
    if !(a4_hz.is_finite() && a4_hz > 0.0) {
        return RangeReport::insufficient();
    }

    // Step (a) + (b): voiced-frame filter + MIDI conversion.
    //   midi_cents = 1200 * log2(f0/a4) + 6900
    //   midi       = midi_cents / 100
    // We carry the cents form through histogramming so the bin index is a
    // simple round-and-clamp.
    let mut histogram = [0_u32; NUM_BINS];
    let mut voiced_count: u32 = 0;
    for frame in &contour.frames {
        if !frame.voiced {
            continue;
        }
        let f0 = frame.f0_hz;
        if !(f0.is_finite() && f0 > 0.0) {
            continue;
        }
        let midi_cents = 1200.0 * (f0 / a4_hz).log2() + 6900.0;
        if !midi_cents.is_finite() {
            continue;
        }
        let bin = (midi_cents / 100.0).round();
        if !bin.is_finite() {
            continue;
        }
        // Frames whose MIDI bin falls outside the GM 0..=127 window are
        // discarded rather than clamped. Clamping silently lumps a noisy
        // 1 Hz / 30 kHz outlier onto bin 0 / 127 and shifts the
        // comfortable-trim boundary; research §11 specifies the
        // histogram is one-bin-per-MIDI-0..=127 so out-of-range frames
        // do not belong in either edge bin. The early-continue also
        // moves the `voiced_count` increment under the guard so the
        // [`MIN_VOICED_FRAMES`] threshold reflects only frames that
        // actually contributed to the histogram.
        if !(0.0..=((NUM_BINS - 1) as f32)).contains(&bin) {
            continue;
        }
        let bin_idx = bin as usize;
        histogram[bin_idx] = histogram[bin_idx].saturating_add(1);
        voiced_count = voiced_count.saturating_add(1);
    }

    // Step (c): insufficient-voicing sentinel.
    if (voiced_count as usize) < MIN_VOICED_FRAMES {
        let mut sentinel = RangeReport::insufficient();
        // Per the test contract: `RangeReport::insufficient()` reports
        // `voiced_frame_count == 0` regardless of how few voiced frames
        // were observed; the report is opaque ("no answer", not
        // "answer with 30 frames").
        sentinel.voiced_frame_count = 0;
        return sentinel;
    }

    // Step (e): median bin (cumulative crosses 50%).
    let total = voiced_count;
    let median_bin = cumulative_cross(&histogram, total, 0.5);

    // Step (f): comfortable range — lowest bin whose cumulative count
    // *exceeds* 1% of total (low side), highest bin whose reverse
    // cumulative count *exceeds* 1% of total (high side).
    let (comfortable_min_bin, comfortable_max_bin) = trimmed_bounds(&histogram, total, 0.01);
    // Step (g): full range — same shape, 0.1% trim.
    let (full_min_bin, full_max_bin) = trimmed_bounds(&histogram, total, 0.001);

    let median_midi = i32::try_from(median_bin).unwrap_or(i32::MAX);
    let median_hz = midi_to_hz(median_midi, a4_hz);
    let comfortable_min_midi = i32::try_from(comfortable_min_bin).unwrap_or(i32::MAX);
    let comfortable_max_midi = i32::try_from(comfortable_max_bin).unwrap_or(i32::MAX);
    let full_min_midi = i32::try_from(full_min_bin).unwrap_or(i32::MAX);
    let full_max_midi = i32::try_from(full_max_bin).unwrap_or(i32::MAX);

    let voice_type_hint = Some(infer_voice_types(
        comfortable_min_midi,
        comfortable_max_midi,
    ));

    RangeReport {
        voiced_frame_count: voiced_count,
        median_midi,
        median_hz,
        comfortable_min_midi,
        comfortable_max_midi,
        full_min_midi,
        full_max_midi,
        voice_type_hint,
    }
}

/// Find the lowest bin where the cumulative count first crosses
/// `fraction * total`. Returns `0` if `total == 0`.
fn cumulative_cross(histogram: &[u32; NUM_BINS], total: u32, fraction: f32) -> usize {
    if total == 0 {
        return 0;
    }
    let threshold = fraction * total as f32;
    let mut acc: u32 = 0;
    for (i, &count) in histogram.iter().enumerate() {
        acc = acc.saturating_add(count);
        if acc as f32 >= threshold {
            return i;
        }
    }
    NUM_BINS - 1
}

/// Compute the (low, high) bin pair after trimming `fraction` from each
/// tail of the histogram.
///
/// Spec semantics: the low bound is the lowest bin whose cumulative count
/// *exceeds* `fraction * total`; the high bound is the highest bin whose
/// reverse cumulative count *exceeds* `fraction * total`. "Exceeds" (not
/// "≥") matches the research-memo intent: a 1% trim with exactly 1% of
/// frames at the edge cuts those edge frames.
fn trimmed_bounds(histogram: &[u32; NUM_BINS], total: u32, fraction: f32) -> (usize, usize) {
    if total == 0 {
        return (0, 0);
    }
    let threshold = fraction * total as f32;

    let mut low: usize = 0;
    let mut acc: u32 = 0;
    for (i, &count) in histogram.iter().enumerate() {
        acc = acc.saturating_add(count);
        if acc as f32 > threshold {
            low = i;
            break;
        }
    }

    let mut high: usize = NUM_BINS - 1;
    let mut racc: u32 = 0;
    for (i, &count) in histogram.iter().enumerate().rev() {
        racc = racc.saturating_add(count);
        if racc as f32 > threshold {
            high = i;
            break;
        }
    }

    if low > high {
        // Degenerate: entirely flat distribution. Collapse both bounds
        // onto the single populated bin to keep the report internally
        // consistent.
        let mode = histogram
            .iter()
            .enumerate()
            .max_by_key(|&(_, &c)| c)
            .map_or(0, |(i, _)| i);
        return (mode, mode);
    }
    (low, high)
}

/// Convert MIDI number to Hz under the caller's `a4_hz`.
fn midi_to_hz(midi: i32, a4_hz: f32) -> f32 {
    a4_hz * 2.0_f32.powf((midi - 69) as f32 / 12.0)
}

/// New Grove voice-type ranges with ±1 semitone tolerance, applied to the
/// comfortable range. The tolerance is one-sided in each direction (low
/// bound minus 1, high bound plus 1) so a comfortable range that lies just
/// inside a class still triggers the hint.
fn infer_voice_types(comfortable_min_midi: i32, comfortable_max_midi: i32) -> Vec<VoiceType> {
    // (variant, low_midi, high_midi).
    // Soprano   C4–C6 -> 60..=84
    // MezzoS    A3–A5 -> 57..=81
    // Alto      F3–F5 -> 53..=77
    // Tenor     C3–C5 -> 48..=72
    // Baritone  G2–G4 -> 43..=67
    // Bass      E2–E4 -> 40..=64
    let table: &[(VoiceType, i32, i32)] = &[
        (VoiceType::Soprano, 60, 84),
        (VoiceType::MezzoSoprano, 57, 81),
        (VoiceType::Alto, 53, 77),
        (VoiceType::Tenor, 48, 72),
        (VoiceType::Baritone, 43, 67),
        (VoiceType::Bass, 40, 64),
    ];
    let tol: i32 = 1;
    let mut out = Vec::new();
    for (vt, lo, hi) in table {
        let lo_t = lo - tol;
        let hi_t = hi + tol;
        // The comfortable range overlaps this voice type if there is any
        // intersection between [comfortable_min, comfortable_max] and
        // [lo_t, hi_t].
        if comfortable_min_midi <= hi_t && comfortable_max_midi >= lo_t {
            out.push(*vt);
        }
    }
    out
}
