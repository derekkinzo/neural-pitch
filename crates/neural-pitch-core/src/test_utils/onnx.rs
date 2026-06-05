//! Phase 2.2 — synthetic ONNX model bytes for the neural-backend test
//! harness.
//!
//! Per the architecture spec, the Tier-1 test suite for [`crate::pitch::pesto`]
//! and [`crate::pitch::crepe`] **does not depend on the real PESTO or
//! CREPE-tiny weights**. Instead, the tests embed a tiny synthetic stub ONNX
//! graph inline as bytes and write it to a `tempfile`-style path before
//! calling `from_onnx`. This keeps test runs hermetic — no network, no
//! external assets, no LGPL-tainted vendored weights (ADR-0008,
//! `MODULAR-PITCH-RESEARCH.md` §8.1).
//!
//! # Why bytes, not a build script?
//!
//! A `build.rs` that runs Python's `onnx` library to emit a fresh stub
//! every build would couple our Rust CI to a Python toolchain — exactly the
//! provenance contamination ADR-0008 closes off. Hard-coded bytes keep the
//! test crate pure-Rust.
//!
//! # TDD-RED placeholder
//!
//! The byte arrays exposed below are **placeholder stubs** for the TDD-RED
//! phase. The neural estimators they feed into ([`PestoEstimator::from_onnx`]
//! and [`CrepeTinyEstimator::from_onnx`]) currently panic with `todo!()`
//! before they even reach the parse step, so the tests fail at the panic
//! site and the byte payload is never consumed.
//!
//! Phase 2.2 GREEN MUST replace these placeholders with valid ONNX
//! protobuf wire-format bytes encoding:
//!
//! - **PESTO stub**: a 2-layer graph with a `cache_in` (shape
//!   `[1, 1, 64, 32]`) and an `audio` input (shape `[1, 960]`); a
//!   `cache_out` and a `cents_logits` (shape `[1, 384]`) output. The graph
//!   body can be a single `MatMul` whose deterministic weight matrix maps a
//!   440 Hz sine's energy onto a known cents bin.
//! - **CREPE stub**: a 1-layer graph with an `audio` input (shape
//!   `[1, 1024]`) and a `cents_logits` (shape `[1, 360]`) output. No cache
//!   tensors.
//!
//! The replacement bytes can be generated via a one-shot offline script
//! (`scripts/gen_stub_onnx.py`) committed alongside the byte literals so
//! reviewers can regenerate them and diff. The script is **not** part of
//! the build graph — it runs only when the stub graph specification
//! changes.
//!
//! [`PestoEstimator::from_onnx`]: crate::pitch::pesto::PestoEstimator::from_onnx
//! [`CrepeTinyEstimator::from_onnx`]: crate::pitch::crepe::CrepeTinyEstimator::from_onnx

/// Minimal ONNX-shaped byte payload for the PESTO stub.
///
/// Phase 2.2 RED: a placeholder so the integration tests compile and run.
/// The bytes do **not** form a valid ONNX file; the tests fail at the
/// `from_onnx` `todo!()` panic before parsing is attempted. See module
/// docs for the GREEN-phase specification.
pub const PESTO_STUB_ONNX_BYTES: &[u8] = b"\x00\x00\x00\x00pesto-stub-placeholder";

/// Minimal ONNX-shaped byte payload for the CREPE-tiny stub.
///
/// Phase 2.2 RED: a placeholder so the integration tests compile and run.
/// The bytes do **not** form a valid ONNX file; the tests fail at the
/// `from_onnx` `todo!()` panic before parsing is attempted. See module
/// docs for the GREEN-phase specification.
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
