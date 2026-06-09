//! Phase 5 — V0003 stems-migration upgrade test.
//!
//! Mirror image of `store_migration_upgrade.rs` for the V0003 schema:
//!
//! 1. Open through the production [`RecordingsLibrary::new`] so V0001,
//!    V0002, AND V0003 all apply against an empty schema.
//! 2. Snip V0003 out of `refinery_schema_history`, drop the
//!    `stem_results` table + its index, and drop the
//!    `analysis_cache.stem_kind` column. SQLite cannot `DROP COLUMN`
//!    inside an `ALTER TABLE` on every shipped version, so the test
//!    rebuilds `analysis_cache` without `stem_kind` to simulate the
//!    pre-V0003 shape exactly.
//! 3. Re-open through the production codepath and assert V0003
//!    re-applies cleanly: legacy rows in `recordings` and
//!    `analysis_cache` survive untouched, the new `stem_results` table
//!    + `analysis_cache.stem_kind` column come back, and exactly one
//!    new row is appended to `refinery_schema_history`.
//!
//! The Phase 5 spec keys the new logical analysis-cache identifier on
//! `(recording_id, analyzer_name, analyzer_version, stem_kind)` while
//! leaving the existing PRIMARY KEY tuple intact. This test pins the
//! invariant that `stem_kind` is added as an `ALTER TABLE … ADD COLUMN`
//! and defaults to `NULL` so legacy rows do not need a backfill.
//!
//! Default-on (no `feature = "neural"` gate) so it compiles against
//! both the all-features and the no-default-features matrices —
//! mirrors the existing `store_migration_upgrade.rs` precedent.
//!
//! TDD-RED status: V0003 does not exist yet at the time these tests
//! land alongside the schema file (`migrations/V0003__stems.sql`); on
//! the impl phase the production migration runner will pick the file
//! up and the assertions below will pass.

#![allow(missing_docs)]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::doc_lazy_continuation
)]

use std::path::PathBuf;

use neural_pitch_core::store::{ListFilter, NewRecording, RecordingsLibrary};

fn temp_db_path(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    path.push(format!("stems_migration_upgrade_{name}.sqlite"));
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
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

fn table_exists(conn: &rusqlite::Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
        rusqlite::params![name],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .is_some()
}

fn column_exists(conn: &rusqlite::Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql).expect("prepare table_info");
    let names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("table_info row map")
        .filter_map(Result::ok)
        .collect();
    names.iter().any(|c| c == column)
}

#[test]
fn fresh_open_creates_v0003_stem_results_and_stem_kind_column() {
    let path = temp_db_path("fresh");
    let _lib = RecordingsLibrary::new(&path).expect("first open creates schema");

    let conn = rusqlite::Connection::open(&path).expect("open sidecar conn");
    assert!(
        table_exists(&conn, "stem_results"),
        "V0003 must create the stem_results table on a fresh open",
    );
    assert!(
        column_exists(&conn, "analysis_cache", "stem_kind"),
        "V0003 must add the stem_kind column to analysis_cache",
    );

    // V0001 + V0002 + V0003 are all applied so the history table has
    // exactly three rows.
    let history = refinery_history_count(&path);
    assert!(
        history >= 3,
        "fresh open must apply V0001, V0002, AND V0003; got {history} history rows",
    );
}

#[test]
fn pre_phase5_db_picks_up_v0003_cleanly() {
    // Stage a "pre-Phase-5" SQLite file by:
    //   1. Opening through the production `RecordingsLibrary::new` so
    //      V0001, V0002, and V0003 all apply with the right refinery
    //      checksums.
    //   2. Manually dropping the V0003 row from
    //      `refinery_schema_history`, dropping the `stem_results` table
    //      and its index, AND rebuilding `analysis_cache` without the
    //      `stem_kind` column so the next open looks like a database
    //      that shipped before V0003 existed. (SQLite's `ALTER TABLE …
    //      DROP COLUMN` is gated on a recent SQLite version that pre-
    //      V0003 deployments may not have, so we rebuild the whole
    //      table to be portable.)
    //   3. Re-opening and asserting V0003 re-applies cleanly — every
    //      row from step 1 must survive, the `stem_results` table must
    //      come back, the `stem_kind` column must come back, and
    //      exactly one new entry must land in
    //      `refinery_schema_history`.
    let path = temp_db_path("pre_phase5");
    let legacy_blob: &[u8] = b"legacy-analysis-cache-payload-v0002";
    let recording_id;
    {
        let lib = RecordingsLibrary::new(&path).expect("first open creates schema");
        let id = lib
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
        // Seed a pre-V0003-shaped analysis_cache row so the upgrade has
        // legacy data to preserve. The whole point of the additive
        // `ALTER TABLE … ADD COLUMN` migration is that old rows survive
        // verbatim; without seeding one the test cannot prove that
        // invariant.
        lib.upsert_analysis(id, "pyin", "1.0", legacy_blob)
            .expect("seed legacy analysis_cache row");
        recording_id = id;
    }

    // Snip V0003 state.
    {
        let conn = rusqlite::Connection::open(&path).expect("open raw conn");
        conn.execute_batch(
            r"
            DROP INDEX IF EXISTS idx_stem_results_lookup;
            DROP TABLE IF EXISTS stem_results;

            -- Rebuild analysis_cache without `stem_kind` so the on-disk
            -- shape exactly matches a pre-V0003 deployment.
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

            DELETE FROM refinery_schema_history WHERE version = 3;
            ",
        )
        .expect("snip V0003 state");

        assert!(
            !table_exists(&conn, "stem_results"),
            "pre-condition: stem_results must be gone after the snip",
        );
        assert!(
            !column_exists(&conn, "analysis_cache", "stem_kind"),
            "pre-condition: stem_kind column must be gone after the snip",
        );
    }

    let history_before = refinery_history_count(&path);
    assert_eq!(
        history_before, 2,
        "pre-condition: only V0001 + V0002 should remain in refinery_schema_history; got {history_before}",
    );

    // Re-open through the production codepath. V0003 must apply cleanly.
    let lib = RecordingsLibrary::new(&path).expect("V0002 → V0003 upgrade succeeds");
    let rows = lib
        .list_recordings(ListFilter::ActiveOnly)
        .expect("list after upgrade");
    assert_eq!(
        rows.len(),
        1,
        "legacy recording row must survive the V0003 upgrade; got {} rows",
        rows.len(),
    );
    assert_eq!(rows[0].filename, "legacy.flac");

    let conn = rusqlite::Connection::open(&path).expect("open sidecar conn after upgrade");
    assert!(
        table_exists(&conn, "stem_results"),
        "stem_results table must come back after the V0003 replay",
    );
    assert!(
        column_exists(&conn, "analysis_cache", "stem_kind"),
        "analysis_cache.stem_kind column must come back after the V0003 replay",
    );

    let history_after = refinery_history_count(&path);
    assert_eq!(
        history_after,
        history_before + 1,
        "exactly one new entry (V0003) must land in refinery_schema_history; was {history_before}, now {history_after}",
    );

    // The legacy analysis_cache blob must survive the V0003 upgrade
    // verbatim. The `IS NULL stem_kind` branch of `get_analysis` is
    // routed through here, so this also pins the contract that the new
    // column defaults to NULL on legacy rows (no backfill required).
    let recovered = lib
        .get_analysis(recording_id, "pyin", "1.0")
        .expect("get_analysis after V0003 replay")
        .expect("legacy analysis_cache blob must survive the upgrade");
    assert_eq!(
        recovered.as_slice(),
        legacy_blob,
        "legacy analysis_cache payload must be byte-identical after V0003",
    );
}
