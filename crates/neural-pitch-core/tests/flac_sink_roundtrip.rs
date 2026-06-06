#![allow(missing_docs)]
#![cfg(feature = "flac")]

//! Tier-1 fidelity test for [`FlacRecordingSink`] (Phase 2.0).
//!
//! Generate 1 s of 440 Hz at 48 kHz f32, write through `FlacRecordingSink`,
//! finalize, decode the resulting FLAC with `claxon`, and assert:
//!
//! - the decoded sample count is exactly 48 000 (1 s @ 48 kHz, mono);
//! - the median YIN-detected f0 over the decoded buffer is within ±1 cent
//!   of 440 Hz.
//!
//! Covers the FLAC fidelity contract: 48 kHz / 24-bit / mono / FLAC
//! must round-trip through `FlacRecordingSink` without measurable pitch
//! drift.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    dead_code,
    unused_imports
)]

use std::f32::consts::TAU;
use std::path::PathBuf;

use claxon::FlacReader;
use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingArtifact, RecordingSink};
use neural_pitch_core::pitch::factory::{Backend, make_estimator};
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint, PitchEstimator};

const SAMPLE_RATE_HZ: u32 = 48_000;
const FREQ_HZ: f32 = 440.0;
const DURATION_SECS: f32 = 1.0;
const HOP: usize = 512;
const WINDOW: usize = 2048;

/// Synthesize a unit-amplitude 440 Hz sine at 48 kHz, peak-scaled to 0.95
/// to stay clear of the 24-bit saturation clamp inside the sink.
fn synth_sine(freq_hz: f32, sample_rate_hz: u32, duration_secs: f32) -> Vec<f32> {
    let total = (f64::from(sample_rate_hz) * f64::from(duration_secs)).round() as usize;
    let mut out = Vec::with_capacity(total);
    let dt = 1.0 / sample_rate_hz as f32;
    for n in 0..total {
        let t = n as f32 * dt;
        out.push(0.95 * (TAU * freq_hz * t).sin());
    }
    out
}

/// Decode a 24-bit mono FLAC into a normalised `Vec<f32>` in `[-1.0, 1.0]`.
fn decode_flac(path: &std::path::Path) -> (u32, u16, Vec<f32>) {
    let mut reader = FlacReader::open(path).expect("open flac");
    let info = reader.streaminfo();
    let max_val = (1_i32 << (info.bits_per_sample - 1)) as f32;
    let samples: Vec<f32> = reader
        .samples()
        .map(|s| s.expect("decode sample") as f32 / max_val)
        .collect();
    (info.sample_rate, info.channels as u16, samples)
}

#[test]
fn flac_sink_roundtrip_440hz_within_one_cent() {
    // Use a per-test temp file under the cargo target dir to keep the test
    // hermetic without pulling in `tempfile` as a dev-dep.
    let mut path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    path.push("flac_sink_roundtrip.flac");
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    let partial = path.with_extension("flac.partial");
    if partial.exists() {
        let _ = std::fs::remove_file(&partial);
    }

    // Build the sink, write 1 s of 440 Hz, finalize.
    let mut sink = FlacRecordingSink::create(&path, SAMPLE_RATE_HZ).expect("create sink");
    let samples = synth_sine(FREQ_HZ, SAMPLE_RATE_HZ, DURATION_SECS);
    // Write in hop-sized chunks so the sink steady-state path (no realloc)
    // is exercised.
    for chunk in samples.chunks(HOP) {
        sink.write(chunk).expect("write hop");
    }
    let artifact: RecordingArtifact = Box::new(sink).finalize().expect("finalize");

    assert_eq!(
        artifact.sample_rate_hz, SAMPLE_RATE_HZ,
        "artifact must preserve sample rate"
    );
    assert_eq!(
        artifact.sample_count,
        samples.len() as u64,
        "artifact must report every sample written"
    );
    assert!(
        artifact.path.exists(),
        "finalized FLAC must exist on disk: {}",
        artifact.path.display()
    );

    // Decode and assert geometry.
    let (sr, channels, decoded) = decode_flac(&artifact.path);
    assert_eq!(sr, SAMPLE_RATE_HZ, "decoded sample rate must be 48 kHz");
    assert_eq!(channels, 1, "decoded FLAC must be mono");
    assert_eq!(
        decoded.len(),
        samples.len(),
        "round-trip must preserve sample count"
    );

    // Run YIN over the decoded buffer; assert the median f0 is within ±1
    // cent of 440 Hz.
    let est_cfg = EstimatorConfig {
        sample_rate_hz: SAMPLE_RATE_HZ,
        window_size: WINDOW,
        hop_size: HOP,
        fmin_hz: 50.0,
        fmax_hz: 1500.0,
        instrument_hint: Some(InstrumentHint::Generic),
    };
    let mut estimator = make_estimator(Backend::YinMpm, est_cfg, None).expect("estimator");
    let mut f0s: Vec<f32> = Vec::new();
    let mut idx = 0;
    while idx + WINDOW <= decoded.len() {
        let frame = &decoded[idx..idx + WINDOW];
        if let Ok(Some(estimate)) = estimator.process(frame) {
            if estimate.voiced && estimate.f0_hz > 0.0 {
                f0s.push(estimate.f0_hz);
            }
        }
        idx += HOP;
    }
    assert!(
        !f0s.is_empty(),
        "estimator must produce at least one voiced frame"
    );
    f0s.sort_by(|a, b| a.partial_cmp(b).expect("nan in f0 stream"));
    let median = f0s[f0s.len() / 2];
    let cents_error = 1200.0 * (median / FREQ_HZ).log2();
    assert!(
        cents_error.abs() < 1.0,
        "median f0 must be within 1 cent of 440 Hz; got {median} Hz ({cents_error} cents)"
    );
}
