//! Phase 2.2 resolver test #3: offline manifest → `NotConfigured`, no
//! `.partial` left on disk.
//!
//! Spec: manifest has a single entry with `url = ""` (the workspace default
//! placeholder). `dest_dir` is empty. `ensure_model("pesto-v1", &dest_dir)`
//! must return `ResolverError::NotConfigured` and must not have created any
//! `.partial` scratch file (the fetch path is short-circuited at step 6 of
//! the resolver algorithm, before any IO that would produce a `.partial`).
//!
//! TDD-RED: `ensure_model` is `todo!()`.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use neural_pitch_core::models::{ResolverError, ensure_model};

#[test]
fn resolver_offline_skip_returns_not_configured() {
    let mut dest_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    dest_dir.push("resolver_offline_skip_dir");
    if dest_dir.exists() {
        std::fs::remove_dir_all(&dest_dir).expect("clear scratch dir");
    }
    std::fs::create_dir_all(&dest_dir).expect("create scratch dir");

    let result = ensure_model("pesto-v1", &dest_dir);

    match result {
        Err(ResolverError::NotConfigured { name }) => {
            assert_eq!(name, "pesto-v1");
        }
        Err(other) => panic!("expected NotConfigured, got: {other:?}"),
        Ok(p) => panic!("expected NotConfigured, got Ok({})", p.display()),
    }

    // The placeholder URL guard fires before any `.partial` file is opened.
    let partial = dest_dir.join("pesto-v1.onnx.partial");
    assert!(
        !partial.exists(),
        "no .partial scratch file may be created when the manifest URL is empty"
    );

    // And the final blob must not exist either.
    let target = dest_dir.join("pesto-v1.onnx");
    assert!(
        !target.exists(),
        "resolver must not synthesise a target blob when not configured"
    );
}

#[test]
fn resolver_unknown_model_surfaces_unknown_model() {
    let mut dest_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    dest_dir.push("resolver_offline_skip_unknown_dir");
    if dest_dir.exists() {
        std::fs::remove_dir_all(&dest_dir).expect("clear scratch dir");
    }
    std::fs::create_dir_all(&dest_dir).expect("create scratch dir");

    // The workspace manifest only lists `pesto-v1`; any other name must
    // surface `UnknownModel`, never `NotConfigured`.
    let result = ensure_model("crepe-tiny", &dest_dir);
    match result {
        Err(ResolverError::UnknownModel(name)) => {
            assert_eq!(name, "crepe-tiny");
        }
        Err(other) => panic!("expected UnknownModel, got: {other:?}"),
        Ok(p) => panic!("expected UnknownModel, got Ok({})", p.display()),
    }
}
