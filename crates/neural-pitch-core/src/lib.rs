#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

//! Pure-Rust core for NeuralPitch: pitch detection traits, music theory math,
//! audio I/O abstractions, contour smoothing, and voice-activity gating.

pub mod audio;
pub mod error;
pub mod music;
pub mod pitch;
pub mod prelude;
pub mod smoothing;
pub mod test_utils;
pub mod voicing;
