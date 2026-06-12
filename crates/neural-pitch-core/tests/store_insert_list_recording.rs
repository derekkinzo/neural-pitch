//! Persistence unit test: insert one recording, list it back.
//!
//! Insert one `NewRecording`, list, assert exact field round-trip
//! including `a4_hz` (REAL) and `user_label = NULL`.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::Path;

use neural_pitch_core::store::{ListFilter, NewRecording, RecordingsLibrary};

#[test]
fn store_inserted_recording_round_trips_through_list() {
    let lib = RecordingsLibrary::new(Path::new(":memory:"))
        .expect("opening :memory: library should succeed once persistence ships");

    let new = NewRecording {
        filename: "2026-06-04_120300_a1b2c3d4.flac".to_string(),
        created_at_unix_ms: 1_717_502_580_000,
        duration_ms: 12_345,
        sample_rate_hz: 48_000,
        channels: 1,
        bit_depth: 24,
        format: "flac".to_string(),
        a4_hz: 440.0,
        instrument_profile: "voice".to_string(),
        user_label: None,
    };

    let id = lib
        .insert_recording(new.clone())
        .expect("insert_recording should succeed once persistence ships");

    let rows = lib
        .list_recordings(ListFilter::ActiveOnly)
        .expect("list_recordings should succeed once persistence ships");

    assert_eq!(rows.len(), 1, "expected exactly one row after insert");
    let r = &rows[0];

    assert_eq!(r.id, id, "list row id must match insert id");
    assert_eq!(r.filename, new.filename);
    assert_eq!(r.created_at_unix_ms, new.created_at_unix_ms);
    assert_eq!(r.duration_ms, new.duration_ms);
    assert_eq!(r.sample_rate_hz, new.sample_rate_hz);
    assert_eq!(r.channels, new.channels);
    assert_eq!(r.bit_depth, new.bit_depth);
    assert_eq!(r.format, new.format);
    // `a4_hz` is `REAL` in the schema — exact bit-pattern round-trip must hold
    // for the canonical value `440.0`. SQLite stores `f64`; no rounding here.
    assert!(
        (r.a4_hz - new.a4_hz).abs() < f64::EPSILON,
        "a4_hz round-trip mismatch: {} vs {}",
        r.a4_hz,
        new.a4_hz
    );
    assert_eq!(r.instrument_profile, new.instrument_profile);
    assert!(
        r.user_label.is_none(),
        "user_label was None on insert; must round-trip as NULL → None"
    );
    assert!(
        r.deleted_at_unix_ms.is_none(),
        "freshly inserted row must not be tombstoned"
    );
}
