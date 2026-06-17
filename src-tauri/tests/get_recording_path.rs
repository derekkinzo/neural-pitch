#![allow(missing_docs)]

//! `resolve_recording_path` projects a recording id onto its absolute
//! on-disk path.
//!
//! `resolve_recording_path` is the id-resolution helper extracted from the
//! `get_recording_path` Tauri command (which has no `_blocking` twin). The
//! front-end PlaybackPanel hands the returned path to `convertFileSrc`.
//! This pins:
//!   * happy path — an inserted recording resolves to
//!     `recordings_dir.join(filename)` as an absolute UTF-8 string.
//!   * the id-not-found branch — an id that parses but matches no row
//!     returns the `"recording {id} not found"` error rather than an
//!     empty/garbage path.
//!
//! Pure SQLite, no ONNX, so NOT `#[ignore]`d.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use neural_pitch_core::store::{NewRecording, RecordingId, RecordingsLibrary};
use neural_pitch_lib::commands::resolve_recording_path;

fn build_library(test_name: &str) -> (RecordingsLibrary, PathBuf) {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test_name);
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");
    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");
    (lib, tmp_root)
}

#[test]
fn resolve_recording_path_joins_filename_onto_recordings_dir() {
    let (lib, recordings_dir) = build_library("get_recording_path_happy");

    let filename = "rec-1717502580000-abcd1234.flac";
    let id = lib
        .insert_recording(NewRecording {
            filename: filename.to_string(),
            created_at_unix_ms: 1_717_502_580_000,
            duration_ms: 1_000,
            sample_rate_hz: 48_000,
            channels: 1,
            bit_depth: 24,
            format: "flac".to_string(),
            a4_hz: 440.0,
            instrument_profile: "voice".to_string(),
            user_label: None,
        })
        .expect("insert recording");

    let resolved = resolve_recording_path(&lib, &recordings_dir, id)
        .expect("an inserted recording must resolve to a path");

    let expected = recordings_dir.join(filename);
    assert_eq!(
        resolved,
        expected.to_str().expect("expected path is utf-8"),
        "resolved path must equal recordings_dir.join(filename) as an absolute string",
    );
    assert!(
        PathBuf::from(&resolved).is_absolute(),
        "resolved path must be absolute for convertFileSrc; got {resolved}",
    );
}

#[test]
fn resolve_recording_path_for_uninserted_id_is_not_found() {
    let (lib, recordings_dir) = build_library("get_recording_path_missing");

    // A freshly-minted id that was never inserted parses fine but matches
    // no row.
    let absent = RecordingId::new_v7();
    let err = resolve_recording_path(&lib, &recordings_dir, absent)
        .expect_err("an id with no matching row must not resolve to a path");

    assert_eq!(
        err,
        format!("recording {absent} not found"),
        "the id-not-found branch must surface the documented \"not found\" string",
    );
}
