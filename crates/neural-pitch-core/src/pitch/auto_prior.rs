//! AutoPrior: running F0-median + hint-intersection range estimator.
//!
//! A running median of recent voiced f0 samples that tightens the
//! second-pass search range without relying on a neural vocal-detection
//! front end — the running-median heuristic is sufficient for the live
//! path.
//!
//! # Lifecycle
//!
//! `AutoPrior` is owned by [`crate::pipeline::DspWorker`]. It lives in
//! `neural-pitch-core` so all pipelines can reuse the same type.
//!
//! Per-iteration contract from the worker:
//!
//! ```text
//! let (fmin, fmax) = auto_prior.current_range();
//! let frame = estimator.process_with_range(&window, fmin, fmax)?;
//! // ... VAD / smoothing / sink ...
//! auto_prior.update(frame);   // post-emit update — see below
//! ```
//!
//! Updating *after* emitting guarantees `current_range` is read off audio
//! strictly older than the frame just produced. This avoids using the same
//! frame's `f0_hz` as a prior for itself.
//!
//! # Hot-path discipline
//!
//! All buffers are allocated in [`AutoPrior::new`]. [`AutoPrior::update`] is
//! `O(1)`; [`AutoPrior::current_range`] is `O(N)` (a single
//! `select_nth_unstable_by` median on a stack-resident scratch slice owned
//! by the struct).
//!
//! # Hint precedence
//!
//! 1. An explicit `hint_range` (set via [`AutoPrior::with_hint`] or
//!    [`AutoPrior::set_hint`]) wins absolutely — `current_range` returns it
//!    verbatim.
//! 2. Otherwise, with fewer than [`AutoPrior::MIN_SAMPLES_FOR_MEDIAN`] voiced
//!    samples, the generic 60–2000 Hz prior is returned.
//! 3. Once enough voiced samples have accumulated, the median is expanded
//!    by ±1.5 octaves and intersected with the configured instrument-hint
//!    range when present (soft clamp; an empty intersection falls back to
//!    the generic range).
//!
//! # Hint vs soft-clamp independence
//!
//! `hint_range` and `soft_clamp` are independent slots; see the impl-block
//! docs on [`AutoPrior::set_hint`] / [`AutoPrior::set_soft_clamp`] for the
//! precedence contract.

use crate::pitch::{F0Frame, InstrumentHint};

/// Generic search range, in Hertz, used before the median has stabilised
/// and as a safe fallback when the auto range collapses to nothing.
///
/// Spans roughly five octaves (B1..B6); broad enough to cover voice,
/// guitar, and most band instruments without manual selection.
pub const GENERIC_RANGE: (f32, f32) = (60.0, 2000.0);

/// Voice-specific range, in Hertz. Lower bound covers low male speaking voice,
/// upper bound covers Soprano top C6 ≈ 1047 Hz with margin.
pub const VOICE_RANGE: (f32, f32) = (75.0, 1100.0);

/// Six-string guitar range, in Hertz. E2 (≈ 82 Hz) up through the high E
/// string at the 12th fret with margin.
pub const GUITAR_RANGE: (f32, f32) = (75.0, 1320.0);

/// Bass guitar range, in Hertz. Low B0 (≈ 31 Hz) on a 5-string with margin
/// up through harmonic content on the high G string.
pub const BASS_RANGE: (f32, f32) = (35.0, 500.0);

/// Acoustic piano range, in Hertz. A0 (27.5 Hz) through C8 (4186 Hz).
pub const PIANO_RANGE: (f32, f32) = (27.5, 4186.0);

/// Violin range, in Hertz. G3 (≈ 196 Hz) through A7 with margin.
pub const VIOLIN_RANGE: (f32, f32) = (196.0, 3520.0);

/// Look up the canonical (fmin, fmax) range for an instrument hint.
///
/// `InstrumentHint::Generic` returns [`GENERIC_RANGE`].
#[must_use]
pub fn range_for_hint(hint: InstrumentHint) -> (f32, f32) {
    match hint {
        InstrumentHint::Voice => VOICE_RANGE,
        InstrumentHint::Guitar => GUITAR_RANGE,
        InstrumentHint::Bass => BASS_RANGE,
        InstrumentHint::Piano => PIANO_RANGE,
        InstrumentHint::Violin => VIOLIN_RANGE,
        InstrumentHint::Generic => GENERIC_RANGE,
    }
}

/// Running F0-median plus instrument-hint-aware search-range estimator.
///
/// See module-level documentation for the lifecycle and hint-precedence
/// rules. Construct via [`AutoPrior::new`] (or [`AutoPrior::default`] for
/// the default 400-sample / 4-second-at-100-Hz capacity), feed every frame
/// through [`AutoPrior::update`], and read [`AutoPrior::current_range`] at
/// the top of every analysis iteration.
#[derive(Debug)]
pub struct AutoPrior {
    /// Ring of recent voiced f0 samples, in Hz. Length is `capacity`; valid
    /// entries are at indices `[head - len .. head)` mod `capacity`.
    ring: Box<[f32]>,
    /// Index of the next slot to write. In `[0, capacity)`.
    head: usize,
    /// Number of valid samples held in the ring. In `[0, capacity]`.
    len: usize,
    /// Total ring capacity.
    capacity: usize,
    /// Optional hint range: when `Some`, [`AutoPrior::current_range`]
    /// returns this verbatim regardless of ring contents.
    hint_range: Option<(f32, f32)>,
    /// The instrument hint used as a *soft clamp* on the auto range when
    /// `hint_range` is `None`. Set by callers that want auto-mode plus a
    /// loose intersection (e.g. "auto-prior, but never wander outside the
    /// voice range"). `None` means no soft clamp.
    soft_clamp: Option<(f32, f32)>,
    /// Pre-allocated scratch buffer used by [`AutoPrior::current_range`] for
    /// median selection. Never reallocated after `new()`.
    scratch: Box<[f32]>,
}

impl AutoPrior {
    /// Default ring capacity. At a 100 Hz frame rate, this is 4 seconds of
    /// voiced audio — long enough that a ±50-cent vibrato (≥ 5 Hz / 20
    /// cycles in 4 s) cannot destabilise the median.
    pub const DEFAULT_CAPACITY: usize = 400;

    /// Minimum voiced-sample count before the median engages. At 100 Hz
    /// this is the first 80 ms of voiced audio — matches MODULAR-PITCH-
    /// RESEARCH §3.1's "0.5–2 ms / second" stage-2 latency budget.
    pub const MIN_SAMPLES_FOR_MEDIAN: usize = 8;

    /// Half-width, in octaves, of the auto-mode expansion around the
    /// running median. ±1.5 octaves clears classic octave-halving (low
    /// voices) and octave-doubling (bright tones) while staying narrower
    /// than the generic 60–2000 Hz / ~5-octave prior.
    pub const EXPANSION_OCTAVES: f32 = 1.5;

    /// Construct a new `AutoPrior` with the given ring capacity.
    ///
    /// `capacity == 0` is clamped up to 1 so the median path can still
    /// degrade gracefully — but in practice callers SHOULD use the default
    /// (400) or a domain-specific value at construction time.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            ring: vec![0.0_f32; capacity].into_boxed_slice(),
            head: 0,
            len: 0,
            capacity,
            hint_range: None,
            soft_clamp: None,
            scratch: vec![0.0_f32; capacity].into_boxed_slice(),
        }
    }

    /// Builder-style hint setter — equivalent to [`AutoPrior::set_hint`]
    /// but consumes and returns `self` for fluent construction.
    #[must_use]
    pub fn with_hint(mut self, hint: InstrumentHint) -> Self {
        self.set_hint(hint);
        self
    }

    /// Pin the active hint range. Subsequent [`AutoPrior::current_range`]
    /// calls return [`range_for_hint(hint)`](range_for_hint) verbatim until
    /// [`AutoPrior::clear_hint`] is invoked.
    ///
    /// `InstrumentHint::Generic` is treated as "no hint" — it clears the
    /// pinned range and re-engages auto-mode. This matches the worker's
    /// `with_instrument_hint(Some(Generic))` semantics.
    ///
    /// **Hint vs soft-clamp independence:** `set_hint` (and `clear_hint`)
    /// leave the configured [`AutoPrior::set_soft_clamp`] state untouched.
    /// The two slots are independent: a pinned hint takes absolute
    /// precedence in [`AutoPrior::current_range`] (the soft clamp is
    /// ignored while a hint is pinned), but the soft clamp is preserved
    /// across hint transitions and re-applies the moment the hint is
    /// cleared. Callers that want to drop both must call
    /// [`AutoPrior::clear_soft_clamp`] explicitly.
    pub fn set_hint(&mut self, hint: InstrumentHint) {
        if matches!(hint, InstrumentHint::Generic) {
            self.hint_range = None;
        } else {
            self.hint_range = Some(range_for_hint(hint));
        }
    }

    /// Configure a *soft clamp* — the auto-mode median range is intersected
    /// with this hint's canonical range, but the median still drives the
    /// returned bounds. Useful for "auto-prior under the assumption it is
    /// a voice" without fully pinning the range.
    ///
    /// When the auto-mode range produces an empty intersection with the
    /// soft clamp, [`AutoPrior::current_range`] falls back to
    /// [`GENERIC_RANGE`].
    ///
    /// Independent of [`AutoPrior::set_hint`]: a pinned hint takes
    /// absolute precedence over the soft clamp in `current_range`, but the
    /// soft clamp is preserved across hint transitions and re-applies the
    /// moment [`AutoPrior::clear_hint`] is called. To drop both, the
    /// caller MUST also call [`AutoPrior::clear_soft_clamp`].
    pub fn set_soft_clamp(&mut self, hint: InstrumentHint) {
        if matches!(hint, InstrumentHint::Generic) {
            self.soft_clamp = None;
        } else {
            self.soft_clamp = Some(range_for_hint(hint));
        }
    }

    /// Drop any pinned hint range and re-engage auto-mode. The ring is
    /// preserved, so toggling back from a hint warms up instantly.
    pub fn clear_hint(&mut self) {
        self.hint_range = None;
    }

    /// Drop any soft clamp without touching the pinned hint range.
    pub fn clear_soft_clamp(&mut self) {
        self.soft_clamp = None;
    }

    /// Returns the configured ring capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the number of voiced samples held in the ring.
    #[must_use]
    pub fn voiced_count(&self) -> usize {
        self.len
    }

    /// Returns `true` iff a hint range is pinned.
    #[must_use]
    pub fn has_hint(&self) -> bool {
        self.hint_range.is_some()
    }

    /// Push one analysis frame into the running window.
    ///
    /// Unvoiced frames, NaN/non-finite samples, and non-positive samples
    /// are silently dropped. `update` is total — it never panics — but it
    /// only records when the frame carries a usable f0.
    pub fn update(&mut self, frame: F0Frame) {
        if !frame.voiced {
            return;
        }
        let f0 = frame.f0_hz;
        if !f0.is_finite() || f0 <= 0.0 {
            return;
        }
        self.ring[self.head] = f0;
        self.head = (self.head + 1) % self.capacity;
        if self.len < self.capacity {
            self.len += 1;
        }
    }

    /// Returns the (fmin, fmax) search range that the next analysis frame
    /// SHOULD use.
    ///
    /// Order of precedence:
    ///
    /// 1. If a `hint_range` is pinned, returns it verbatim.
    /// 2. If fewer than [`Self::MIN_SAMPLES_FOR_MEDIAN`] voiced samples
    ///    have been recorded, returns [`GENERIC_RANGE`].
    /// 3. Otherwise, returns
    ///    `(median * 2^-EXPANSION_OCTAVES, median * 2^+EXPANSION_OCTAVES)`,
    ///    intersected with [`Self::set_soft_clamp`]'s range when present.
    ///    An empty intersection falls back to [`GENERIC_RANGE`].
    ///
    /// Takes `&mut self` because the median selection writes into the
    /// owned scratch slice; this is `O(N)` but allocation-free.
    #[must_use]
    pub fn current_range(&mut self) -> (f32, f32) {
        if let Some(r) = self.hint_range {
            return r;
        }
        if self.len < Self::MIN_SAMPLES_FOR_MEDIAN {
            return GENERIC_RANGE;
        }
        let median = self.median_hz();
        if !median.is_finite() || median <= 0.0 {
            return GENERIC_RANGE;
        }
        let factor = 2.0_f32.powf(Self::EXPANSION_OCTAVES);
        let mut lo = median / factor;
        let mut hi = median * factor;
        if let Some((clamp_lo, clamp_hi)) = self.soft_clamp {
            lo = lo.max(clamp_lo);
            hi = hi.min(clamp_hi);
            if hi <= lo {
                return GENERIC_RANGE;
            }
        }
        if !lo.is_finite() || !hi.is_finite() || lo <= 0.0 || hi <= lo {
            return GENERIC_RANGE;
        }
        (lo, hi)
    }

    /// Drop all recorded samples but keep the configured hint and soft
    /// clamp. The ring's allocated buffers are preserved.
    pub fn reset(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    /// Compute the median of the live ring contents using a
    /// `select_nth_unstable_by` partial sort. `O(N)` in the ring length.
    ///
    /// Caller MUST ensure `self.len > 0`. The scratch buffer is owned by
    /// `self`, so this never allocates.
    fn median_hz(&mut self) -> f32 {
        debug_assert!(self.len > 0, "median_hz called with empty ring");
        // Copy live entries from the ring into the scratch slice.
        let n = self.len;
        // Live entries occupy the most recent `n` writes ending at
        // `head - 1` (mod capacity). When `len < capacity` they live at
        // `[0, len)`; otherwise they wrap. We do not care about their
        // chronological order for a median, so a contiguous copy is fine.
        if n < self.capacity {
            self.scratch[..n].copy_from_slice(&self.ring[..n]);
        } else {
            // Ring is full — every entry in `self.ring` is valid. Order
            // does not matter for the median.
            self.scratch[..n].copy_from_slice(&self.ring[..]);
        }
        let scratch = &mut self.scratch[..n];
        let mid = n / 2;
        // `f32` is not `Ord`; use total-ordering compare. Non-finite
        // samples are filtered in `update`, so total_cmp is well-defined
        // on the contents.
        let (_, mid_val, _) = scratch.select_nth_unstable_by(mid, f32::total_cmp);
        let mid_val = *mid_val;
        if n.is_multiple_of(2) {
            // Even count: average mid_val with the maximum of the lower
            // half. Both `mid` and `mid - 1` indices are valid here.
            // After `select_nth_unstable_by`, `scratch[..mid]` holds
            // the lower half; its max is the (n/2 - 1)-th order
            // statistic.
            // After `select_nth_unstable_by`, `scratch[..mid]` holds the
            // lower half; its max is the (n/2 - 1)-th order statistic. Use
            // the same total-ordering fold pattern as `pitch::yin::pick_mpm_tau`.
            let lower_max = scratch[..mid]
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max);
            if lower_max.is_finite() {
                0.5 * (mid_val + lower_max)
            } else {
                mid_val
            }
        } else {
            mid_val
        }
    }
}

impl Default for AutoPrior {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
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

    fn voiced(hz: f32, ts: u64) -> F0Frame {
        F0Frame {
            f0_hz: hz,
            confidence: 0.9,
            voiced: true,
            timestamp_samples: ts,
        }
    }

    fn unvoiced(ts: u64) -> F0Frame {
        F0Frame {
            f0_hz: 0.0,
            confidence: 0.0,
            voiced: false,
            timestamp_samples: ts,
        }
    }

    #[test]
    fn cold_start_returns_generic_range() {
        let mut p = AutoPrior::new(400);
        assert_eq!(p.current_range(), GENERIC_RANGE);
    }

    #[test]
    fn unvoiced_frames_do_not_advance() {
        let mut p = AutoPrior::new(16);
        for i in 0..32 {
            p.update(unvoiced(i));
        }
        assert_eq!(p.voiced_count(), 0);
        assert_eq!(p.current_range(), GENERIC_RANGE);
    }

    #[test]
    fn nan_and_negative_dropped() {
        let mut p = AutoPrior::new(16);
        p.update(voiced(f32::NAN, 0));
        p.update(voiced(-100.0, 1));
        p.update(voiced(0.0, 2));
        assert_eq!(p.voiced_count(), 0);
    }

    #[test]
    fn hint_overrides_ring() {
        let mut p = AutoPrior::new(16).with_hint(InstrumentHint::Bass);
        // Inject 10 voiced 1000 Hz samples — the auto path would never
        // produce a bass range from these, but the pinned hint must win.
        for i in 0..10 {
            p.update(voiced(1000.0, i));
        }
        assert_eq!(p.current_range(), BASS_RANGE);
    }

    #[test]
    fn clear_hint_re_engages_auto() {
        let mut p = AutoPrior::new(16).with_hint(InstrumentHint::Bass);
        for i in 0..16 {
            p.update(voiced(220.0, i));
        }
        assert_eq!(p.current_range(), BASS_RANGE);
        p.clear_hint();
        let (lo, hi) = p.current_range();
        // Median is 220 Hz, expansion ±1.5 octaves.
        let expected_lo = 220.0_f32 / 2.0_f32.powf(1.5);
        let expected_hi = 220.0_f32 * 2.0_f32.powf(1.5);
        assert!((lo - expected_lo).abs() < 0.5);
        assert!((hi - expected_hi).abs() < 0.5);
    }

    #[test]
    fn generic_hint_does_not_pin() {
        let mut p = AutoPrior::new(16).with_hint(InstrumentHint::Generic);
        // Generic is a non-hint; auto path with cold ring → generic.
        assert_eq!(p.current_range(), GENERIC_RANGE);
    }

    #[test]
    fn soft_clamp_narrows_auto_range() {
        // Configure the Voice soft clamp (75–1100 Hz) and feed enough
        // voiced 220 Hz samples to engage the median path. The auto
        // expansion ±1.5 octaves around 220 yields ≈ (77.78, 622.25);
        // intersected with (75, 1100) the lower bound clamps to 77.78
        // (auto value beats clamp lo) and the upper bound stays at the
        // auto value because 622.25 < 1100.
        let mut p = AutoPrior::new(16);
        p.set_soft_clamp(InstrumentHint::Voice);
        for i in 0..16 {
            p.update(voiced(220.0, i));
        }
        let (lo, hi) = p.current_range();
        let expected_lo = (220.0_f32 / 2.0_f32.powf(1.5)).max(VOICE_RANGE.0);
        let expected_hi = (220.0_f32 * 2.0_f32.powf(1.5)).min(VOICE_RANGE.1);
        assert!(
            (lo - expected_lo).abs() < 0.5,
            "lo {lo} != expected {expected_lo}"
        );
        assert!(
            (hi - expected_hi).abs() < 0.5,
            "hi {hi} != expected {expected_hi}"
        );
    }

    #[test]
    fn soft_clamp_empty_intersection_returns_generic() {
        // Push enough voiced 50 Hz samples to engage the median path.
        // ±1.5-octave expansion gives ≈ (17.68, 141.42); intersected
        // with the Voice clamp (75, 1100) the resulting range is
        // (75, 141.42) — non-empty. To force an empty intersection we
        // configure a Bass clamp (35, 500) but feed extremely high f0
        // (1500 Hz) so the auto range ≈ (530.33, 4242.64) sits entirely
        // above the clamp upper bound (500), producing an empty
        // intersection that MUST fall back to GENERIC_RANGE.
        let mut p = AutoPrior::new(16);
        p.set_soft_clamp(InstrumentHint::Bass);
        for i in 0..16 {
            p.update(voiced(1500.0, i));
        }
        assert_eq!(p.current_range(), GENERIC_RANGE);
    }

    #[test]
    fn hint_overrides_soft_clamp() {
        // With both a pinned Voice hint and a Guitar soft clamp, the
        // pinned hint MUST win verbatim. The soft clamp is preserved
        // (rule: independent slots) but is ignored at read time while
        // a hint is pinned.
        let mut p = AutoPrior::new(16).with_hint(InstrumentHint::Voice);
        p.set_soft_clamp(InstrumentHint::Guitar);
        for i in 0..16 {
            p.update(voiced(220.0, i));
        }
        assert_eq!(p.current_range(), VOICE_RANGE);
        // Clearing the hint must re-engage auto-mode AND keep the
        // Guitar soft clamp active — the slot was not touched by
        // set_hint / clear_hint.
        p.clear_hint();
        let (lo, hi) = p.current_range();
        let auto_lo = 220.0_f32 / 2.0_f32.powf(1.5);
        let auto_hi = 220.0_f32 * 2.0_f32.powf(1.5);
        let expected_lo = auto_lo.max(GUITAR_RANGE.0);
        let expected_hi = auto_hi.min(GUITAR_RANGE.1);
        assert!(
            (lo - expected_lo).abs() < 0.5,
            "lo {lo} != expected {expected_lo} (soft clamp survived clear_hint?)"
        );
        assert!(
            (hi - expected_hi).abs() < 0.5,
            "hi {hi} != expected {expected_hi} (soft clamp survived clear_hint?)"
        );
    }

    #[test]
    fn clear_soft_clamp_drops_only_clamp() {
        // set_soft_clamp + clear_soft_clamp returns the auto-mode range
        // to its un-clamped form without disturbing any pinned hint.
        let mut p = AutoPrior::new(16);
        p.set_soft_clamp(InstrumentHint::Voice);
        for i in 0..16 {
            p.update(voiced(220.0, i));
        }
        let _clamped = p.current_range();
        p.clear_soft_clamp();
        let (lo, hi) = p.current_range();
        let expected_lo = 220.0_f32 / 2.0_f32.powf(1.5);
        let expected_hi = 220.0_f32 * 2.0_f32.powf(1.5);
        assert!((lo - expected_lo).abs() < 0.5);
        assert!((hi - expected_hi).abs() < 0.5);
    }
}
