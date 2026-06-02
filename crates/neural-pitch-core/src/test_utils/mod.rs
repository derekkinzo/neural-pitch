//! Test utilities shared across `neural-pitch-core` integration tests and
//! downstream crates. Day 1 exposes only synthesised signal generators in
//! [`signals`]; later phases will add fixture loaders and tolerance helpers.
//!
//! NOTE: this module is `pub` unconditionally for day-1 simplicity. A future
//! refactor will gate it behind `#[cfg(any(test, feature = "test-utils"))]`
//! so production binaries do not pull in the helpers.

pub mod signals;
