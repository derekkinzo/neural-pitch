//! Persistence unit test: hard-purge drops the row, cascades to
//! `analysis_cache` + `stem_results`, and invokes the unlink callback.
//!
//! Insert a recording plus a cached analysis row and a stem-results row,
//! then `hard_purge` with a capturing unlink closure. Assert the row and
//! both cascaded child rows are gone, the closure saw the resolved path,
//! that the delete-callback's `NotFound`-to-`Ok` mapping (the same
//! decision the Tauri `delete_recording` command makes) lets the purge
//! report success, and that a non-`NotFound` unlink failure surfaces as
//! [`StoreError::Unlink`] with the failing path preserved.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use neural_pitch_core::store::{ListFilter, NewRecording, RecordingsLibrary, StoreError};

const SEPARATOR_VERSION: &str = "htdemucs-test";

fn seed_recording(
    lib: &RecordingsLibrary,
    filename: &str,
) -> neural_pitch_core::store::RecordingId {
    lib.insert_recording(NewRecording {
        filename: filename.to_string(),
        created_at_unix_ms: 1_717_502_580_000,
        duration_ms: 5_000,
        sample_rate_hz: 48_000,
        channels: 1,
        bit_depth: 24,
        format: "flac".to_string(),
        a4_hz: 440.0,
        instrument_profile: "voice".to_string(),
        user_label: None,
    })
    .expect("insert_recording should succeed")
}

#[test]
fn store_hard_purge_drops_row_cascades_children_and_unlinks_file() {
    let lib =
        RecordingsLibrary::new(Path::new(":memory:")).expect("opening :memory: library succeeds");

    let id = seed_recording(&lib, "purge_me.flac");

    // Seed a cached analysis row and a stem-results row so the FK cascade
    // has something to drop. Both reference the recording id.
    lib.upsert_analysis(id, "pyin", "0.2", b"cached-analysis-blob")
        .expect("upsert_analysis should succeed");
    lib.upsert_stem_result(
        id,
        SEPARATOR_VERSION,
        1_717_502_590_000,
        "vocals.flac",
        "drums.flac",
        "bass.flac",
        "other.flac",
    )
    .expect("upsert_stem_result should succeed");

    // Pre-condition: both child rows are present.
    assert_eq!(
        lib.list_analyses(id).expect("list_analyses succeeds").len(),
        1,
        "analysis_cache row must exist before purge"
    );
    assert!(
        lib.get_stem_result(id, SEPARATOR_VERSION)
            .expect("get_stem_result succeeds")
            .is_some(),
        "stem_results row must exist before purge"
    );

    // Capture the path the unlink callback receives so we can assert the
    // store resolved `<root>/<filename>` and handed it through.
    let unlinked: RefCell<Option<PathBuf>> = RefCell::new(None);
    lib.hard_purge(id, |path| {
        *unlinked.borrow_mut() = Some(path.to_path_buf());
        Ok(())
    })
    .expect("hard_purge should succeed");

    // The unlink callback fired with a path ending in the recording's
    // filename (the `:memory:` library roots paths at the parent of the
    // db file; we assert the filename component rather than the absolute
    // prefix so the test is location-agnostic).
    let seen = unlinked
        .borrow()
        .clone()
        .expect("unlink callback must fire");
    assert_eq!(
        seen.file_name().and_then(|n| n.to_str()),
        Some("purge_me.flac"),
        "unlink path must resolve to the recording filename, got {seen:?}"
    );

    // The recording row is gone from every filter (not merely tombstoned).
    let all = lib
        .list_recordings(ListFilter::IncludingDeleted)
        .expect("list_recordings(IncludingDeleted) succeeds");
    assert!(
        all.iter().all(|r| r.id != id),
        "hard_purge must remove the row entirely, including from IncludingDeleted"
    );

    // FK cascade dropped both child rows.
    assert!(
        lib.list_analyses(id)
            .expect("list_analyses succeeds")
            .is_empty(),
        "FK cascade must drop analysis_cache rows on hard_purge"
    );
    assert!(
        lib.get_stem_result(id, SEPARATOR_VERSION)
            .expect("get_stem_result succeeds")
            .is_none(),
        "FK cascade must drop stem_results rows on hard_purge"
    );
}

#[test]
fn store_hard_purge_delete_callback_maps_not_found_to_ok() {
    let lib =
        RecordingsLibrary::new(Path::new(":memory:")).expect("opening :memory: library succeeds");
    let id = seed_recording(&lib, "already_gone.flac");

    // A real on-disk file may already be absent (a prior partial purge, an
    // external cleanup). The `delete_recording` command's unlink callback
    // maps a NotFound unlink to Ok — the row is the source of truth, so an
    // orphan-unlink retry must converge. Replicate that exact callback so
    // the purge reports success and the row is gone.
    lib.hard_purge(id, |path| match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    })
    .expect("hard_purge must report Ok when the callback maps NotFound to Ok");

    let all = lib
        .list_recordings(ListFilter::IncludingDeleted)
        .expect("list_recordings(IncludingDeleted) succeeds");
    assert!(
        all.iter().all(|r| r.id != id),
        "the row must still be removed even when the unlink reports NotFound"
    );
}

#[test]
fn store_hard_purge_surfaces_a_non_not_found_unlink_error() {
    let lib =
        RecordingsLibrary::new(Path::new(":memory:")).expect("opening :memory: library succeeds");
    let id = seed_recording(&lib, "permission_denied.flac");

    // A non-NotFound unlink failure (e.g. EPERM) is a genuine partial
    // failure: the row is already deleted but the file is orphaned. The
    // store must surface it as StoreError::Unlink with the failing path so
    // the caller can observe the orphan rather than silently swallow it.
    let err = lib
        .hard_purge(id, |_path| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .expect_err("a non-NotFound unlink must surface as an error");
    match err {
        StoreError::Unlink { path, source } => {
            assert_eq!(
                path.file_name().and_then(|n| n.to_str()),
                Some("permission_denied.flac"),
                "Unlink error must preserve the failing path, got {path:?}"
            );
            assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
        }
        other => panic!("expected StoreError::Unlink, got {other:?}"),
    }

    // The row is gone even though the file unlink failed — the delete-then
    // -unlink ordering leaves an orphaned file, never a re-listable row.
    let all = lib
        .list_recordings(ListFilter::IncludingDeleted)
        .expect("list_recordings(IncludingDeleted) succeeds");
    assert!(
        all.iter().all(|r| r.id != id),
        "the row must be removed before the unlink runs, so a failed unlink still leaves no row"
    );
}
