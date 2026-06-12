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
    /// `ListFilter` is `#[non_exhaustive]`; downstream callers MUST
    /// extend their match in the same revision that adds a new variant
    /// so the IPC boundary stays unchanged when filters such as
    /// `DeletedOnly` or `ByInstrument` are introduced.
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
        analysis::upsert(&conn, id, name, version, None, blob)
    }

    /// Insert or replace an `analysis_cache` row keyed by
    /// `(recording_id, analyzer_name, analyzer_version, stem_kind)`.
    ///
    /// `stem_kind = None` round-trips as SQL NULL and matches every
    /// pre-V0003 row verbatim; `Some(slug)` distinguishes per-stem cache
    /// entries (`vocals` / `drums` / `bass` / `other`). The PRIMARY KEY
    /// on `analysis_cache` is the legacy three-tuple by SQL definition;
    /// the four-tuple is enforced at the application layer by always
    /// passing both keys through this helper.
    pub fn upsert_analysis_for_stem(
        &self,
        id: RecordingId,
        name: &str,
        version: &str,
        stem_kind: Option<&str>,
        blob: &[u8],
    ) -> Result<(), StoreError> {
        let conn = self.lock_conn();
        analysis::upsert(&conn, id, name, version, stem_kind, blob)
    }

    /// Fetch a previously cached analysis blob, if present.
    pub fn get_analysis(
        &self,
        id: RecordingId,
        name: &str,
        version: &str,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let conn = self.lock_conn();
        analysis::get(&conn, id, name, version, None)
    }

    /// Fetch a per-stem cached analysis blob, if present.
    pub fn get_analysis_for_stem(
        &self,
        id: RecordingId,
        name: &str,
        version: &str,
        stem_kind: Option<&str>,
    ) -> Result<Option<Vec<u8>>, StoreError> {
        let conn = self.lock_conn();
        analysis::get(&conn, id, name, version, stem_kind)
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
        analysis::get_meta(&conn, id, name, version, None)
    }

    /// Fetch metadata for a per-stem cached analysis row, if present.
    pub fn get_analysis_meta_for_stem(
        &self,
        id: RecordingId,
        name: &str,
        version: &str,
        stem_kind: Option<&str>,
    ) -> Result<Option<(i64, i64)>, StoreError> {
        let conn = self.lock_conn();
        analysis::get_meta(&conn, id, name, version, stem_kind)
    }

    /// Insert or replace a `stem_results` row keyed on
    /// `(recording_id, separator_version)`. `paths` are the four canonical
    /// on-disk FLAC paths under
    /// `<recordings_dir>/<recording_id>/stems/{vocals,drums,bass,other}.flac`.
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_stem_result(
        &self,
        id: RecordingId,
        separator_version: &str,
        completed_at_unix_ms: i64,
        vocals_path: &str,
        drums_path: &str,
        bass_path: &str,
        other_path: &str,
    ) -> Result<(), StoreError> {
        let conn = self.lock_conn();
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
        conn.execute(
            "INSERT INTO stem_results (
                 recording_id, separator_version, completed_at_unix_ms,
                 vocals_path, drums_path, bass_path, other_path
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(recording_id, separator_version) DO UPDATE SET
                 completed_at_unix_ms = excluded.completed_at_unix_ms,
                 vocals_path          = excluded.vocals_path,
                 drums_path           = excluded.drums_path,
                 bass_path            = excluded.bass_path,
                 other_path           = excluded.other_path",
            params![
                &id.0[..],
                separator_version,
                completed_at_unix_ms,
                vocals_path,
                drums_path,
                bass_path,
                other_path,
            ],
        )?;
        Ok(())
    }

    /// Fetch a previously cached `stem_results` row keyed on
    /// `(recording_id, separator_version)`. Returns the four on-disk FLAC
    /// paths plus the `completed_at_unix_ms` timestamp; `None` if no row
    /// matches.
    pub fn get_stem_result(
        &self,
        id: RecordingId,
        separator_version: &str,
    ) -> Result<Option<StemResultRow>, StoreError> {
        let conn = self.lock_conn();
        let row: Option<StemResultRow> = conn
            .query_row(
                "SELECT completed_at_unix_ms, vocals_path, drums_path, bass_path, other_path
                 FROM stem_results
                 WHERE recording_id = ?1 AND separator_version = ?2",
                params![&id.0[..], separator_version],
                |r| {
                    Ok(StemResultRow {
                        completed_at_unix_ms: r.get(0)?,
                        vocals_path: r.get(1)?,
                        drums_path: r.get(2)?,
                        bass_path: r.get(3)?,
                        other_path: r.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
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

    /// Insert one row into `drill_attempts`. The row's UUIDv7 is minted
    /// here so callers do not have to plumb `uuid::Uuid` themselves;
    /// the returned 16-byte id is the row's primary key.
    ///
    /// `correct` is persisted as `0`/`1` to match the SQLite STRICT
    /// INTEGER schema; `recording_id` may be `None` for live drills
    /// that do not stash an audio recording.
    pub fn insert_drill_attempt(&self, row: &NewDrillAttempt) -> Result<[u8; 16], StoreError> {
        let id = *uuid::Uuid::now_v7().as_bytes();
        let conn = self.lock_conn();
        let recording_blob: Option<Vec<u8>> = row.recording_id.map(|r| r.0.to_vec());
        conn.execute(
            "INSERT INTO drill_attempts (
                 id, drill_kind, drill_payload, correct,
                 mean_cents_error, time_on_pitch_ratio,
                 started_at_unix_ms, finished_at_unix_ms,
                 recording_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                &id[..],
                row.drill_kind,
                row.drill_payload,
                i64::from(row.correct),
                row.mean_cents_error,
                row.time_on_pitch_ratio,
                row.started_at_unix_ms,
                row.finished_at_unix_ms,
                recording_blob,
            ],
        )?;
        Ok(id)
    }

    /// Page over `drill_attempts` ordered by
    /// `(drill_kind, finished_at_unix_ms DESC)`. The supplied `limit`
    /// is treated as-is — clamping to a server-side cap is the
    /// caller's responsibility (the IPC layer enforces
    /// `HISTORY_LIMIT_CAP` so a stray UI bug cannot starve the
    /// connection).
    pub fn list_drill_attempts(
        &self,
        kind: Option<&str>,
        since_unix_ms: Option<i64>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<DrillAttemptRow>, StoreError> {
        let conn = self.lock_conn();

        // Build a single SQL string for each (kind, since) combination
        // so the index `idx_drill_attempts_history` is preserved.
        let sql = match (kind.is_some(), since_unix_ms.is_some()) {
            (true, true) => {
                "SELECT id, drill_kind, correct, mean_cents_error,
                        time_on_pitch_ratio, started_at_unix_ms,
                        finished_at_unix_ms, recording_id
                 FROM drill_attempts
                 WHERE drill_kind = ?1 AND finished_at_unix_ms >= ?2
                 ORDER BY finished_at_unix_ms DESC
                 LIMIT ?3 OFFSET ?4"
            }
            (true, false) => {
                "SELECT id, drill_kind, correct, mean_cents_error,
                        time_on_pitch_ratio, started_at_unix_ms,
                        finished_at_unix_ms, recording_id
                 FROM drill_attempts
                 WHERE drill_kind = ?1
                 ORDER BY finished_at_unix_ms DESC
                 LIMIT ?2 OFFSET ?3"
            }
            (false, true) => {
                "SELECT id, drill_kind, correct, mean_cents_error,
                        time_on_pitch_ratio, started_at_unix_ms,
                        finished_at_unix_ms, recording_id
                 FROM drill_attempts
                 WHERE finished_at_unix_ms >= ?1
                 ORDER BY finished_at_unix_ms DESC
                 LIMIT ?2 OFFSET ?3"
            }
            (false, false) => {
                "SELECT id, drill_kind, correct, mean_cents_error,
                        time_on_pitch_ratio, started_at_unix_ms,
                        finished_at_unix_ms, recording_id
                 FROM drill_attempts
                 ORDER BY finished_at_unix_ms DESC
                 LIMIT ?1 OFFSET ?2"
            }
        };

        let mut stmt = conn.prepare(sql)?;
        let mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<DrillAttemptRow> {
            let id_blob: Vec<u8> = row.get(0)?;
            let id_bytes: [u8; 16] = id_blob.as_slice().try_into().map_err(|_| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Blob,
                    "expected 16-byte UUID id".into(),
                )
            })?;
            let recording_blob: Option<Vec<u8>> = row.get(7)?;
            let recording_id: Option<RecordingId> = match recording_blob {
                Some(b) => {
                    let arr: [u8; 16] = b.as_slice().try_into().map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Blob,
                            "expected 16-byte recording id".into(),
                        )
                    })?;
                    Some(RecordingId(arr))
                }
                None => None,
            };
            let correct_int: i64 = row.get(2)?;
            Ok(DrillAttemptRow {
                id: id_bytes,
                drill_kind: row.get(1)?,
                correct: correct_int != 0,
                mean_cents_error: row.get(3)?,
                time_on_pitch_ratio: row.get(4)?,
                started_at_unix_ms: row.get(5)?,
                finished_at_unix_ms: row.get(6)?,
                recording_id,
            })
        };

        let rows = match (kind, since_unix_ms) {
            (Some(k), Some(s)) => stmt
                .query_map(params![k, s, limit, offset], mapper)?
                .collect::<Result<Vec<_>, _>>()?,
            (Some(k), None) => stmt
                .query_map(params![k, limit, offset], mapper)?
                .collect::<Result<Vec<_>, _>>()?,
            (None, Some(s)) => stmt
                .query_map(params![s, limit, offset], mapper)?
                .collect::<Result<Vec<_>, _>>()?,
            (None, None) => stmt
                .query_map(params![limit, offset], mapper)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows)
    }

    /// Count rows in `drill_attempts` for one drill kind whose
    /// `mean_cents_error` is strictly less than the supplied value.
    /// Used by the IPC layer to compute a per-kind percentile.
    pub fn count_drill_attempts_below(
        &self,
        kind: &str,
        mean_cents_error: f64,
    ) -> Result<i64, StoreError> {
        let conn = self.lock_conn();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM drill_attempts
             WHERE drill_kind = ?1 AND mean_cents_error < ?2",
            params![kind, mean_cents_error],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count)
    }

    /// Count all rows in `drill_attempts` for one drill kind. Used as
    /// the denominator of the per-kind percentile calculation.
    pub fn count_drill_attempts(&self, kind: &str) -> Result<i64, StoreError> {
        let conn = self.lock_conn();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM drill_attempts WHERE drill_kind = ?1",
            params![kind],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count)
    }
}

/// One row in the `stem_results` table, hydrated for the IPC layer.
///
/// All four `*_path` fields point at FLACs on disk under
/// `<recordings_dir>/<recording_id>/stems/{vocals,drums,bass,other}.flac`.
/// `completed_at_unix_ms` mirrors the column verbatim.
#[derive(Debug, Clone)]
pub struct StemResultRow {
    /// Wall-clock time the separation completed, in Unix milliseconds.
    pub completed_at_unix_ms: i64,
    /// On-disk FLAC for the vocals bus.
    pub vocals_path: String,
    /// On-disk FLAC for the drums bus.
    pub drums_path: String,
    /// On-disk FLAC for the bass bus.
    pub bass_path: String,
    /// On-disk FLAC for the "other" bus.
    pub other_path: String,
}

/// Input row for [`RecordingsLibrary::insert_drill_attempt`]. Mirrors
/// the schema columns minus the surrogate `id` (minted by the insert
/// helper) and `drill_payload` which is taken by ref.
#[derive(Debug, Clone)]
pub struct NewDrillAttempt<'a> {
    /// Drill-kind discriminator string. Free-form so future drills
    /// can be added without a schema migration.
    pub drill_kind: &'a str,
    /// Postcard-encoded snapshot of the IPC drill spec.
    pub drill_payload: &'a [u8],
    /// Whether the scorer judged the attempt correct.
    pub correct: bool,
    /// Mean cents error across voiced frames.
    pub mean_cents_error: f64,
    /// Fraction of voiced frames in the in-window tolerance.
    pub time_on_pitch_ratio: f64,
    /// Wall-clock attempt-start timestamp in Unix milliseconds.
    pub started_at_unix_ms: i64,
    /// Wall-clock attempt-finish timestamp in Unix milliseconds.
    pub finished_at_unix_ms: i64,
    /// Optional recording the attempt was paired with.
    pub recording_id: Option<RecordingId>,
}

/// One row in the `drill_attempts` table, hydrated for the IPC layer.
#[derive(Debug, Clone)]
pub struct DrillAttemptRow {
    /// 16-byte UUIDv7.
    pub id: [u8; 16],
    /// Drill-kind discriminator string.
    pub drill_kind: String,
    /// Whether the scorer judged the attempt correct.
    pub correct: bool,
    /// Mean cents error across voiced frames.
    pub mean_cents_error: f64,
    /// Fraction of voiced frames in the in-window tolerance.
    pub time_on_pitch_ratio: f64,
    /// Wall-clock attempt-start timestamp in Unix milliseconds.
    pub started_at_unix_ms: i64,
    /// Wall-clock attempt-finish timestamp in Unix milliseconds.
    pub finished_at_unix_ms: i64,
    /// Optional recording id this attempt was paired with.
    pub recording_id: Option<RecordingId>,
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
