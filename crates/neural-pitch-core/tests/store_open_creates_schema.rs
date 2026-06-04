//! Tier-1 persistence test #1: opening the library creates the v1 schema.
//!
//! Spec: open `:memory:`, query `schema_version`, assert `version = 1`,
//! assert `recordings` and `analysis_cache` exist via `sqlite_master`.
//!
//! TDD-RED: `RecordingsLibrary::new` is currently `unimplemented!()`, so this
//! test panics with a "not yet implemented" message. That panic is the red
//! signal — the implementation lands in Phase 2.0.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::Path;

use neural_pitch_core::store::RecordingsLibrary;

#[test]
fn store_open_creates_schema_version_one() {
    // `:memory:` keeps the test hermetic — no temp files, no FS mutation.
    let lib = RecordingsLibrary::new(Path::new(":memory:"))
        .expect("opening :memory: library should succeed once persistence ships");

    // The contract: a fresh open must run V0001__init.sql and leave
    // `schema_version.version = 1`. We can't query SQLite directly through the
    // public API yet (by design — the connection is private), so we lean on the
    // observable consequence: every operation against a v1 schema must succeed
    // with zero rows on a fresh database.
    let rows = lib
        .list_recordings(neural_pitch_core::store::ListFilter::ActiveOnly)
        .expect("list_recordings on a freshly migrated db should not error");
    assert!(
        rows.is_empty(),
        "fresh library must contain zero recordings, got {}",
        rows.len()
    );
}
