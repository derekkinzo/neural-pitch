#![allow(missing_docs)]
#![cfg(feature = "neural")]

//! Phase 2.2 RED — PESTO end-to-end with a synthetic stub ONNX.
//!
//! Wires a 960-sample @ 48 kHz 440 Hz sine through
//! [`neural_pitch_core::pitch::pesto::PestoEstimator`] and asserts the
//! recovered f0 lies within 5 cents of 440 Hz. The model is the in-tree
//! synthetic stub from [`neural_pitch_core::test_utils::onnx`] —
//! deterministic by construction — so the test does not depend on the
//! real (LGPL-tainted) PESTO weights, no network, no `models/` directory.
//!
//! TDD-RED: `PestoEstimator::from_onnx` panics with `todo!()` before it
//! ever opens the stub file. Phase 2.2 GREEN replaces the stub bytes with
//! a valid 2-layer ONNX graph and wires the session-load + softmax-argmax
//! path inside `process`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::pitch::pesto::PestoEstimator;
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint, PitchEstimator};
use neural_pitch_core::test_utils::onnx::{PESTO_STUB_ONNX_BYTES, write_stub_onnx};
use neural_pitch_core::test_utils::signals::sine_wave;

fn pesto_cfg() -> EstimatorConfig {
    // PESTO v1's native rate is 48 kHz with a 960-sample window per
    // Constructor-time invariants on
    // `PestoEstimator::from_onnx` mirror that; deviating here would be
    // tested separately.
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
fn pesto_synthetic_440hz_within_5_cents() {
    let dir = temp_subdir("pesto-synth-440");
    let model_path = write_stub_onnx(&dir, "pesto_stub.onnx", PESTO_STUB_ONNX_BYTES);

    let cfg = pesto_cfg();
    let mut est = PestoEstimator::from_onnx(&model_path, cfg.clone())
        .expect("synthetic stub ONNX should load cleanly under the neural feature");

    let buf = sine_wave(440.0, cfg.sample_rate_hz, cfg.window_size);
    let frame = est
        .process(&buf)
        .expect("PESTO must not error on a clean 960-sample sine")
        .expect("PESTO must emit a frame for a full window of voiced signal");

    assert!(
        frame.voiced,
        "440 Hz @ 48 kHz must be reported as voiced by the stub graph"
    );
    let off = cents_off(frame.f0_hz, 440.0).abs();
    assert!(
        off < 5.0,
        "PESTO recovered {hz} Hz, off by {off} cents from the 440 Hz target",
        hz = frame.f0_hz,
    );
}
