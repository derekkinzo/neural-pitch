//! Test utilities shared across `neural-pitch-core` integration tests and
//! downstream crates. Exposes synthesised signal generators in [`signals`]
//! and the voice-shaped fixture builder in [`voice`].
//!
//! NOTE: this module is `pub` unconditionally — production binaries
//! pay the cost of pulling in the synthesised-signal helpers. Gating
//! it behind `#[cfg(any(test, feature = "test-utils"))]` would shrink
//! the production surface but is not part of the contract.

pub mod signals;
pub mod voice;

// Synthetic ONNX byte payloads for the neural-backend test harness.
// Gated behind `feature = "neural"` because no non-neural consumer
// references the bytes; the gate keeps the surface area inspectable
// in `cargo doc --features neural` without polluting the
// classical-only doc build.
#[cfg(feature = "neural")]
pub mod onnx;
