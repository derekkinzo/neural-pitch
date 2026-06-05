//! Offline analysis pipelines.
//!
//! Phase 2.1 introduces [`contour::ContourAnalyzer`]: an offline FLAC →
//! per-frame F0 contour analyser that buffers an entire recording, runs
//! pYIN with global Viterbi smoothing, and writes the result into the
//! `analysis_cache` schema (Phase 2.0). Live-path code never touches this
//! module; the only callers are Tauri offline-analysis commands.

pub mod contour;

// Phase 2.2 — generic log-domain Viterbi decoder. Gated behind the
// `neural` feature because its only Phase 2.2 consumers are the PESTO and
// CREPE estimators (also `neural`-gated). A future classical backend can
// lift the gate by adding a feature union if it wants HMM smoothing.
#[cfg(feature = "neural")]
pub mod viterbi;

// Phase 2.3 — vocal-range histogram report and vibrato detector. Both ride
// on top of `analysis::contour::ContourResult` and are pure functions; no
// feature gate, because they have no neural / pyin dependency themselves
// (the upstream contour producer is the gated layer).
pub mod range;
pub mod vibrato;
