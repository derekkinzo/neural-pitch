//! Property tests for the YIN backend.
//!
//! For any sine in the human-vocal-and-soprano range across the two
//! supported sample rates, mixed with white noise at >= 30 dB SNR, the
//! estimator must report `voiced = true` and a fundamental frequency within
//! +/- 20 cents of the input.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::pitch::factory::{Backend, make_estimator};
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};
use neural_pitch_core::test_utils::signals::{mix, sine_wave, white_noise};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::default())]

    /// A clean (>= 30 dB SNR) sine in the supported range produces a voiced
    /// frame within +/- 20 cents of the input.
    #[test]
    fn yin_voiced_within_20_cents_for_clean_sine(
        f in 80.0_f32..=1500.0_f32,
        sr_choice in 0u32..=1u32,
        snr_db in 30.0_f32..=60.0_f32,
    ) {
        let sample_rate_hz = if sr_choice == 0 { 44_100 } else { 48_000 };
        let cfg = EstimatorConfig {
            sample_rate_hz,
            window_size: 2048,
            hop_size: 512,
            fmin_hz: 60.0,
            fmax_hz: 2000.0,
            instrument_hint: Some(InstrumentHint::Generic),
        };
        let mut est = make_estimator(Backend::YinMpm, cfg.clone(), None)
            .expect("construct yin");

        // Build a noisy sine with the requested SNR. signals::mix peak-normalises
        // the result, so RMS is well above the silence gate.
        let pure = sine_wave(f, sample_rate_hz, 4096);
        let noise = white_noise(sample_rate_hz, 4096, 0xC0FFEE);
        let noisy = mix(&pure, &noise, snr_db);

        let frame = est
            .process(&noisy[..cfg.window_size])
            .expect("process must not error on finite input")
            .expect("estimator must emit a frame for a full window");

        prop_assert!(frame.voiced, "noisy sine at {} Hz, {} dB SNR not voiced", f, snr_db);
        let cents = (frame.f0_hz / f).log2() * 1200.0;
        prop_assert!(
            cents.abs() < 20.0,
            "f={} sr={} snr={} got_f0={} off_cents={}",
            f, sample_rate_hz, snr_db, frame.f0_hz, cents,
        );
    }
}
