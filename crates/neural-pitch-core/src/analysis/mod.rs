//! Offline analysis pipelines.
//!
//! Phase 2.1 introduces [`contour::ContourAnalyzer`]: an offline FLAC →
//! per-frame F0 contour analyser that buffers an entire recording, runs
//! pYIN with global Viterbi smoothing, and writes the result into the
//! `analysis_cache` schema (Phase 2.0). Live-path code never touches this
//! module; the only callers are Tauri offline-analysis commands.

pub mod contour;
