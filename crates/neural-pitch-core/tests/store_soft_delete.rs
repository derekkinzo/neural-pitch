//! Persistence unit test: soft delete hides rows from `ActiveOnly`.
//!
//! Insert, soft-delete, `ListFilter::ActiveOnly` → empty;
//! `IncludingDeleted` → 1 row with `deleted_at_unix_ms.is_some()`.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::Path;

use neural_pitch_core::store::{ListFilter, NewRecording, RecordingsLibrary};

#[test]
fn store_soft_delete_hides_from_active_but_visible_with_including_deleted() {
    let lib = RecordingsLibrary::new(Path::new(":memory:"))
        .expect("opening :memory: library should succeed once persistence ships");

    let id = lib
        .insert_recording(NewRecording {
            filename: "soft_delete_me.flac".to_string(),
            created_at_unix_ms: 1_717_502_580_000,
            duration_ms: 5_000,
            sample_rate_hz: 48_000,
            channels: 1,
            bit_depth: 24,
            format: "flac".to_string(),
            a4_hz: 440.0,
            instrument_profile: "voice".to_string(),
            user_label: Some("scratch take".to_string()),
        })
        .expect("insert_recording should succeed once persistence ships");

    lib.soft_delete(id)
        .expect("soft_delete should succeed once persistence ships");

    let active = lib
        .list_recordings(ListFilter::ActiveOnly)
        .expect("list_recordings(ActiveOnly) should succeed once persistence ships");
    assert!(
        active.is_empty(),
        "ActiveOnly must hide soft-deleted rows; got {} rows",
        active.len()
    );

    let all = lib
        .list_recordings(ListFilter::IncludingDeleted)
        .expect("list_recordings(IncludingDeleted) should succeed once persistence ships");
    assert_eq!(
        all.len(),
        1,
        "IncludingDeleted must surface the soft-deleted row"
    );
    assert_eq!(all[0].id, id, "soft-deleted row id must match insert id");
    assert!(
        all[0].deleted_at_unix_ms.is_some(),
        "soft-deleted row must carry a tombstone timestamp"
    );
}
