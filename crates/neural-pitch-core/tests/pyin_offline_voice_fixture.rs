#![allow(missing_docs)]
#![cfg(feature = "pyin")]

//! pYIN end-to-end through a real FLAC fixture.
//!
//! Decode `069_A4_synthvoice_clean.flac` via `claxon`, run
//! `analyze_contour`, and assert the median voiced F0 is within 5 cents
//! of 440 Hz.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use claxon::FlacReader;
use neural_pitch_core::analysis::contour::{ContourResult, analyze_contour};
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};

fn cents_off(actual_hz: f32, expected_hz: f32) -> f32 {
    1200.0 * (actual_hz / expected_hz).log2()
}

/// Decode a FLAC fixture into normalised mono `f32` PCM in `[-1.0, 1.0]`.
/// Mirrors the helper in `acceptance_voice.rs` so the fixtures are read
/// identically across the two harnesses.
fn decode_flac(path: &std::path::Path) -> (u32, Vec<f32>) {
    let mut reader = FlacReader::open(path).expect("open flac fixture");
    let info = reader.streaminfo();
    let bits = info.bits_per_sample;
    assert!(
        (16..=24).contains(&bits),
        "fixture {} has unexpected bits_per_sample={}",
        path.display(),
        bits
    );
    let max_val = (1_i32 << (bits - 1)) as f32;
    let samples: Vec<f32> = reader
        .samples()
        .map(|s| s.expect("decode sample") as f32 / max_val)
        .collect();
    (info.sample_rate, samples)
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
        "no voiced frames on a clean A4 voice fixture — analyzer is broken"
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
fn pyin_offline_a4_voice_fixture_within_5_cents() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = PathBuf::from(manifest_dir)
        .join("tests")
        .join("fixtures")
        .join("voice")
        .join("069_A4_synthvoice_clean.flac");
    let (sr, samples) = decode_flac(&path);
    assert_eq!(
        sr, 48_000,
        "fixture sample rate is {sr} (expected 48000) — analyzer cfg is hard-coded for 48 kHz"
    );

    let cfg = EstimatorConfig {
        sample_rate_hz: sr,
        window_size: 4096,
        hop_size: 1024,
        fmin_hz: 60.0,
        fmax_hz: 1100.0,
        instrument_hint: Some(InstrumentHint::Voice),
    };

    let result = analyze_contour(&samples, &cfg, 440.0)
        .expect("analyze_contour should succeed on a real synth-voice fixture");

    let med = median_hz_voiced(&result);
    let off = cents_off(med, 440.0).abs();
    assert!(
        off < 5.0,
        "voice-fixture median voiced F0 = {med} Hz, off by {off} cents (>= 5)"
    );

    // Frame rate should be `sr / hop` to a few floats of precision. At
    // 48 kHz / 1024-sample hop that's 46.875 Hz exactly.
    let expected_frame_rate_hz = (sr as f32) / (cfg.hop_size as f32);
    assert!(
        (result.frame_rate_hz - expected_frame_rate_hz).abs() < 0.01,
        "frame_rate_hz = {} (expected ~{expected_frame_rate_hz})",
        result.frame_rate_hz
    );
}
