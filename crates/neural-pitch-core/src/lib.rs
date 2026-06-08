#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![forbid(unsafe_code)]

//! Pure-Rust core for NeuralPitch: pitch detection traits, music theory math,
//! audio I/O abstractions, contour smoothing, and voice-activity gating.

pub mod analysis;
pub mod audio;
pub mod error;
pub mod models;
pub mod music;
pub mod pipeline;
pub mod pitch;
// Polyphonic transcription surface (Basic Pitch + MIDI export). Gated
// behind `feature = "neural"` because the GREEN implementation depends on
// `ort` and `ndarray`; under `--no-default-features` the module is
// compiled out entirely so the classical-only build stays clean.
#[cfg(feature = "neural")]
pub mod poly;
pub mod prelude;
pub mod settings;
pub mod smoothing;
pub mod store;
pub mod test_utils;
pub mod voicing;
