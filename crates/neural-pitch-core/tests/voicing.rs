//! Tests for `VoiceActivityGate`. Real day-1 code; runs as part of `cargo test`.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::test_utils::signals::{silence, sine_wave};
use neural_pitch_core::voicing::VoiceActivityGate;

#[test]
fn silence_is_unvoiced() {
    let mut gate = VoiceActivityGate::new(0.01, 3);
    assert!(!gate.is_voiced(&silence(1024)));
    assert!(!gate.is_voiced(&silence(1024)));
}

#[test]
fn loud_sine_is_voiced() {
    let mut gate = VoiceActivityGate::new(0.01, 3);
    let buf = sine_wave(440.0, 48_000, 1024);
    assert!(gate.is_voiced(&buf));
}

#[test]
fn hangover_keeps_gate_open_briefly() {
    let mut gate = VoiceActivityGate::new(0.01, 3);
    let loud = sine_wave(440.0, 48_000, 1024);
    let quiet = silence(1024);
    // Open the gate.
    assert!(gate.is_voiced(&loud));
    // Three quiet chunks should still report voiced because of hangover.
    for i in 0..3 {
        assert!(
            gate.is_voiced(&quiet),
            "hangover frame {i} should still report voiced",
        );
    }
    // Fourth quiet chunk falls past the hangover.
    assert!(!gate.is_voiced(&quiet));
}

#[test]
fn loud_quiet_loud_silent_transitions() {
    let mut gate = VoiceActivityGate::new(0.01, 1);
    let loud = sine_wave(440.0, 48_000, 1024);
    let quiet = silence(1024);
    assert!(gate.is_voiced(&loud));
    assert!(gate.is_voiced(&quiet)); // within hangover
    assert!(gate.is_voiced(&loud));
    // Two consecutive quiets: first within hangover, second past it.
    assert!(gate.is_voiced(&quiet));
    assert!(!gate.is_voiced(&quiet));
}
