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
// Phase 4 prompt synth. The neural-backed inner modules
// (`poly::basic_pitch`, `poly::midi`) are themselves gated behind
// `feature = "neural"` inside `poly/mod.rs` because their GREEN
// implementations depend on `ort` / `ndarray`. The Phase 4 `poly::synth`
// surface is pure-Rust additive synthesis and ships unconditionally so
// the training subsystem builds under `--no-default-features`.
pub mod poly;
pub mod prelude;
pub mod settings;
pub mod smoothing;
// Phase 5 stem-separation surface (HTDemucs ONNX). Gated behind
// `feature = "neural"` because every submodule's GREEN
// implementation depends on `ort`, `rubato`, or a TLS HTTP client.
// Sits parallel to `poly` for the same reason `poly` sits parallel
// to `pitch`: a fundamentally different output shape (four named
// stem buffers) deserves its own surface.
#[cfg(feature = "neural")]
pub mod stems;
pub mod store;
pub mod test_utils;
// Phase 4 ear-training subsystem. Default-on (no feature gate) so the
// classical-only build still ships drills; the neural surface lives
// elsewhere under `feature = "neural"`.
pub mod training;
pub mod voicing;
