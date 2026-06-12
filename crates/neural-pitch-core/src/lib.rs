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
// Polyphonic transcription surface (Basic Pitch + MIDI export) plus the
// pure-Rust prompt synth. Inner modules apply their own `feature = "neural"`
// gates — see `poly/mod.rs`.
pub mod poly;
pub mod prelude;
pub mod settings;
pub mod smoothing;
// HTDemucs ONNX stem-separation surface — gated on `feature = "neural"`.
#[cfg(feature = "neural")]
pub mod stems;
pub mod store;
pub mod test_utils;
// Ear-training subsystem — feature-gate-free; ships in every build
// configuration.
pub mod training;
pub mod voicing;
