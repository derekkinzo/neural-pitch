//! Test utilities shared across `neural-pitch-core` integration tests and
//! downstream crates. Day 1 exposes only synthesised signal generators in
//! [`signals`]; later phases will add fixture loaders and tolerance helpers.
//!
//! NOTE: this module is `pub` unconditionally for day-1 simplicity. A future
//! refactor will gate it behind `#[cfg(any(test, feature = "test-utils"))]`
//! so production binaries do not pull in the helpers.

pub mod signals;
pub mod voice;

// Phase 2.2 — synthetic ONNX byte payloads for the neural-backend test
// harness. Gated behind `feature = "neural"` because no non-neural
// consumer references the bytes; the gate keeps the surface area
// inspectable in `cargo doc --features neural` without polluting the
// classical-only doc build.
#[cfg(feature = "neural")]
pub mod onnx;
