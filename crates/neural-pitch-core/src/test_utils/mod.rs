//! Test utilities shared across `neural-pitch-core` integration tests and
//! downstream crates. Exposes synthesised signal generators in [`signals`]
//! and the voice-shaped fixture builder in [`voice`].
//!
//! NOTE: this module is `pub` unconditionally; gating it behind
//! `#[cfg(any(test, feature = "test-utils"))]` is left as a follow-up so
//! production binaries do not pull in the helpers.

pub mod signals;
pub mod voice;

// Synthetic ONNX byte payloads for the neural-backend test harness.
// Gated behind `feature = "neural"` because no non-neural consumer
// references the bytes; the gate keeps the surface area inspectable
// in `cargo doc --features neural` without polluting the
// classical-only doc build.
#[cfg(feature = "neural")]
pub mod onnx;
