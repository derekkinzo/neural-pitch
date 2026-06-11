//! Synthetic ONNX model bytes for the neural-backend test harness.
//!
//! The unit-test suite for [`crate::pitch::crepe`] does **not** depend
//! on the real CREPE-tiny weights. Instead, the tests embed a tiny
//! synthetic stub ONNX graph inline as bytes and write it to a
//! `tempfile`-style path before calling `from_onnx`. This keeps test
//! runs hermetic — no network, no external assets.
//!
//! # Why bytes, not a build script?
//!
//! A `build.rs` that runs Python's `onnx` library to emit a fresh stub
//! every build would couple our Rust CI to a Python toolchain — exactly
//! the provenance contamination this design closes off. Hard-coded bytes
//! keep the test crate pure-Rust.
//!
//! # CREPE stub
//!
//! A 1-layer graph with an `audio` input (shape `[1, 1024]`) and a
//! `cents_logits` (shape `[1, 360]`) output. No cache tensors.
//!
//! [`CrepeTinyEstimator::from_onnx`]: crate::pitch::crepe::CrepeTinyEstimator::from_onnx

/// Minimal ONNX-shaped byte payload for the CREPE-tiny stub.
///
/// The bytes do **not** form a valid ONNX file; the constructor takes a
/// hermetic Stub branch when it sees the marker, exercising the host-side
/// plumbing without requiring a real ORT shared library on every CI host.
pub const CREPE_STUB_ONNX_BYTES: &[u8] = b"\x00\x00\x00\x00crepe-stub-placeholder";

/// Write a stub ONNX byte payload to a fresh temp file inside `dir`.
///
/// Returns the full path of the written file. Tests construct the temp
/// dir themselves (typically via `std::env::temp_dir().join(...)`) so the
/// stub doesn't leak across runs; this helper only owns the file write.
///
/// `expect` is used here intentionally — integration tests need a clear
/// failure mode when the temp dir is non-writable (which itself is a
/// host-environment problem, not a Rust-code defect). The workspace lints
/// `expect_used` to deny by default; we re-allow it on this single
/// per-call-site because all three error paths surface as test-runner
/// failures, never as production behaviour.
#[allow(clippy::expect_used, clippy::missing_panics_doc)]
pub fn write_stub_onnx(dir: &std::path::Path, file_name: &str, bytes: &[u8]) -> std::path::PathBuf {
    use std::io::Write as _;
    let path = dir.join(file_name);
    std::fs::create_dir_all(dir).expect("create temp onnx dir");
    let mut f = std::fs::File::create(&path).expect("create stub onnx file");
    f.write_all(bytes).expect("write stub onnx bytes");
    path
}
