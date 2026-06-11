//! Offline analysis pipelines.
//!
//! [`contour::ContourAnalyzer`] is an offline FLAC → per-frame F0 contour
//! analyser that buffers an entire recording, runs pYIN with global
//! Viterbi smoothing, and writes the result into the `analysis_cache`
//! schema. Live-path code never touches this module; the only callers
//! are Tauri offline-analysis commands.

pub mod contour;

// Generic log-domain Viterbi decoder. Gated behind the `neural` feature
// because its only consumer today is the CREPE estimator (also
// `neural`-gated). A future classical backend can lift the gate by
// adding a feature union if it wants HMM smoothing.
#[cfg(feature = "neural")]
pub mod viterbi;

// Vocal-range histogram report and vibrato detector. Both ride on top
// of `analysis::contour::ContourResult` and are pure functions; no
// feature gate, because they have no neural / pyin dependency
// themselves (the upstream contour producer is the gated layer).
pub mod range;
pub mod vibrato;
