//! Error surface for the `store` module.
//!
//! All variants flow through `?` into `error::CoreError::Store` via `#[from]`
//! at the crate root, so callers in `src-tauri/` and the IPC boundary do not
//! need to match on `StoreError` directly unless they want fine-grained
//! handling.
//!
//! `#[non_exhaustive]` is applied so additional variants are not breaking
//! changes to downstream `match` statements (the IPC boundary exhaustively
//! pattern-matches errors; a new variant must not break that).

use std::path::PathBuf;

use thiserror::Error;

use super::model::RecordingId;

/// Persistence error surface for the `store` module.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    /// I/O failure (file unlink, directory create, etc.).
    ///
    /// Surfaced via `?` from any `std::io::Result`. For `hard_purge`'s
    /// post-DELETE unlink the typed [`StoreError::Unlink`] variant is used
    /// instead so operators can recover the failing path.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// `hard_purge` deleted the row but failed to unlink the on-disk audio
    /// file. The path is preserved so operators triaging "orphaned file"
    /// incidents do not have to reverse-engineer which file the unlink
    /// targeted. Distinct from [`StoreError::Io`] because the row is gone
    /// from the catalog by the time this surfaces.
    #[error("unlink {path}: {source}")]
    Unlink {
        /// Filesystem path that the unlink call targeted.
        path: PathBuf,
        /// Underlying I/O failure.
        #[source]
        source: std::io::Error,
    },

    /// Failed to open the SQLite database. Distinct from [`StoreError::Sql`]
    /// so callers can disambiguate open-time failures (bad path, lock
    /// contention, missing parent dir) from query-time failures on an
    /// already-open handle. The variant is constructed via
    /// `.map_err(StoreError::DbOpen)` rather than `#[from]` so it does not
    /// accidentally swallow query-time errors at the trait boundary.
    #[error("db open: {0}")]
    DbOpen(rusqlite::Error),

    /// A migration step failed. Refinery's `Error` does not overlap with
    /// `rusqlite::Error`'s `#[from]`, so the auto-conversion is safe.
    #[error("migration: {0}")]
    Migration(#[from] refinery::Error),

    /// A SQL statement failed at query time.
    ///
    /// This is a catch-all for `rusqlite::Error`. Constraint violations
    /// (UNIQUE, CHECK, FOREIGN KEY) are wrapped in
    /// [`StoreError::ConstraintViolation`] before they reach this variant
    /// when the call site can disambiguate them; otherwise the raw error is
    /// preserved for debugging.
    ///
    /// Callers needing fine-grained handling SHOULD inspect the inner
    /// `rusqlite::Error::SqliteFailure { code, .. }` (e.g.
    /// `SQLITE_CONSTRAINT_FOREIGNKEY`).
    #[error("sql: {0}")]
    Sql(#[from] rusqlite::Error),

    /// A SQL constraint (UNIQUE / CHECK / FOREIGN KEY) was violated.
    ///
    /// Produced by `upsert_analysis` when the supplied `recording_id`
    /// does not exist (foreign-key violation) so callers see `NotFound`
    /// instead of an opaque `Sql` error. The variant remains generic
    /// for additional constraint-violation surfaces.
    #[error("constraint violation: {0}")]
    ConstraintViolation(String),

    /// The requested recording id is not present in the `recordings` table.
    #[error("recording {0} not found")]
    NotFound(RecordingId),

    /// Stored blob format version does not match the caller's expected
    /// version. Both fields are `i64` to mirror the schema column type
    /// (SQLite STRICT INTEGER is signed 64-bit).
    #[error("blob format v{found} != expected v{expected}")]
    FormatVersionMismatch {
        /// The format version the caller expected.
        expected: i64,
        /// The format version actually present in the stored blob.
        found: i64,
    },

    /// The system clock was set before the Unix epoch when generating a
    /// timestamp. Should be vanishingly rare; surfaced as an explicit error
    /// so callers can distinguish a clock-skew condition from a "0 ms"
    /// timestamp value.
    #[error("system clock before unix epoch")]
    Clock,
}
