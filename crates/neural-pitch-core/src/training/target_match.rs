//! Real-time target-matching scorer for the karaoke ribbon and tuning
//! drills.
//!
//! Consumes [`PitchUpdate`] frames from the DSP worker, computes
//! signed cents error against the target MIDI (not the nearest note —
//! the worker's `smoothed_cents` is relative to the nearest note, which
//! is the wrong reference for drill scoring), and emits aggregate
//! [`MatchUpdate`]s at a configurable output rate.
//!
//! Allocation policy: a fixed-capacity `VecDeque` ring is preallocated
//! in [`TargetMatcher::new`]; no allocations occur on the hot path.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use super::drill::HitWindow;
use crate::music::midi_to_hz;
use crate::pipeline::sink::PitchUpdate;

/// Aggregate scoring summary emitted at the matcher's output rate.
///
/// All ratios fall in `[0.0, 1.0]`. When the ring contains no voiced
/// frames every float is `0.0` and the consumer treats it as "not
/// started yet".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MatchUpdate {
    /// Fraction of voiced frames scored in-tune:
    /// `count(in_tune) / count(voiced)`.
    pub in_tune_fraction: f32,
    /// Mean signed cents error across voiced frames.
    pub mean_cents_error_signed: f32,
    /// Mean absolute cents error across voiced frames.
    pub mean_cents_error_abs: f32,
    /// Fraction of *all* frames (voiced + unvoiced) on-pitch:
    /// `count(voiced && in_tune) / total_frames`.
    pub time_on_pitch_ratio: f32,
    /// Total frames currently in the ring.
    pub frames_observed: u32,
}

/// One captured frame in the ring. `None` cents mean unvoiced — kept
/// so the time axis stays aligned but excluded from voiced aggregates.
#[derive(Debug, Clone, Copy)]
struct MatchSample {
    /// `Some((signed_cents, in_tune))` when voiced, `None` when unvoiced.
    voiced: Option<(f32, bool)>,
}

/// Real-time scoring engine. One instance per active drill prompt /
/// karaoke segment.
#[derive(Debug)]
pub struct TargetMatcher {
    window: HitWindow,
    ring: VecDeque<MatchSample>,
    capacity: usize,
    /// Frames between [`MatchUpdate`] emits.
    emit_period_frames: u32,
    /// Frames pushed since last emit boundary.
    frames_since_emit: u32,
}

impl TargetMatcher {
    /// Construct a matcher for the supplied hit-window and emit rate
    /// (the worker's frame-emit rate, ~93 Hz at 22.05 kHz / 256-sample
    /// hop). The internal ring is sized for the default 1-second window.
    #[must_use]
    pub fn new(window: HitWindow, emit_rate_hz: f32) -> Self {
        Self::with_params(window, emit_rate_hz, 1_000, 10.0)
    }

    /// Construct a matcher with explicit ring-window-ms and output-rate
    /// parameters. Used by unit tests that drive deterministic frame
    /// counts.
    #[must_use]
    pub fn with_params(
        window: HitWindow,
        emit_rate_hz: f32,
        ring_window_ms: u32,
        output_hz: f32,
    ) -> Self {
        let emit_rate = emit_rate_hz.max(1.0);
        let output = output_hz.max(0.000_001);
        let capacity_f = emit_rate * (ring_window_ms as f32 / 1000.0);
        let capacity = capacity_f.ceil().max(1.0) as usize;
        let emit_period_frames = ((emit_rate / output).ceil().max(1.0)) as u32;
        Self {
            window,
            ring: VecDeque::with_capacity(capacity),
            capacity,
            emit_period_frames,
            frames_since_emit: 0,
        }
    }

    /// Push one [`PitchUpdate`] frame. Returns `Some(MatchUpdate)` on
    /// the cadence-driven emit boundary; `None` between emits — the
    /// `None` branch is the no-emit boundary case and is fine to
    /// ignore. `Some(_)` is the score payload and MUST be observed
    /// (hence `#[must_use]`).
    ///
    /// `target_midi` and `a4_hz` parameterise the cents-error reference
    /// — cents are computed relative to `midi_to_hz(target_midi, a4_hz)`,
    /// not the nearest note. Treats `a4_hz <= 0.0` or `f0_hz <= 0.0` as
    /// unvoiced so a garbage f0 cannot fabricate a perfect-cents
    /// reading; only frames that pass the f0 / a4 sanity check are
    /// scored against the hit-window.
    #[must_use]
    pub fn push(
        &mut self,
        update: PitchUpdate,
        target_midi: i32,
        a4_hz: f32,
    ) -> Option<MatchUpdate> {
        // Slide scoring isn't implemented; the matcher treats the pair
        // as a `[min, max]` envelope and scores cents against a single
        // `target_midi`. In debug builds, surface the envelope-vs-point
        // contract loudly so a future slide-aware caller does not get
        // silently wrong scores. Production builds do not assert.
        debug_assert!(
            self.window.start_midi <= self.window.end_midi,
            "HitWindow.start_midi must be <= end_midi (was [{}, {}])",
            self.window.start_midi,
            self.window.end_midi,
        );
        let sample = if !update.voiced || update.f0_hz <= 0.0 || a4_hz <= 0.0 {
            MatchSample { voiced: None }
        } else {
            let target_hz = midi_to_hz(target_midi, a4_hz);
            let cents_error = if target_hz > 0.0 {
                100.0 * 12.0 * (update.f0_hz / target_hz).log2()
            } else {
                0.0
            };
            // Continuous MIDI estimate from f0. Both `a4_hz > 0.0` and
            // `update.f0_hz > 0.0` are guaranteed by the early-out
            // above — no fabricated fallback path needed.
            let midi_f = 69.0 + 12.0 * (update.f0_hz / a4_hz).log2();
            let nearest_midi = midi_f.round() as i32;
            let in_window =
                nearest_midi >= self.window.start_midi && nearest_midi <= self.window.end_midi;
            let in_tune = in_window && cents_error.abs() <= self.window.tolerance_cents;
            MatchSample {
                voiced: Some((cents_error, in_tune)),
            }
        };

        if self.ring.len() == self.capacity {
            self.ring.pop_front();
        }
        self.ring.push_back(sample);

        self.frames_since_emit = self.frames_since_emit.saturating_add(1);
        if self.frames_since_emit >= self.emit_period_frames {
            self.frames_since_emit = 0;
            Some(self.snapshot())
        } else {
            None
        }
    }

    /// Force-flush the current ring contents into a [`MatchUpdate`]
    /// without waiting for the next emit boundary. Used at
    /// end-of-prompt; the returned summary is the per-attempt score
    /// the caller persists, hence `#[must_use]`.
    #[must_use]
    pub fn flush(&mut self) -> MatchUpdate {
        self.frames_since_emit = 0;
        self.snapshot()
    }

    /// Compute the current aggregate from the ring contents.
    fn snapshot(&self) -> MatchUpdate {
        let total_frames = self.ring.len() as u32;
        let mut voiced_count: u32 = 0;
        let mut in_tune_count: u32 = 0;
        let mut sum_signed: f32 = 0.0;
        let mut sum_abs: f32 = 0.0;

        for s in &self.ring {
            if let Some((cents, in_tune)) = s.voiced {
                voiced_count += 1;
                sum_signed += cents;
                sum_abs += cents.abs();
                if in_tune {
                    in_tune_count += 1;
                }
            }
        }

        if voiced_count == 0 {
            return MatchUpdate {
                in_tune_fraction: 0.0,
                mean_cents_error_signed: 0.0,
                mean_cents_error_abs: 0.0,
                time_on_pitch_ratio: 0.0,
                frames_observed: total_frames,
            };
        }

        let voiced_f = voiced_count as f32;
        let total_f = total_frames.max(1) as f32;
        MatchUpdate {
            in_tune_fraction: in_tune_count as f32 / voiced_f,
            mean_cents_error_signed: sum_signed / voiced_f,
            mean_cents_error_abs: sum_abs / voiced_f,
            time_on_pitch_ratio: in_tune_count as f32 / total_f,
            frames_observed: total_frames,
        }
    }
}
