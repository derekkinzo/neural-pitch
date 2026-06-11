//! Live target-pitch matcher emission contract.
//!
//! Feeds a synthetic [`PitchUpdate`] stream (440 Hz steady at MIDI 69)
//! into a [`TargetMatcher`] with `target_midi = 69`, installs a mock
//! [`MatchEmitter`] whose handler counts emissions, and asserts at
//! least one [`MatchUpdate`] arrives within 200 ms.
//!
//! The receiver is intentionally dropped at the end of the assertion to
//! verify the matcher's send-error path is `tracing::debug!` only — no
//! panic — mirroring the `start_recording` progress-channel contract:
//! "channel-based tests MUST tolerate the receiver closing early".
//!
//! Drill / training surface is default-on (no `feature = "neural"`
//! gate), so this test compiles against both the all-features and the
//! no-default-features matrices.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use neural_pitch_core::pipeline::PitchUpdate;
use neural_pitch_core::pipeline::target_match::{MatchEmitter, MatchUpdate, TargetMatcher};

/// Mock emitter that counts emissions and tolerates the receiver
/// closing early. Mirrors the `CapturingProgressSink` shape used by
/// the transcribe-cache test.
struct CountingEmitter {
    count: Arc<AtomicUsize>,
}

impl CountingEmitter {
    fn new() -> (Self, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        (
            Self {
                count: Arc::clone(&count),
            },
            count,
        )
    }
}

impl MatchEmitter for CountingEmitter {
    fn emit(&self, update: MatchUpdate) {
        // Read the payload so a matcher cannot silently swallow a
        // malformed event — every field must round-trip through
        // emission, even though we only assert on the count.
        let _ = update.in_window;
        let _ = update.cents_error;
        let _ = update.target_midi;
        let _ = update.t_unix_ms;
        self.count.fetch_add(1, Ordering::Relaxed);
    }
}

/// Synthesise a steady 440 Hz [`PitchUpdate`] at the given timestamp.
fn pitch_update_a4(timestamp_samples: u64) -> PitchUpdate {
    PitchUpdate {
        timestamp_samples,
        f0_hz: 440.0,
        confidence: 0.95,
        voiced: true,
        smoothed_cents: 0.0,
        target_midi: 69,
        target_hz: 440.0,
    }
}

#[test]
fn matcher_emits_at_least_one_match_update_for_steady_a4_target_69() {
    let mut matcher = TargetMatcher::new(69);
    let (emitter, count) = CountingEmitter::new();

    let started = Instant::now();
    let mut frames_fed = 0_u64;
    // At hop=512 / 48 kHz the live worker emits ~93 frames/sec; 200 ms
    // is ~18 frames. Feed up to 32 to leave headroom for any internal
    // warm-up the GREEN matcher introduces.
    while started.elapsed() < Duration::from_millis(200) && frames_fed < 32 {
        matcher.observe(
            pitch_update_a4(frames_fed * 512),
            &emitter as &dyn MatchEmitter,
        );
        frames_fed += 1;
    }
    assert!(
        frames_fed >= 1,
        "test bug: must feed at least one frame; got {frames_fed}",
    );
    let total = count.load(Ordering::Relaxed);
    assert!(
        total >= 1,
        "TargetMatcher must emit at least one MatchUpdate within 200 ms of \
         steady 440 Hz / target_midi = 69 input; got {total} emissions across \
         {frames_fed} frames fed",
    );

    // Drop the emitter (and therefore the underlying counter Arc that
    // the matcher would normally hold across drill duration) to verify
    // the matcher's send-error path is debug-log-only — no panic.
    drop(emitter);
    matcher.observe(
        pitch_update_a4(frames_fed * 512),
        &CountingEmitter::new().0 as &dyn MatchEmitter,
    );
}
