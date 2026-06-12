//! Migration-upgrade unit test for `store::RecordingsLibrary`.
//!
//! The other `store_*` tests open a fresh `:memory:` DB so refinery only
//! ever runs every migration against an empty schema. This test exercises
//! the path that production hits on every app launch after the first:
//! open an existing on-disk DB, confirm `migrations::run` is a no-op
//! when nothing new has been added, AND confirm the V0001 → V0002 upgrade
//! is non-destructive against pre-Phase-4 row state.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::path::PathBuf;

use neural_pitch_core::store::{ListFilter, NewRecording, RecordingsLibrary};

fn temp_db_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    path.push(format!("store_migration_upgrade_{name}.sqlite"));
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    // Also clean up the WAL/SHM sidecar files so a stale WAL from a
    // crashed test run cannot spoof state.
    let mut wal = path.clone();
    wal.set_extension("sqlite-wal");
    let _ = std::fs::remove_file(&wal);
    let mut shm = path.clone();
    shm.set_extension("sqlite-shm");
    let _ = std::fs::remove_file(&shm);
    path
}

fn refinery_history_count(path: &std::path::Path) -> i64 {
    let conn = rusqlite::Connection::open(path).expect("open sidecar conn for history check");
    conn.query_row("SELECT COUNT(*) FROM refinery_schema_history", [], |row| {
        row.get::<_, i64>(0)
    })
    .expect("query refinery_schema_history")
}

#[test]
fn reopen_already_migrated_db_is_a_noop() {
    // First open: refinery applies V0001 + V0002 against the empty
    // schema.
    let path = temp_db_path("reopen");
    {
        let lib = RecordingsLibrary::new(&path).expect("first open creates schema");
        let _id = lib
            .insert_recording(NewRecording {
                filename: "first.flac".into(),
                created_at_unix_ms: 1_700_000_000_000,
                duration_ms: 1_000,
                sample_rate_hz: 48_000,
                channels: 1,
                bit_depth: 24,
                format: "flac".into(),
                a4_hz: 440.0,
                instrument_profile: "Voice".into(),
                user_label: None,
            })
            .expect("first insert succeeds");
    }
    let history_after_first = refinery_history_count(&path);
    assert!(
        history_after_first >= 1,
        "first open must apply at least V0001; got {history_after_first}"
    );

    // Re-open: refinery sees the same migration set already applied and
    // must not insert duplicate rows in `refinery_schema_history`. The
    // existing recordings row remains visible.
    {
        let lib = RecordingsLibrary::new(&path).expect("re-open of migrated db must succeed");
        let rows = lib
            .list_recordings(ListFilter::ActiveOnly)
            .expect("list after reopen");
        assert_eq!(
            rows.len(),
            1,
            "re-opening must preserve existing rows; got {} rows",
            rows.len()
        );
        assert_eq!(rows[0].filename, "first.flac");
    }
    let history_after_second = refinery_history_count(&path);
    assert_eq!(
        history_after_second, history_after_first,
        "re-opening an already-migrated db must NOT add to refinery_schema_history (was {history_after_first}, now {history_after_second})",
    );

    // Insert one more row across a *third* open, then re-list to confirm
    // the connection-level pragmas (WAL, foreign_keys, synchronous) and
    // the migration runner all coexist cleanly across opens.
    {
        let lib = RecordingsLibrary::new(&path).expect("third open of migrated db");
        let _id = lib
            .insert_recording(NewRecording {
                filename: "second.flac".into(),
                created_at_unix_ms: 1_700_000_001_000,
                duration_ms: 2_000,
                sample_rate_hz: 48_000,
                channels: 1,
                bit_depth: 24,
                format: "flac".into(),
                a4_hz: 440.0,
                instrument_profile: "Guitar".into(),
                user_label: None,
            })
            .expect("third-open insert succeeds");
        let rows = lib
            .list_recordings(ListFilter::ActiveOnly)
            .expect("list after third open");
        assert_eq!(rows.len(), 2, "third open must accumulate, not overwrite");
    }
}

#[test]
fn pre_phase4_db_picks_up_v0002_cleanly() {
    // Stage a "pre-Phase-4" SQLite file by:
    //   1. Opening through the production `RecordingsLibrary::new` so
    //      both V0001 and V0002 apply with the right refinery checksums.
    //   2. Manually dropping the V0002 row from
    //      `refinery_schema_history` AND the `drill_attempts` table /
    //      its index so the next open looks like a database that
    //      shipped before V0002 existed.
    //   3. Re-opening and asserting V0002 re-applies cleanly — the
    //      `recordings` row from step 1 must survive.
    let path = temp_db_path("pre_phase4");
    {
        let lib = RecordingsLibrary::new(&path).expect("first open creates schema");
        let _id = lib
            .insert_recording(NewRecording {
                filename: "legacy.flac".into(),
                created_at_unix_ms: 1_700_000_000_000,
                duration_ms: 500,
                sample_rate_hz: 48_000,
                channels: 1,
                bit_depth: 24,
                format: "flac".into(),
                a4_hz: 440.0,
                instrument_profile: "Voice".into(),
                user_label: None,
            })
            .expect("seed legacy row");
    }

    // Snip V0002 + V0003 out of the refinery history and drop the
    // tables/columns each migration introduces so the database matches
    // the on-disk state of an app installed before V0002 existed. The
    // pragmas survive (they are per-connection; refinery does not
    // touch them).
    //
    // V0003's `analysis_cache.stem_kind` column would normally be
    // dropped via `ALTER TABLE … DROP COLUMN`, but that DDL only lands
    // on recent SQLite versions. Rebuild `analysis_cache` from scratch
    // so the on-disk shape exactly matches a pre-V0003 deployment —
    // mirrors the strategy the V0003-specific test uses.
    {
        let conn = rusqlite::Connection::open(&path).expect("open raw conn");
        conn.execute_batch(
            r"
            DROP INDEX IF EXISTS idx_drill_attempts_history;
            DROP TABLE IF EXISTS drill_attempts;
            DROP INDEX IF EXISTS idx_stem_results_lookup;
            DROP TABLE IF EXISTS stem_results;
            CREATE TABLE analysis_cache_pre_v0003 (
              recording_id           BLOB    NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
              analyzer_name          TEXT    NOT NULL,
              analyzer_version       TEXT    NOT NULL,
              computed_at_unix_ms    INTEGER NOT NULL,
              result_format_version  INTEGER NOT NULL,
              result_blob            BLOB    NOT NULL,
              PRIMARY KEY (recording_id, analyzer_name, analyzer_version)
            ) STRICT;
            INSERT INTO analysis_cache_pre_v0003
              SELECT recording_id, analyzer_name, analyzer_version,
                     computed_at_unix_ms, result_format_version, result_blob
              FROM analysis_cache;
            DROP TABLE analysis_cache;
            ALTER TABLE analysis_cache_pre_v0003 RENAME TO analysis_cache;
            DELETE FROM refinery_schema_history WHERE version IN (2, 3);
            ",
        )
        .expect("snip V0002 + V0003 state");
    }

    let history_before = refinery_history_count(&path);
    assert_eq!(
        history_before, 1,
        "pre-condition: only V0001 should be in refinery_schema_history; got {history_before}",
    );

    // Open through the production codepath. V0002 must apply cleanly.
    let lib = RecordingsLibrary::new(&path).expect("V0001 → V0002 upgrade succeeds");
    let rows = lib
        .list_recordings(ListFilter::ActiveOnly)
        .expect("list after upgrade");
    assert_eq!(rows.len(), 1, "legacy row must survive the upgrade");
    assert_eq!(rows[0].filename, "legacy.flac");

    let history_after = refinery_history_count(&path);
    assert_eq!(
        history_after,
        history_before + 2,
        "V0002 + V0003 must both land in refinery_schema_history on the upgrade; was {history_before}, now {history_after}",
    );
}
