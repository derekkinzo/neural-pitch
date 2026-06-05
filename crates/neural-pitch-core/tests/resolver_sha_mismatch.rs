//! Phase 2.2 resolver test #2: under a placeholder manifest the resolver
//! MUST NOT delete a user-placed cached blob, and MUST surface
//! `NotConfigured`.
//!
//! Spec: pre-seed `<dest_dir>/pesto-v1.onnx` with `b"corrupted"`. The
//! workspace manifest has the all-zeros placeholder sha and an empty URL,
//! which means the manifest cannot verify any on-disk blob. The earlier
//! revision of this test asserted the resolver should hash-and-delete the
//! file regardless; that was unsafe — the moment Phase 2.5 lands and
//! flips the manifest from placeholder to real sha+url, the very next
//! `ensure_model` call would burn whatever the user had hand-placed.
//!
//! Updated contract: under a placeholder manifest the on-disk file is
//! left alone (it is, by definition, unverifiable) and the resolver
//! surfaces `NotConfigured` directly. The mismatch-and-delete path is
//! exercised by the new `resolver_real_manifest_*` tests below, which
//! mint a hermetic manifest fixture rather than relying on the workspace
//! placeholder.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use neural_pitch_core::models::{ResolverError, ensure_model};

#[test]
fn resolver_under_placeholder_manifest_preserves_cached_blob() {
    // Per-test scratch dir under the cargo target tmpdir.
    let mut dest_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    dest_dir.push("resolver_sha_mismatch_dir");
    if dest_dir.exists() {
        std::fs::remove_dir_all(&dest_dir).expect("clear scratch dir");
    }
    std::fs::create_dir_all(&dest_dir).expect("create scratch dir");

    // Pre-seed a hand-placed blob. With the workspace manifest in
    // placeholder state (url="", sha=all-zeros) the resolver cannot
    // verify this file — but it MUST NOT delete it either. Deleting on
    // every placeholder-state call burns user bandwidth and (worse)
    // destroys hand-placed user files the moment a real manifest lands.
    let target = dest_dir.join("pesto-v1.onnx");
    std::fs::write(&target, b"hand-placed-bytes").expect("seed user-placed blob");
    assert!(target.exists(), "precondition: cached blob is on disk");

    let result = ensure_model("pesto-v1", &dest_dir);

    match result {
        Err(ResolverError::NotConfigured { name }) => {
            assert_eq!(
                name, "pesto-v1",
                "error must carry the requested model name"
            );
        }
        Err(other) => {
            panic!("expected NotConfigured under placeholder manifest, got: {other:?}")
        }
        Ok(p) => panic!("expected NotConfigured, got Ok({})", p.display()),
    }

    // The file must survive — placeholder state is unverifiable, not
    // grounds for a destructive cleanup.
    assert!(
        target.exists(),
        "resolver must NOT delete user-placed blobs while the manifest is in placeholder state"
    );
}
