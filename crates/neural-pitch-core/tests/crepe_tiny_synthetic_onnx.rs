#![allow(missing_docs)]
#![cfg(feature = "neural")]

//! CREPE-tiny end-to-end with a synthetic stub ONNX.
//!
//! Pushes a 960-sample @ 48 kHz 440 Hz sine through
//! [`CrepeTinyEstimator::process`] and asserts the recovered f0 lies
//! within 5 cents of 440 Hz. Notable properties:
//!   * stateless graph — no `cache_in` / `cache_out` tensors,
//!   * native rate is 16 kHz with a 1024-sample window, so the estimator
//!     must resample the 48 kHz capture audio internally (rubato) and
//!     keep the work alloc-free per the hot-path contract.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::pitch::crepe::CrepeTinyEstimator;
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint, PitchEstimator};
use neural_pitch_core::test_utils::onnx::{CREPE_STUB_ONNX_BYTES, write_stub_onnx};
use neural_pitch_core::test_utils::signals::sine_wave;

fn crepe_cfg() -> EstimatorConfig {
    // CREPE-tiny is a 1024-sample @ 16 kHz model. The estimator
    // accepts 48 kHz capture audio (window_size = 960) and resamples
    // internally to the model's native rate.
    EstimatorConfig {
        sample_rate_hz: 48_000,
        window_size: 960,
        hop_size: 960,
        fmin_hz: 50.0,
        fmax_hz: 1500.0,
        instrument_hint: Some(InstrumentHint::Voice),
    }
}

fn temp_subdir(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "neural-pitch-test-{tag}-{pid}",
        pid = std::process::id()
    ));
    p
}

fn cents_off(actual_hz: f32, expected_hz: f32) -> f32 {
    1200.0 * (actual_hz / expected_hz).log2()
}

#[test]
fn crepe_tiny_synthetic_440hz_within_5_cents() {
    let dir = temp_subdir("crepe-synth-440");
    let model_path = write_stub_onnx(&dir, "crepe_stub.onnx", CREPE_STUB_ONNX_BYTES);

    let cfg = crepe_cfg();
    let mut est = CrepeTinyEstimator::from_onnx(&model_path, cfg.clone())
        .expect("synthetic stub ONNX should load cleanly under the neural feature");

    let buf = sine_wave(440.0, cfg.sample_rate_hz, cfg.window_size);
    let frame = est
        .process(&buf)
        .expect("CREPE-tiny must not error on a clean 960-sample sine")
        .expect("CREPE-tiny must emit a frame for a full window of voiced signal");

    assert!(
        frame.voiced,
        "440 Hz @ 48 kHz must be reported as voiced by the stub graph"
    );
    let off = cents_off(frame.f0_hz, 440.0).abs();
    assert!(
        off < 5.0,
        "CREPE-tiny recovered {hz} Hz, off by {off} cents from the 440 Hz target",
        hz = frame.f0_hz,
    );
}

#[test]
fn crepe_tiny_is_stateless_so_consecutive_calls_are_deterministic() {
    // CREPE has no cache tensor — a second call on the same input MUST
    // produce the same f0 as the first. This pins the "stateless"
    // contract.
    let dir = temp_subdir("crepe-stateless");
    let model_path = write_stub_onnx(&dir, "crepe_stub.onnx", CREPE_STUB_ONNX_BYTES);

    let cfg = crepe_cfg();
    let mut est =
        CrepeTinyEstimator::from_onnx(&model_path, cfg.clone()).expect("stub onnx should load");

    let buf = sine_wave(440.0, cfg.sample_rate_hz, cfg.window_size);
    let frame1 = est
        .process(&buf)
        .expect("first call must not error")
        .expect("first call must emit a frame");
    let frame2 = est
        .process(&buf)
        .expect("second call must not error")
        .expect("second call must emit a frame");

    let cents_diff = 1200.0 * (frame1.f0_hz / frame2.f0_hz).log2();
    assert!(
        cents_diff.abs() < 0.01,
        "CREPE-tiny is stateless — consecutive identical inputs must \
         produce identical f0; got {a} Hz then {b} Hz (diff {cents_diff} cents)",
        a = frame1.f0_hz,
        b = frame2.f0_hz,
    );
}
