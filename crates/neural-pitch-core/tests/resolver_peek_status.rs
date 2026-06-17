//! Non-fetching `models::peek` status resolution.
//!
//! `peek` is the sole core fn behind the `get_model_status` Tauri command;
//! its `(is_placeholder, on_disk_match)` verdict maps directly onto the
//! three `ModelStatus` variants the Settings UI renders. This pins the
//! branches the command depends on, driven against the real workspace
//! `models.toml` (the only manifest `peek` consults):
//!   * a configured, non-bundled-resolved entry with no on-disk blob →
//!     `!is_placeholder && !on_disk_match` (MissingButFetchable).
//!   * the same entry once a blob whose sha256 matches the manifest is
//!     placed in `dest_dir` → `on_disk_match` (Cached).
//!   * an unknown model name → `ResolverError::UnknownModel`.
//!
//! No network, no ONNX inference.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use neural_pitch_core::models::{ResolverError, peek};

/// The one configured entry in the workspace `models.toml`. It is
/// `bundled = true`, so `is_placeholder` is always false; the on-disk
/// match flips with whether a sha-matching blob is present in `dest_dir`.
const CONFIGURED_MODEL: &str = "basic-pitch-v1";

/// Source of a blob whose sha256 matches the `basic-pitch-v1` manifest
/// entry — the in-tree bundled asset the manifest pins.
const BUNDLED_ASSET: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/basic_pitch_v1.0.onnx");

fn scratch_dir(test_name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test_name);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn configured_entry_with_no_disk_blob_is_missing_but_fetchable() {
    let dir = scratch_dir("peek_missing_but_fetchable");

    let result = peek(CONFIGURED_MODEL, &dir).expect("peek of a known model must succeed");

    assert!(
        !result.is_placeholder,
        "a configured manifest entry must not be a placeholder",
    );
    assert!(
        !result.on_disk_match,
        "with no blob in dest_dir the entry must report no on-disk match \
         (the front-end renders MissingButFetchable); got on_disk_match=true",
    );
    assert_eq!(
        result.target,
        dir.join(format!("{CONFIGURED_MODEL}.onnx")),
        "target must resolve to <dest_dir>/<name>.onnx",
    );
}

#[test]
fn sha_matching_disk_blob_reports_on_disk_match() {
    let dir = scratch_dir("peek_cached");

    // Place a blob whose sha256 matches the manifest entry at the resolved
    // target path so `peek` verifies it as a cached match.
    let target = dir.join(format!("{CONFIGURED_MODEL}.onnx"));
    std::fs::copy(BUNDLED_ASSET, &target).expect("stage sha-matching blob in dest_dir");

    let result = peek(CONFIGURED_MODEL, &dir).expect("peek of a known model must succeed");

    assert!(
        !result.is_placeholder,
        "a configured manifest entry must not be a placeholder",
    );
    assert!(
        result.on_disk_match,
        "a blob whose sha256 matches the manifest must report on_disk_match \
         (the front-end renders Cached); got on_disk_match=false",
    );
}

#[test]
fn unknown_model_name_is_rejected() {
    let dir = scratch_dir("peek_unknown");

    let err = peek("model-that-does-not-exist", &dir)
        .expect_err("an unknown model name must not resolve");

    assert!(
        matches!(err, ResolverError::UnknownModel(_)),
        "an unknown model name must surface ResolverError::UnknownModel so the command \
         can render \"unknown model\"; got {err:?}",
    );
}
