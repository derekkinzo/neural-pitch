//! Phase 2.2 resolver test #1: `models.toml` round-trip.
//!
//! Spec: write a fixture manifest with one valid `[[models]]` block to a
//! per-test scratch dir, parse it, and assert the entry's name + license
//! survive the round-trip.
//!
//! TDD-RED: `Manifest::from_toml_str` is `todo!()`, so this test panics with
//! "not yet implemented". That panic is the red signal — Phase 2.2 lands the
//! parser.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use neural_pitch_core::models::Manifest;

const FIXTURE: &str = r#"
schema_version = 1

[[models]]
name        = "pesto-v1"
url         = "https://example.invalid/pesto-v1.onnx"
sha256      = "abcdef0000000000000000000000000000000000000000000000000000000000"
size_bytes  = 4_000_000
license     = "LGPL-3.0-or-later"
"#;

#[test]
fn resolver_manifest_round_trips_one_entry() {
    // Write the fixture to a hermetic per-test path so any future
    // `read_from_path` smoke can reuse the same scaffolding.
    let mut path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    path.push("resolver_manifest_parsing.toml");
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    std::fs::write(&path, FIXTURE).expect("write fixture manifest");

    // Parse via the public API. The string variant is the inner contract;
    // `read_from_path` is just IO + delegation.
    let parsed = Manifest::from_toml_str(FIXTURE).expect("parse fixture manifest");

    assert_eq!(
        parsed.schema_version, 1,
        "schema_version must round-trip exactly"
    );
    assert_eq!(parsed.models.len(), 1, "fixture has exactly one entry");

    let entry = parsed
        .entry("pesto-v1")
        .expect("pesto-v1 entry must be looked up by name");
    assert_eq!(entry.name, "pesto-v1");
    assert_eq!(entry.url, "https://example.invalid/pesto-v1.onnx");
    assert_eq!(
        entry.sha256,
        "abcdef0000000000000000000000000000000000000000000000000000000000"
    );
    assert_eq!(entry.size_bytes, 4_000_000);
    assert_eq!(entry.license, "LGPL-3.0-or-later");
}

#[test]
fn resolver_manifest_rejects_unsupported_schema_version() {
    let bad = r#"
schema_version = 2

[[models]]
name        = "pesto-v1"
url         = ""
sha256      = "0000000000000000000000000000000000000000000000000000000000000000"
size_bytes  = 0
license     = "LGPL-3.0-or-later"
"#;
    let res = Manifest::from_toml_str(bad);
    assert!(
        res.is_err(),
        "schema_version != 1 must be rejected, not silently reinterpreted"
    );
}
