//! Phase 4 — live target-pitch matcher (TDD-RED stub).
//!
//! Sits on the Rust side of the IPC boundary so the per-frame
//! "is the user on pitch?" decision stays on the audio-rate thread. The
//! [`crate::pipeline::DspWorker`] keeps emitting [`PitchUpdate`] frames
//! exactly as today; when a [`TargetMatcher`] is attached it consumes
//! every update and emits a [`MatchUpdate`] decision through whatever
//! [`MatchEmitter`] the shell wires in. The Tauri shell adapts the
//! emitter trait against `tauri::ipc::Channel<MatchUpdate>` so the
//! decision lands directly in the karaoke-ribbon React component without
//! a second JSON serialise hop.
//!
//! Trait surface:
//!
//! - [`MatchEmitter`] — backend-agnostic delivery surface for
//!   [`MatchUpdate`] frames. Mirrors the
//!   [`crate::store::ProgressSink`] / [`crate::pipeline::FrameSink`]
//!   shape so the Tauri shell adapts a `Channel` against it the same
//!   way the existing analysis-progress / pitch-update channels are
//!   adapted. `emit` is best-effort — implementations MUST tolerate a
//!   dropped consumer (the channel-wrapping adapter logs at `debug!`
//!   and continues).
//!
//! Each call to [`TargetMatcher::observe`] computes `cents_error`
//! against the matcher's `target_midi` (against the standard 440 Hz
//! A4 reference), gates on `update.voiced`, stamps `t_unix_ms` from
//! the wall clock, and emits exactly one [`MatchUpdate`].

use std::time::SystemTime;

use crate::music::midi_to_hz;
use crate::pipeline::PitchUpdate;

/// Default ±-cents window the front-end's karaoke ribbon highlights as
/// "in tune". 25 cents matches the JI-vs-12TET worst-case Pythagorean
/// comma residue and gives the singer enough headroom that an honest
/// vibrato does not flicker the indicator.
pub const DEFAULT_IN_WINDOW_CENTS: f32 = 25.0;

/// One pitch-match decision emitted by [`TargetMatcher`].
///
/// `Copy` because the Tauri shell's `Channel::send` adapter takes the
/// payload by value and the live audio-rate path emits one of these per
/// hop (~93 frames/sec at 48 kHz / hop=512). Keeping the struct
/// register-shaped avoids per-frame heap traffic on the hot path.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct MatchUpdate {
    /// `true` when `cents_error.abs() <= in_window_cents` AND the
    /// underlying [`PitchUpdate::voiced`] flag was `true`.
    pub in_window: bool,
    /// Signed deviation in cents from `target_midi` at the configured
    /// `a4_hz`. Range `(-1200.0, 1200.0)`; values outside that window
    /// indicate the user is more than an octave off and the matcher's
    /// smoother has not yet locked.
    pub cents_error: f32,
    /// MIDI note number the matcher was constructed against.
    pub target_midi: i32,
    /// Wall-clock timestamp at which the underlying [`PitchUpdate`] was
    /// observed, in Unix milliseconds.
    pub t_unix_ms: i64,
}

/// Backend-agnostic delivery surface for [`MatchUpdate`] frames.
///
/// The Tauri shell's adapter wraps a `tauri::ipc::Channel<MatchUpdate>`
/// against this trait, mirroring the [`crate::store::ProgressSink`] and
/// [`crate::pipeline::FrameSink`] shapes. Implementations MUST tolerate
/// a dropped consumer; `emit` returns `()` so the matcher never has to
/// branch on a channel error.
pub trait MatchEmitter: Send + Sync {
    /// Deliver one match decision. Implementations log at `debug!` and
    /// continue if the underlying transport is closed.
    fn emit(&self, update: MatchUpdate);
}

/// Live target-pitch matcher. Constructed by `start_drill` once the
/// drill session's expected response note is known; fed every
/// [`PitchUpdate`] the [`crate::pipeline::DspWorker`] produces.
///
/// # TDD-RED status
///
/// [`Self::observe`] is a no-op today; tests expecting at least one
/// emission therefore fail at runtime.
#[derive(Debug)]
pub struct TargetMatcher {
    target_midi: i32,
    in_window_cents: f32,
}

impl TargetMatcher {
    /// Construct a matcher against `target_midi` with the
    /// [`DEFAULT_IN_WINDOW_CENTS`] tolerance.
    #[must_use]
    pub fn new(target_midi: i32) -> Self {
        Self {
            target_midi,
            in_window_cents: DEFAULT_IN_WINDOW_CENTS,
        }
    }

    /// Override the in-window tolerance. `cents` is clamped to
    /// `(0.0, 100.0]` — anything wider than a semitone defeats the
    /// matcher's purpose.
    #[must_use]
    pub fn with_in_window_cents(mut self, cents: f32) -> Self {
        if cents.is_finite() && cents > 0.0 {
            self.in_window_cents = cents.min(100.0);
        }
        self
    }

    /// MIDI note this matcher was constructed against.
    #[must_use]
    pub fn target_midi(&self) -> i32 {
        self.target_midi
    }

    /// In-window tolerance in cents.
    #[must_use]
    pub fn in_window_cents(&self) -> f32 {
        self.in_window_cents
    }

    /// Observe one [`PitchUpdate`] and emit a [`MatchUpdate`] through
    /// `emitter`. Computes `cents_error` against the matcher's
    /// `target_midi` (440 Hz A4 reference) and stamps `t_unix_ms` from
    /// the wall clock. Unvoiced frames still emit so consumers can
    /// drive a "no signal" indicator from the same channel; the
    /// `in_window` flag is `false` for unvoiced frames regardless of
    /// the cents-error magnitude.
    pub fn observe(&mut self, update: PitchUpdate, emitter: &dyn MatchEmitter) {
        const A4_HZ: f32 = 440.0;
        let target_hz = midi_to_hz(self.target_midi, A4_HZ);
        let cents_error = if update.voiced && update.f0_hz > 0.0 && target_hz > 0.0 {
            100.0 * 12.0 * (update.f0_hz / target_hz).log2()
        } else {
            0.0
        };
        let in_window = update.voiced && cents_error.abs() <= self.in_window_cents;
        let t_unix_ms = i64::try_from(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        )
        .unwrap_or(i64::MAX);
        emitter.emit(MatchUpdate {
            in_window,
            cents_error,
            target_midi: self.target_midi,
            t_unix_ms,
        });
    }
}
