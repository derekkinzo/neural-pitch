//! `analysis_cache` helpers used by `RecordingsLibrary::{upsert,get}_analysis`.
//!
//! The cache is keyed on `(recording_id, analyzer_name, analyzer_version)`
//! with `ON CONFLICT REPLACE` semantics so a re-run of the same analyzer at
//! the same version simply overwrites its previous blob — but a different
//! version is a brand-new row, not an overwrite.

use rusqlite::{Connection, OptionalExtension, params};

use super::error::StoreError;
use super::library::now_unix_ms;
use super::model::RecordingId;

/// Insert or replace one analyzer-result blob.
///
/// Returns [`StoreError::NotFound`] if the supplied `recording_id` does not
/// map to an existing row in `recordings`. The underlying SQLite engine
/// would surface a `SQLITE_CONSTRAINT_FOREIGNKEY` for this case; we
/// detect it via an explicit existence check before the insert so callers
/// see a typed `NotFound` rather than an opaque [`StoreError::Sql`]. The
/// existence check and the insert run inside the caller-held connection
/// lock, so the row cannot be deleted between the two statements.
///
/// `stem_kind` round-trips as SQL `NULL` when `None` so pre-V0003 rows
/// keep their existing key intact; per-stem callers pass
/// `Some("vocals" | "drums" | "bass" | "other")`. The PRIMARY KEY on
/// `analysis_cache` remains the legacy three-tuple in SQL; the new
/// logical four-tuple is enforced at the application layer by always
/// matching the same `stem_kind` on read and write.
pub(super) fn upsert(
    conn: &Connection,
    id: RecordingId,
    name: &str,
    version: &str,
    stem_kind: Option<&str>,
    blob: &[u8],
) -> Result<(), StoreError> {
    // Pre-flight FK check: the connection lock is held by the caller, so
    // the existence verdict is stable across the subsequent INSERT.
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM recordings WHERE id = ?1",
            params![&id.0[..]],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !exists {
        return Err(StoreError::NotFound(id));
    }

    let now = now_unix_ms()?;

    // `result_format_version` defaults to 1; analyzers bump it independently
    // of `analyzer_version` when their on-the-wire blob shape changes without
    // changing the analyzer logic.
    conn.execute(
        "INSERT INTO analysis_cache (
             recording_id, analyzer_name, analyzer_version,
             computed_at_unix_ms, result_format_version, result_blob, stem_kind
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(recording_id, analyzer_name, analyzer_version) DO UPDATE SET
             computed_at_unix_ms = excluded.computed_at_unix_ms,
             result_format_version = excluded.result_format_version,
             result_blob = excluded.result_blob,
             stem_kind = excluded.stem_kind",
        params![&id.0[..], name, version, now, 1_i64, blob, stem_kind],
    )?;
    Ok(())
}

/// Fetch one analyzer-result blob, if present.
///
/// `stem_kind` is matched exactly: `None` selects rows where the column
/// is SQL NULL; `Some(slug)` selects rows where the column equals the
/// supplied slug. This mirrors the application-layer four-tuple key
/// described on [`upsert`].
pub(super) fn get(
    conn: &Connection,
    id: RecordingId,
    name: &str,
    version: &str,
    stem_kind: Option<&str>,
) -> Result<Option<Vec<u8>>, StoreError> {
    let blob: Option<Vec<u8>> = match stem_kind {
        Some(slug) => conn
            .query_row(
                "SELECT result_blob FROM analysis_cache
                 WHERE recording_id = ?1 AND analyzer_name = ?2
                   AND analyzer_version = ?3 AND stem_kind = ?4",
                params![&id.0[..], name, version, slug],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?,
        None => conn
            .query_row(
                "SELECT result_blob FROM analysis_cache
                 WHERE recording_id = ?1 AND analyzer_name = ?2
                   AND analyzer_version = ?3 AND stem_kind IS NULL",
                params![&id.0[..], name, version],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?,
    };
    Ok(blob)
}

/// Fetch the metadata `(computed_at_unix_ms, result_format_version)` of
/// one analyzer row, if present. Used by the cached-path of
/// `analyze_recording_blocking` so it can avoid materialising the full
/// blob just to recompute the wire summary.
pub(super) fn get_meta(
    conn: &Connection,
    id: RecordingId,
    name: &str,
    version: &str,
    stem_kind: Option<&str>,
) -> Result<Option<(i64, i64)>, StoreError> {
    let row: Option<(i64, i64)> = match stem_kind {
        Some(slug) => conn
            .query_row(
                "SELECT computed_at_unix_ms, result_format_version FROM analysis_cache
                 WHERE recording_id = ?1 AND analyzer_name = ?2
                   AND analyzer_version = ?3 AND stem_kind = ?4",
                params![&id.0[..], name, version, slug],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?,
        None => conn
            .query_row(
                "SELECT computed_at_unix_ms, result_format_version FROM analysis_cache
                 WHERE recording_id = ?1 AND analyzer_name = ?2
                   AND analyzer_version = ?3 AND stem_kind IS NULL",
                params![&id.0[..], name, version],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?,
    };
    Ok(row)
}

/// Enumerate every cached analysis row for one recording. Result rows
/// carry `(analyzer_name, analyzer_version, computed_at_unix_ms,
/// result_format_version)`. Ordered by `computed_at_unix_ms DESC` so the
/// recording-list UI's "available analyses" picker shows the most recent
/// row first.
pub(super) fn list(
    conn: &Connection,
    id: RecordingId,
) -> Result<Vec<(String, String, i64, i64)>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT analyzer_name, analyzer_version,
                computed_at_unix_ms, result_format_version
         FROM analysis_cache
         WHERE recording_id = ?1
         ORDER BY computed_at_unix_ms DESC",
    )?;
    let rows = stmt
        .query_map(params![&id.0[..]], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Drop one analyzer-result row. Idempotent: deleting a non-existent
/// row is `Ok(())`. Mirrors `soft_delete`'s "missing row is not an
/// error" stance for the cache layer.
pub(super) fn delete(
    conn: &Connection,
    id: RecordingId,
    name: &str,
    version: &str,
) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM analysis_cache
         WHERE recording_id = ?1 AND analyzer_name = ?2 AND analyzer_version = ?3",
        params![&id.0[..], name, version],
    )?;
    Ok(())
}
