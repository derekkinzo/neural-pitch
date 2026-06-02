//! Smoke tests for the YIN/MPM backends.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::pitch::factory::{Backend, make_estimator};
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};
use neural_pitch_core::test_utils::signals::{silence, sine_wave, two_tone, vibrato_signal};

fn default_cfg() -> EstimatorConfig {
    EstimatorConfig {
        sample_rate_hz: 48_000,
        window_size: 2048,
        hop_size: 512,
        fmin_hz: 50.0,
        fmax_hz: 1500.0,
        instrument_hint: Some(InstrumentHint::Voice),
    }
}

fn cents_off(actual_hz: f32, expected_hz: f32) -> f32 {
    1200.0 * (actual_hz / expected_hz).log2()
}

#[test]
fn yin_440hz_clean_within_5_cents() {
    let cfg = default_cfg();
    let mut est = make_estimator(Backend::YinMpm, cfg.clone(), None).expect("construct yin");
    let buf = sine_wave(440.0, cfg.sample_rate_hz, cfg.window_size);
    let frame = est
        .process(&buf)
        .expect("yin should not error on a clean sine")
        .expect("yin should emit a frame for a full window of clean signal");
    assert!(frame.voiced, "clean 440 Hz must be reported as voiced");
    let off = cents_off(frame.f0_hz, 440.0).abs();
    let f0 = frame.f0_hz;
    assert!(off < 5.0, "440 Hz off by {off} cents (got {f0} Hz)");
}

#[test]
fn yin_silence_returns_unvoiced() {
    let cfg = default_cfg();
    let mut est = make_estimator(Backend::YinMpm, cfg.clone(), None).expect("construct yin");
    let buf = silence(cfg.window_size);
    let frame = est
        .process(&buf)
        .expect("silence should not error")
        .expect("estimator must emit a frame even on silence");
    assert!(!frame.voiced, "silence must not be reported as voiced");
}

#[test]
fn yin_vibrato_within_10_cents() {
    let cfg = default_cfg();
    let mut est = make_estimator(Backend::YinMpm, cfg.clone(), None).expect("construct yin");
    // 5 Hz vibrato, +/-50 cents extent — typical singer.
    let buf = vibrato_signal(440.0, 5.0, 50.0, cfg.sample_rate_hz, cfg.window_size);
    let frame = est
        .process(&buf)
        .expect("yin should not error on vibrato")
        .expect("vibrato should emit a frame");
    assert!(frame.voiced, "vibrato signal must be reported as voiced");
    let off = cents_off(frame.f0_hz, 440.0).abs();
    assert!(off < 10.0, "vibrato center off by {off} cents");
}

#[test]
fn yin_two_tone_picks_louder() {
    let cfg = default_cfg();
    let mut est = make_estimator(Backend::YinMpm, cfg.clone(), None).expect("construct yin");
    // 440 Hz fundamental at unit amplitude, plus its third harmonic (1320 Hz)
    // at half amplitude. The composite signal is periodic at 440 Hz, which
    // is what classical YIN returns: the louder tone's period is the
    // dominant period of the mixture. This is the canonical "harmonic
    // mixture, fundamental wins" case.
    let buf = two_tone(440.0, 1320.0, cfg.sample_rate_hz, cfg.window_size);
    let frame = est
        .process(&buf)
        .expect("yin should not error on two-tone")
        .expect("two-tone should emit a frame");
    assert!(frame.voiced);
    let off = cents_off(frame.f0_hz, 440.0).abs();
    let f0 = frame.f0_hz;
    assert!(off < 25.0, "two-tone picked {f0} Hz, off by {off} cents");
}

#[test]
fn yin_low_voice_85hz_within_5_cents() {
    let cfg = EstimatorConfig {
        sample_rate_hz: 48_000,
        window_size: 2048,
        hop_size: 512,
        fmin_hz: 70.0,
        fmax_hz: 400.0,
        instrument_hint: Some(InstrumentHint::Voice),
    };
    let mut est = make_estimator(Backend::YinMpm, cfg.clone(), None).expect("construct yin");
    let buf = sine_wave(85.0, cfg.sample_rate_hz, cfg.window_size);
    let frame = est
        .process(&buf)
        .expect("yin should not error on low sine")
        .expect("low-voice sine should emit a frame");
    assert!(frame.voiced, "85 Hz sine must be voiced");
    let off = cents_off(frame.f0_hz, 85.0).abs();
    let f0 = frame.f0_hz;
    assert!(off < 5.0, "85 Hz off by {off} cents (got {f0} Hz)");
}

#[test]
fn yin_high_voice_1000hz_within_5_cents() {
    let cfg = EstimatorConfig {
        sample_rate_hz: 48_000,
        window_size: 2048,
        hop_size: 512,
        fmin_hz: 200.0,
        fmax_hz: 1500.0,
        instrument_hint: Some(InstrumentHint::Voice),
    };
    let mut est = make_estimator(Backend::YinMpm, cfg.clone(), None).expect("construct yin");
    let buf = sine_wave(1000.0, cfg.sample_rate_hz, cfg.window_size);
    let frame = est
        .process(&buf)
        .expect("yin should not error on high sine")
        .expect("high-voice sine should emit a frame");
    assert!(frame.voiced, "1000 Hz sine must be voiced");
    let off = cents_off(frame.f0_hz, 1000.0).abs();
    let f0 = frame.f0_hz;
    assert!(off < 5.0, "1000 Hz off by {off} cents (got {f0} Hz)");
}

#[test]
fn mpm_440hz_clean_within_5_cents() {
    let cfg = default_cfg();
    let mut est = make_estimator(Backend::Mpm, cfg.clone(), None).expect("construct mpm");
    let buf = sine_wave(440.0, cfg.sample_rate_hz, cfg.window_size);
    let frame = est
        .process(&buf)
        .expect("mpm should not error on a clean sine")
        .expect("mpm should emit a frame for a full window of clean signal");
    assert!(
        frame.voiced,
        "clean 440 Hz must be reported as voiced (MPM)"
    );
    let off = cents_off(frame.f0_hz, 440.0).abs();
    let f0 = frame.f0_hz;
    assert!(off < 5.0, "MPM 440 Hz off by {off} cents (got {f0} Hz)");
}
