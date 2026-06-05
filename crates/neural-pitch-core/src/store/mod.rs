//! Phase 2.0 persistence layer: `RecordingsLibrary`.
//!
//! Module shape (per the Phase 2.0 persistence spec):
//!
//! * `library.rs` — [`RecordingsLibrary`] (open, insert, list, soft delete,
//!   hard purge, analysis cache).
//! * `model.rs` — [`NewRecording`], [`Recording`], [`RecordingId`],
//!   [`ListFilter`].
//! * `error.rs` — [`StoreError`] (`thiserror`, wired into `error::CoreError`).
//! * `analysis.rs` — `analysis_cache` upsert/get helpers.
//! * `migrations.rs` — `refinery::embed_migrations!` runner.
//! * `migrations/V0001__init.sql` — append-only schema.
//!
//! See ADR-0012 for the architectural contract (one `Arc<Mutex<Connection>>`,
//! WAL + `synchronous = NORMAL`, append-only migrations).

mod analysis;
mod analysis_runtime;
mod error;
mod library;
mod migrations;
mod model;

pub use analysis_runtime::{
    AnalysisError, AnalysisProgress, AnalysisProgressState, AnalysisRow, AnalysisSummary,
    ContourResult, ProgressSink, analyze_recording_blocking, delete_analysis_blocking,
    get_contour_blocking, get_range_report_blocking, get_vibrato_report_blocking,
    list_analyses_blocking,
};
pub use error::StoreError;
pub use library::RecordingsLibrary;
pub use model::{ListFilter, NewRecording, Recording, RecordingId};
