//! Tier-1 migration-upgrade test for `store::RecordingsLibrary`.
//!
//! The other `store_*` tests open a fresh `:memory:` DB so refinery only
//! ever runs V0001 against an empty schema. This test exercises the path
//! that production hits on every app launch after the first: open an
//! existing on-disk DB, confirm `migrations::run` is a no-op (no extra
//! `refinery_schema_history` rows), and that subsequent inserts/lists
//! succeed.
//!
//! When V0002+ migrations land, this file is the canonical home for
//! "upgrades from V0001-only DB do not lose data" coverage.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    dead_code,
    unused_imports
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
    // First open: refinery applies V0001 against the empty schema.
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
    // must not insert a duplicate row in `refinery_schema_history`. The
    // existing row remains visible.
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
