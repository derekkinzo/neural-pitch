#![allow(missing_docs)]
#![cfg(feature = "pyin")]

//! Clean-tone pYIN sanity test.
//!
//! Synthesise 1 s of 440 Hz at 48 kHz, run `analyze_contour`, and assert
//! the median voiced-frame F0 is within 1 cent of 440 with a high
//! voiced ratio.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::analysis::contour::{ContourResult, analyze_contour};
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};
use neural_pitch_core::test_utils::signals::sine_wave;

fn cents_off(actual_hz: f32, expected_hz: f32) -> f32 {
    1200.0 * (actual_hz / expected_hz).log2()
}

fn median_hz_voiced(result: &ContourResult) -> f32 {
    let mut hz: Vec<f32> = result
        .frames
        .iter()
        .filter(|f| f.voiced && f.f0_hz.is_finite() && f.f0_hz > 0.0)
        .map(|f| f.f0_hz)
        .collect();
    assert!(
        !hz.is_empty(),
        "no voiced frames in 1 s of clean 440 Hz — analyzer is broken"
    );
    hz.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = hz.len();
    if n.is_multiple_of(2) {
        0.5 * (hz[n / 2 - 1] + hz[n / 2])
    } else {
        hz[n / 2]
    }
}

#[test]
fn pyin_clean_440hz_within_1_cent() {
    let cfg = EstimatorConfig {
        sample_rate_hz: 48_000,
        window_size: 4096,
        hop_size: 1024,
        fmin_hz: 60.0,
        fmax_hz: 1100.0,
        instrument_hint: Some(InstrumentHint::Voice),
    };
    // 1 s of clean 440 Hz audio.
    let samples = sine_wave(440.0, cfg.sample_rate_hz, cfg.sample_rate_hz as usize);

    let result = analyze_contour(&samples, &cfg, 440.0)
        .expect("analyze_contour should succeed on a clean sine wave");

    let med = median_hz_voiced(&result);
    let off = cents_off(med, 440.0).abs();
    assert!(
        off < 1.0,
        "median voiced F0 = {med} Hz, off by {off} cents (>= 1)"
    );

    assert!(
        result.voiced_ratio > 0.9,
        "voiced_ratio {} <= 0.9 on clean 440 Hz",
        result.voiced_ratio
    );

    assert_eq!(
        result.source_sample_rate_hz, cfg.sample_rate_hz,
        "source_sample_rate_hz must round-trip the cfg sample rate"
    );
    assert_eq!(
        result.sample_count,
        samples.len() as u64,
        "sample_count must be the input length"
    );
}
