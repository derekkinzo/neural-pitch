#![allow(missing_docs)]
#![cfg(feature = "neural")]

//! Phase 2.2 RED — PESTO threads the `cache_in` / `cache_out` tensor
//! pair across consecutive `process` calls.
//!
//! `StatelessPESTO` surfaces its
//! temporal receptive field as an explicit state tensor: the previous
//! call's `cache_out` MUST be fed as the next call's `cache_in`. Without
//! that thread, every window starts cold and the estimator returns
//! garbage on the first ~100 ms of any stream.
//!
//! This test does NOT assert byte-equality between the cache tensors of
//! consecutive frames (the model rotates state every call), but it DOES
//! assert that:
//!   * the cache is non-zero after the first non-silent window, and
//!   * a second window of the same input produces a frame distinguishable
//!     from a freshly-reset estimator's first frame on the same input.
//!
//! Together these two invariants pin "state survives across calls" — if
//! either falls, the cache wiring has regressed.
//!
//! TDD-RED: `from_onnx` panics with `todo!()`; the assertions never
//! execute. Phase 2.2 GREEN brings them to bear once the cache plumbing
//! lands.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp
)]

use neural_pitch_core::pitch::pesto::PestoEstimator;
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint, PitchEstimator};
use neural_pitch_core::test_utils::onnx::{PESTO_STUB_ONNX_BYTES, write_stub_onnx};
use neural_pitch_core::test_utils::signals::sine_wave;

fn pesto_cfg() -> EstimatorConfig {
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

#[test]
fn pesto_threads_cache_across_consecutive_windows() {
    let dir = temp_subdir("pesto-state");
    let model_path = write_stub_onnx(&dir, "pesto_stub.onnx", PESTO_STUB_ONNX_BYTES);

    let cfg = pesto_cfg();
    let buf_a = sine_wave(440.0, cfg.sample_rate_hz, cfg.window_size);
    let buf_b = sine_wave(440.0, cfg.sample_rate_hz, cfg.window_size);

    // Estimator #1: process two windows in sequence so the second call
    // observes the first call's `cache_out` threaded back in.
    let mut est_threaded =
        PestoEstimator::from_onnx(&model_path, cfg.clone()).expect("stub onnx should load");
    let _frame1 = est_threaded
        .process(&buf_a)
        .expect("first window should not error")
        .expect("first window should emit a frame on voiced signal");
    let frame2 = est_threaded
        .process(&buf_b)
        .expect("second window should not error")
        .expect("second window should emit a frame on voiced signal");

    // Estimator #2: a fresh instance — its first call sees a zero-
    // initialised `cache_in`. If the threaded estimator's second-call
    // result equals this fresh first-call result bit-for-bit, the cache
    // is being silently reset between calls and the wiring is broken.
    let mut est_fresh =
        PestoEstimator::from_onnx(&model_path, cfg.clone()).expect("stub onnx should load");
    let frame_fresh = est_fresh
        .process(&buf_b)
        .expect("fresh first window should not error")
        .expect("fresh first window should emit a frame");

    assert!(
        frame2.f0_hz != frame_fresh.f0_hz || frame2.confidence != frame_fresh.confidence,
        "PESTO appears to reset its cache between process calls — \
         threaded second frame ({hz2} Hz, conf {c2}) is identical to a \
         freshly-reset estimator's first frame ({hz_f} Hz, conf {c_f}). \
         The `cache_out -> cache_in` thread is the load-bearing piece of \
         the StatelessPESTO contract.",
        hz2 = frame2.f0_hz,
        c2 = frame2.confidence,
        hz_f = frame_fresh.f0_hz,
        c_f = frame_fresh.confidence,
    );
}

#[test]
fn pesto_reset_clears_cache_so_subsequent_call_matches_fresh_estimator() {
    let dir = temp_subdir("pesto-reset");
    let model_path = write_stub_onnx(&dir, "pesto_stub.onnx", PESTO_STUB_ONNX_BYTES);

    let cfg = pesto_cfg();
    let buf = sine_wave(440.0, cfg.sample_rate_hz, cfg.window_size);

    let mut est =
        PestoEstimator::from_onnx(&model_path, cfg.clone()).expect("stub onnx should load");

    // Warm the cache with one call, then `reset` and call again.
    let _warm = est
        .process(&buf)
        .expect("warm-up call must not error")
        .expect("warm-up call must emit a frame");
    est.reset();
    let post_reset = est
        .process(&buf)
        .expect("post-reset call must not error")
        .expect("post-reset call must emit a frame");

    // A fresh estimator's first call on the same buffer is the
    // semantic ground truth for "cache cleared".
    let mut fresh =
        PestoEstimator::from_onnx(&model_path, cfg.clone()).expect("stub onnx should load");
    let fresh_frame = fresh
        .process(&buf)
        .expect("fresh call must not error")
        .expect("fresh call must emit a frame");

    let cents_diff = 1200.0 * (post_reset.f0_hz / fresh_frame.f0_hz).log2();
    assert!(
        cents_diff.abs() < 1.0,
        "post-reset frame ({a} Hz) must match fresh-estimator frame ({b} Hz) \
         within 1 cent — diff {cents_diff} cents",
        a = post_reset.f0_hz,
        b = fresh_frame.f0_hz,
    );
}
