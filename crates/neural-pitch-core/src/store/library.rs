//! `RecordingsLibrary`: SQLite-backed handle for the recordings catalog.
//!
//! Holds an `Arc<parking_lot::Mutex<rusqlite::Connection>>` so multiple Tauri
//! command handlers can share one connection without opening per-call
//! handles. WAL + `synchronous = NORMAL` are set on every fresh handle. All
//! writes are single-row, indexed, and complete in microseconds, so
//! SQLITE_BUSY is structurally impossible with a single serialized
//! connection.
//!
//! `parking_lot::Mutex` is non-poisoning, aligned with the rest
//! of the project (`src-tauri/src/state.rs`). The previous `std::sync::Mutex`
//! exposed a `Poisoned` error variant that is no longer reachable.
//!
//! Threading model: methods on this type are *blocking* (they take the
//! connection mutex synchronously and run SQL on the calling thread). When
//! invoked from an async runtime (e.g. a `#[tauri::command]`), callers MUST
//! wrap the call in `tokio::task::spawn_blocking` so the runtime worker is
//! not parked on disk I/O. The mutex guard is `!Send` only when held across
//! `.await`, which this module never does internally.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::{Mutex, MutexGuard};
use rusqlite::{Connection, OptionalExtension, params};

use super::analysis;
use super::error::StoreError;
use super::migrations;
use super::model::{ListFilter, NewRecording, Recording, RecordingId};

/// Owning handle to the SQLite-backed recordings library.
pub struct RecordingsLibrary {
    conn: Arc<Mutex<Connection>>,
    root: PathBuf,
}

impl RecordingsLibrary {
    /// Open (or create) the SQLite database at `db_path` and run any pending
    /// migrations. The special path `":memory:"` keeps the database hermetic
    /// for tests — the in-memory db lives only as long as the connection.
    pub fn new(db_path: &Path) -> Result<Self, StoreError> {
        let mut conn = if db_path == Path::new(":memory:") {
            Connection::open_in_memory().map_err(StoreError::DbOpen)?
        } else {
            // Best-effort: ensure the parent directory exists. Returning `Io`
            // here gives the caller a clean error rather than a cryptic
            // "unable to open database file" from SQLite.
            if let Some(parent) = db_path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            Connection::open(db_path).map_err(StoreError::DbOpen)?
        };

        // Connection-level pragmas. WAL is per-database (persists across
        // opens for on-disk files); the others are per-connection. Set them
        // on every fresh handle so behaviour is uniform.
        //
        // `journal_mode` is a query pragma — `pragma_update` rejects it. Use
        // `query_row` and discard the row.
        conn.query_row("PRAGMA journal_mode = WAL;", [], |_| Ok(()))
            .map_err(StoreError::DbOpen)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(StoreError::DbOpen)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(StoreError::DbOpen)?;

        migrations::run(&mut conn)?;

        let root = db_path.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            root,
        })
    }

    /// Resolve the audio-file root directory used by `hard_purge`'s unlink
    /// callback. Defaults to the database file's parent (or empty for
    /// `:memory:`).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Insert a new recording row and return its freshly minted UUIDv7.
    ///
    /// `NewRecording` is taken by value because all `String` fields are moved
    /// into `rusqlite::params!` bindings; passing by reference would force
    /// per-field `.clone()` calls inside this function for the same allocation
    /// the caller would otherwise drop after this call.
    #[allow(clippy::needless_pass_by_value)]
    pub fn insert_recording(&self, m: NewRecording) -> Result<RecordingId, StoreError> {
        let id = RecordingId::new_v7();
        let conn = self.lock_conn();

        conn.execute(
            "INSERT INTO recordings (
                id, filename, created_at_unix_ms, duration_ms,
                sample_rate_hz, channels, bit_depth, format,
                a4_hz, instrument_profile, user_label, deleted_at_unix_ms
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)",
            params![
                &id.0[..],
                m.filename,
                m.created_at_unix_ms,
                m.duration_ms,
                m.sample_rate_hz,
                m.channels,
                m.bit_depth,
                m.format,
                m.a4_hz,
                m.instrument_profile,
                m.user_label,
            ],
        )?;

        Ok(id)
    }

    /// List recordings, ordered by `created_at_unix_ms DESC`.
    ///
    /// `ListFilter` is `#[non_exhaustive]`. The match is intentionally
    /// non-exhaustive at the call site so a future variant (e.g.
    /// `DeletedOnly`, `ByInstrument`) can be added without churning the
    /// IPC boundary; callers MUST extend this match in the same revision
    /// that ships the new variant.
    pub fn list_recordings(&self, f: ListFilter) -> Result<Vec<Recording>, StoreError> {
        let conn = self.lock_conn();

        let sql = match f {
            ListFilter::ActiveOnly => {
                "SELECT id, filename, created_at_unix_ms, duration_ms,
                        sample_rate_hz, channels, bit_depth, format,
                        a4_hz, instrument_profile, user_label, deleted_at_unix_ms
                 FROM recordings
                 WHERE deleted_at_unix_ms IS NULL
                 ORDER BY created_at_unix_ms DESC"
            }
            ListFilter::IncludingDeleted => {
                "SELECT id, filename, created_at_unix_ms, duration_ms,
                        sample_rate_hz, channels, bit_depth, format,
                        a4_hz, instrument_profile, user_label, deleted_at_unix_ms
                 FROM recordings
                 ORDER BY created_at_unix_ms DESC"
            }
        };

        let mut stmt = conn.prepare(sql)?;
        let rows = stmt
            .query_map([], |row| {
                let id_blob: Vec<u8> = row.get(0)?;
                let id_bytes: [u8; 16] = id_blob.as_slice().try_into().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        "expected 16-byte UUID id".into(),
                    )
                })?;
                Ok(Recording {
                    id: RecordingId(id_bytes),
                    filename: row.get(1)?,
                    created_at_unix_ms: row.get(2)?,
                    duration_ms: row.get(3)?,
                    sample_rate_hz: row.get(4)?,
                    channels: row.get(5)?,
                    bit_depth: row.get(6)?,
                    format: row.get(7)?,
                    a4_hz: row.get(8)?,
                    instrument_profile: row.get(9)?,
                    user_label: row.get(10)?,
                    deleted_at_unix_ms: row.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Mark a recording as soft-deleted. Idempotent: if the row is already
    /// tombstoned the existing timestamp is preserved.
    pub fn soft_delete(&self, id: RecordingId) -> Result<(), StoreError> {
        let conn = self.lock_conn();
        let now = now_unix_ms()?;
        let updated = conn.execute(
            "UPDATE recordings
             SET deleted_at_unix_ms = ?1
             WHERE id = ?2 AND deleted_at_unix_ms IS NULL",
            params![now, &id.0[..]],
        )?;
        // If `updated == 0`, the row was either already deleted (idempotent
        // win) or the id does not exist. Confirm the row exists before
        // declaring success.
        if updated == 0 {
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
        }
        Ok(())
    }

    /// Hard-purge a recording: delete the row and call `unlink_file` on the
    /// resolved on-disk path. Cascades to `analysis_cache` rows via the FK.
    /// `unlink_file` failures bubble up as [`StoreError::Unlink`] (with the
    /// failing path preserved) so the caller can observe partial-failure
    /// cases (db row gone, file orphaned). Order: read filename → delete row
    /// → unlink file. If the unlink fails the row stays deleted; this
    /// matches the spec's "store stays I/O-policy-agnostic" stance.
    pub fn hard_purge<F>(&self, id: RecordingId, unlink_file: F) -> Result<(), StoreError>
    where
        F: FnOnce(&Path) -> std::io::Result<()>,
    {
        let conn = self.lock_conn();

        let filename: Option<String> = conn
            .query_row(
                "SELECT filename FROM recordings WHERE id = ?1",
                params![&id.0[..]],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        let Some(filename) = filename else {
            return Err(StoreError::NotFound(id));
        };

        conn.execute("DELETE FROM recordings WHERE id = ?1", params![&id.0[..]])?;

        // Drop the lock before invoking the user-supplied callback so a slow
        // FS unlink does not block other writers.
        drop(conn);

        let path = self.root.join(filename);
        if let Err(source) = unlink_file(&path) {
            return Err(StoreError::Unlink { path, source });
        }
        Ok(())
    }

    /// Insert or replace an `analysis_cache` row keyed by
    /// `(recording_id, analyzer_name, analyzer_version)`.
    ///
    /// Returns [`StoreError::NotFound`] if `id` does not refer to an
    /// existing row in `recordings` — without this, the underlying
    /// `INSERT` would raise `SQLITE_CONSTRAINT_FOREIGNKEY` and surface as
    /// an opaque [`StoreError::Sql`].
    pub fn upsert_analysis(
        &self,
        id: RecordingId,
        name: &str,
        version: &str,
        blob: &[u8],
    ) -> Result<(), StoreError> {
        let conn = self.lock_conn();
        analysis::upsert(&conn, id, name, version, blob)
    }

    /// Fetch a previously cached analysis blob, if present.
    pub fn get_analysis(
        &self,
        id: RecordingId,
        name: &str,
        version: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let conn = self.lock_conn();
        analysis::get(&conn, id, name, version)
    }

    /// Fetch metadata for a previously cached analysis row, if present.
    /// Returns `(computed_at_unix_ms, result_format_version)`.
    pub fn get_analysis_meta(
        &self,
        id: RecordingId,
        name: &str,
        version: &str,
    ) -> Result<Option<(i64, i64)>, StoreError> {
        let conn = self.lock_conn();
        analysis::get_meta(&conn, id, name, version)
    }

    /// Enumerate every cached analysis row for one recording.
    ///
    /// Returns `(analyzer_name, analyzer_version, computed_at_unix_ms,
    /// result_format_version)` for every row keyed on the supplied
    /// `recording_id`. Empty `Vec` if the row exists but has no analyses,
    /// or if the row does not exist (lookup is a left-join in spirit).
    pub fn list_analyses(
        &self,
        id: RecordingId,
    ) -> Result<Vec<(String, String, i64, i64)>, StoreError> {
        let conn = self.lock_conn();
        analysis::list(&conn, id)
    }

    /// Drop one cached analysis row keyed on
    /// `(recording_id, analyzer_name, analyzer_version)`. Idempotent — if
    /// no row matches, returns `Ok(())` rather than `NotFound`.
    pub fn delete_analysis(
        &self,
        id: RecordingId,
        name: &str,
        version: &str,
    ) -> Result<(), StoreError> {
        let conn = self.lock_conn();
        analysis::delete(&conn, id, name, version)
    }

    /// Lock the inner connection mutex.
    ///
    /// `parking_lot::Mutex::lock` does not return a `Result` — the mutex is
    /// non-poisoning by design, so this is infallible.
    fn lock_conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock()
    }
}

/// Wall-clock now in Unix milliseconds.
///
/// Returns [`StoreError::Clock`] if the system clock is set before the Unix
/// epoch (an extremely rare condition that historically silently produced
/// `0` ms timestamps and corrupted tombstones / analysis rows). Clipping to
/// `i64::MAX` if the value somehow overflows is preserved as a hard cap;
/// SQLite STRICT INTEGER is signed 64-bit so this is the largest legal
/// value.
pub(super) fn now_unix_ms() -> Result<i64, StoreError> {
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| StoreError::Clock)?;
    Ok(i64::try_from(dur.as_millis()).unwrap_or(i64::MAX))
}
