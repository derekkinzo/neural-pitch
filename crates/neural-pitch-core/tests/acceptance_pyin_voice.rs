#![allow(missing_docs)]
#![cfg(feature = "pyin")]

//! pYIN offline acceptance over the voice fixtures (Tier-2).
//!
//! Runs `analyze_contour` (pYIN backend) over every FLAC fixture under
//! `tests/fixtures/voice/`, computes the median voiced F0 per fixture
//! against the MIDI binding from `MANIFEST.toml`, and asserts ≥ 95% of
//! fixtures land within 5 cents of truth (the Tier-2 acceptance gate).
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(clippy::print_stdout)]

use std::path::{Path, PathBuf};

use claxon::FlacReader;
use neural_pitch_core::analysis::contour::{ContourResult, analyze_contour};
use neural_pitch_core::music::midi_to_hz;
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};

const MANIFEST_TEXT: &str = include_str!("fixtures/voice/MANIFEST.toml");

#[derive(Clone, Debug)]
struct FixtureSpec {
    filename: String,
    expected_midi: i32,
}

fn load_manifest() -> Vec<FixtureSpec> {
    let parsed: toml::Value = toml::from_str(MANIFEST_TEXT).expect("parse MANIFEST.toml");
    let array = parsed
        .get("fixture")
        .and_then(toml::Value::as_array)
        .expect("MANIFEST.toml: missing [[fixture]] array");
    let mut out = Vec::with_capacity(array.len());
    for entry in array {
        let table = entry.as_table().expect("fixture entry must be a table");
        let filename = table
            .get("filename")
            .and_then(toml::Value::as_str)
            .expect("fixture.filename")
            .to_owned();
        let expected_midi = i32::try_from(
            table
                .get("expected_midi")
                .and_then(toml::Value::as_integer)
                .expect("fixture.expected_midi"),
        )
        .expect("expected_midi fits in i32");
        out.push(FixtureSpec {
            filename,
            expected_midi,
        });
    }
    assert!(
        !out.is_empty(),
        "MANIFEST.toml parsed but contains no [[fixture]] entries"
    );
    out
}

fn decode_flac(path: &Path) -> (u32, Vec<f32>) {
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

fn median_voiced_hz(result: &ContourResult) -> Option<f32> {
    let mut hz: Vec<f32> = result
        .frames
        .iter()
        .filter(|f| f.voiced && f.f0_hz.is_finite() && f.f0_hz > 0.0)
        .map(|f| f.f0_hz)
        .collect();
    if hz.is_empty() {
        return None;
    }
    hz.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = hz.len();
    let median = if n.is_multiple_of(2) {
        0.5 * (hz[n / 2 - 1] + hz[n / 2])
    } else {
        hz[n / 2]
    };
    Some(median)
}

fn cents_off(actual_hz: f32, expected_hz: f32) -> f32 {
    1200.0 * (actual_hz / expected_hz).log2()
}

#[test]
fn acceptance_pyin_voice_tier_2() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let voice_root = PathBuf::from(manifest_dir)
        .join("tests")
        .join("fixtures")
        .join("voice");

    let fixtures = load_manifest();
    let total = fixtures.len();
    println!(
        "[pyin-acceptance] running pYIN Tier-2 acceptance over {total} synthetic voice fixtures"
    );

    let mut passed = 0_usize;
    let mut per_fixture: Vec<(String, i32, Option<f32>, f32, bool)> = Vec::with_capacity(total);

    for spec in &fixtures {
        let path = voice_root.join(&spec.filename);
        let (sr, samples) = decode_flac(&path);
        assert_eq!(
            sr, 48_000,
            "fixture {} has sample rate {} (expected 48000)",
            spec.filename, sr
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
            .expect("analyze_contour should succeed on a synthetic voice fixture");

        let truth_hz = midi_to_hz(spec.expected_midi, 440.0);
        let median_hz = median_voiced_hz(&result);
        let cents = match median_hz {
            Some(hz) => cents_off(hz, truth_hz).abs(),
            None => f32::INFINITY,
        };
        let pass = cents.is_finite() && cents < 5.0;
        if pass {
            passed += 1;
        }
        println!(
            "[pyin-acceptance] {}: expected_midi={}, median_hz={:?}, |Δcents|={:5.2}  {}",
            spec.filename,
            spec.expected_midi,
            median_hz,
            cents,
            if pass { "PASS" } else { "FAIL" }
        );
        per_fixture.push((
            spec.filename.clone(),
            spec.expected_midi,
            median_hz,
            cents,
            pass,
        ));
    }

    let pass_rate = passed as f32 / total as f32;
    println!(
        "[pyin-acceptance] aggregate: {passed}/{total} = {:.1}% (≥ 95% required)",
        pass_rate * 100.0
    );

    assert!(
        pass_rate >= 0.95,
        "pYIN Tier-2 voice acceptance failed: {passed}/{total} = {:.1}% < 95%",
        pass_rate * 100.0
    );
}
