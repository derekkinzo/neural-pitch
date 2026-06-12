//! Persistence unit test: analysis cache round-trips by version.
//!
//! `upsert v=1` then `get v=1 == Some`, `get v=2 == None`, then
//! `upsert v=2`, then `get v=2 == Some(latest)` and `get v=1` still
//! `Some`.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::Path;

use neural_pitch_core::store::{NewRecording, RecordingsLibrary};

#[test]
fn store_analysis_cache_keys_on_analyzer_name_and_version() {
    let lib = RecordingsLibrary::new(Path::new(":memory:"))
        .expect("opening :memory: library should succeed once persistence ships");

    let id = lib
        .insert_recording(NewRecording {
            filename: "analysis_target.flac".to_string(),
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
        .expect("insert_recording should succeed once persistence ships");

    let analyzer = "pyin";
    let blob_v1: Vec<u8> = b"pyin v=1 result blob".to_vec();
    let blob_v2: Vec<u8> = b"pyin v=2 result blob with different bytes".to_vec();

    // 1. Upsert v=1.
    lib.upsert_analysis(id, analyzer, "1", &blob_v1)
        .expect("upsert_analysis(v=1) should succeed once persistence ships");

    // 2. get v=1 → Some(blob_v1).
    let got_v1 = lib
        .get_analysis(id, analyzer, "1")
        .expect("get_analysis(v=1) should not error once persistence ships");
    assert_eq!(
        got_v1.as_deref(),
        Some(blob_v1.as_slice()),
        "v=1 round-trip must return the exact bytes that were upserted"
    );

    // 3. get v=2 → None (not yet upserted).
    let got_v2_before = lib
        .get_analysis(id, analyzer, "2")
        .expect("get_analysis(v=2) should not error once persistence ships");
    assert!(
        got_v2_before.is_none(),
        "v=2 must be absent before its upsert; got {} bytes",
        got_v2_before.map_or(0, |b| b.len())
    );

    // 4. Upsert v=2.
    lib.upsert_analysis(id, analyzer, "2", &blob_v2)
        .expect("upsert_analysis(v=2) should succeed once persistence ships");

    // 5. get v=2 → Some(blob_v2).
    let got_v2_after = lib
        .get_analysis(id, analyzer, "2")
        .expect("get_analysis(v=2) should not error once persistence ships");
    assert_eq!(
        got_v2_after.as_deref(),
        Some(blob_v2.as_slice()),
        "v=2 round-trip must return the latest bytes"
    );

    // 6. v=1 still present — versions are independent rows, not overwrites.
    let got_v1_again = lib
        .get_analysis(id, analyzer, "1")
        .expect("get_analysis(v=1) should still succeed after v=2 upsert");
    assert_eq!(
        got_v1_again.as_deref(),
        Some(blob_v1.as_slice()),
        "v=1 must be untouched by a v=2 upsert; cache keys on (id, name, version)"
    );
}
