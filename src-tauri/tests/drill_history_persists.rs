//! Phase 4 — Tauri / persistence integration test for the
//! ear-training drill subsystem.
//!
//! Drives the headless `*_blocking` helpers
//! [`neural_pitch_core::training::start_drill_blocking`],
//! [`neural_pitch_core::training::submit_drill_attempt_blocking`], and
//! [`neural_pitch_core::training::list_drill_history_blocking`]
//! directly so the test harness does not need to spin up a full Tauri
//! runtime — same shape as the Phase 2.1 / Phase 3 integration tests.
//!
//! Asserts the V0002 schema contract:
//!
//! 1. `RecordingsLibrary::new` re-opens an existing on-disk database
//!    cleanly across the V0001 → V0002 boundary.
//! 2. `start_drill` → `submit_drill_attempt` → `list_drill_history`
//!    persists exactly one row.
//! 3. The persisted row's `drill_kind` matches the spec submitted, and
//!    `mean_cents_error` round-trips through SQLite REAL.
//!
//! Drill / training surface is default-on (no `feature = "neural"`
//! gate), so this test compiles against both the all-features and the
//! no-default-features matrices.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use neural_pitch_core::store::RecordingsLibrary;
use neural_pitch_core::training::{
    AttemptPayload, DrillKind, HistoryFilter, IpcDrillSpec, NoteSpec, list_drill_history_blocking,
    start_drill_blocking, submit_drill_attempt_blocking,
};

#[test]
fn drill_history_persists_one_row_through_start_submit_list_round_trip() {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("phase4_drill_history_persists");
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    let db_path = tmp_root.join("library.sqlite");
    // Open + close once so the V0001 / V0002 migrations are recorded
    // in `refinery_schema_history`. Re-open verifies the migration is
    // non-destructive against an existing database file.
    {
        let _lib = RecordingsLibrary::new(&db_path).expect("open library (initial)");
    }
    let lib = RecordingsLibrary::new(&db_path).expect("re-open library");

    // 1) start_drill — interval drill, prompt = A4, expected response = E5 (P5 up).
    let spec = IpcDrillSpec {
        kind: DrillKind::Interval,
        prompt_notes: vec![NoteSpec::new(69)],
        expected_response_midi: vec![76],
    };
    let session =
        start_drill_blocking(&spec).expect("start_drill must succeed for a valid interval spec");
    assert_eq!(
        session.expected_response_midi, 76,
        "session must surface the spec's first expected response MIDI; got {}",
        session.expected_response_midi,
    );

    // 2) submit_drill_attempt — score against the session.
    let attempt = AttemptPayload {
        cents_error_frames: vec![5.0, 4.0, 3.0, 4.0, 5.0],
        voiced_frames: vec![true, true, true, true, true],
        started_at_unix_ms: 1_000_000,
        finished_at_unix_ms: 1_000_500,
    };
    let result =
        submit_drill_attempt_blocking(&lib, session.session_id, &spec, Some(76), &attempt, None)
            .expect("submit_drill_attempt must succeed against a valid session id");
    assert!(
        result.correct,
        "5-cent average against expected_response = E5 must score correct; \
         got result: {result:?}",
    );
    assert!(
        result.mean_cents_error.is_finite(),
        "mean_cents_error must be a finite number; got {}",
        result.mean_cents_error,
    );

    // 3) list_drill_history — exactly one row, kind round-trips.
    let rows = list_drill_history_blocking(&lib, &HistoryFilter::default())
        .expect("list_drill_history must succeed");
    assert_eq!(
        rows.len(),
        1,
        "exactly one row must be present after a single submit; got {}",
        rows.len(),
    );
    let only = &rows[0];
    assert_eq!(
        only.drill_kind,
        DrillKind::Interval.as_str(),
        "drill_kind must round-trip through SQLite TEXT; got {}",
        only.drill_kind,
    );
    assert!(
        (only.mean_cents_error - result.mean_cents_error).abs() < 1e-3,
        "mean_cents_error must round-trip through SQLite REAL; \
         result returned {}, list_drill_history returned {}",
        result.mean_cents_error,
        only.mean_cents_error,
    );
    assert_eq!(
        only.started_at_unix_ms, attempt.started_at_unix_ms,
        "started_at_unix_ms must round-trip; got {}",
        only.started_at_unix_ms,
    );
    assert_eq!(
        only.finished_at_unix_ms, attempt.finished_at_unix_ms,
        "finished_at_unix_ms must round-trip; got {}",
        only.finished_at_unix_ms,
    );
}
