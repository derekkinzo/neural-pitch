#![allow(missing_docs)]
#![cfg(feature = "pyin")]

//! Bumping `PYIN_ANALYZER_VERSION` from `"0.1"` to `"0.2"` is
//! non-destructive on the cache layer.
//!
//! 0.1 rows stay in `analysis_cache` and remain fetchable via
//! `get_contour(.., "0.1")` for back-compat. New analyses write under
//! `(recording_id, "pyin", "0.2")` alongside any existing 0.1 row.
//!
//! This test:
//!   1. Hand-writes a postcard blob keyed `(id, "pyin", "0.1")` so the
//!      cache row predates the version bump.
//!   2. Runs `analyze_recording_blocking` at the live
//!      [`PYIN_ANALYZER_VERSION`] (`"0.2"`).
//!   3. Asserts both blobs coexist (`list_analyses_blocking` length == 2).
//!   4. Asserts the legacy 0.1 blob still decodes through
//!      `get_contour_blocking(.., "0.1")` — the back-compat path.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::f32::consts::TAU;
use std::path::PathBuf;

use neural_pitch_core::analysis::contour::{PYIN_ANALYZER_NAME, PYIN_ANALYZER_VERSION};
use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingSink};
use neural_pitch_core::store::{
    NewRecording, RecordingId, RecordingsLibrary, analyze_recording_blocking, get_contour_blocking,
    list_analyses_blocking,
};

const SAMPLE_RATE_HZ: u32 = 48_000;
const HOP_SIZE: usize = 256;
const FREQ_HZ: f32 = 440.0;
const DURATION_SECS: f32 = 1.0;

fn synth_sine(freq_hz: f32, sample_rate_hz: u32, duration_secs: f32) -> Vec<f32> {
    let total = (f64::from(sample_rate_hz) * f64::from(duration_secs)).round() as usize;
    let mut out = Vec::with_capacity(total);
    let dt = 1.0 / sample_rate_hz as f32;
    for n in 0..total {
        let t = n as f32 * dt;
        out.push(0.95 * (TAU * freq_hz * t).sin());
    }
    out
}

fn build_fixture(test_name: &str) -> (RecordingsLibrary, RecordingId) {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test_name);
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    let flac_path = tmp_root.join("fixture.flac");
    let mut sink = FlacRecordingSink::create(&flac_path, SAMPLE_RATE_HZ).expect("create sink");
    for chunk in synth_sine(FREQ_HZ, SAMPLE_RATE_HZ, DURATION_SECS).chunks(HOP_SIZE) {
        sink.write(chunk).expect("write hop");
    }
    Box::new(sink).finalize().expect("finalize");

    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");

    let id = lib
        .insert_recording(NewRecording {
            filename: flac_path
                .file_name()
                .expect("flac filename")
                .to_string_lossy()
                .into_owned(),
            created_at_unix_ms: 1_717_502_580_000,
            duration_ms: (DURATION_SECS * 1_000.0) as i64,
            sample_rate_hz: i64::from(SAMPLE_RATE_HZ),
            channels: 1,
            bit_depth: 24,
            format: "flac".to_string(),
            a4_hz: 440.0,
            instrument_profile: "voice".to_string(),
            user_label: None,
        })
        .expect("insert recording");

    (lib, id)
}

#[test]
fn cache_version_0_1_blob_coexists_with_0_2_analysis() {
    // Sanity: the live constant must be at the post-bump value or this
    // test would silently become a no-op.
    assert_eq!(
        PYIN_ANALYZER_VERSION, "0.2",
        "PYIN_ANALYZER_VERSION must be the post-bump value for this test to mean anything",
    );
    assert_eq!(PYIN_ANALYZER_NAME, "pyin");

    let (lib, id) = build_fixture("cache_version_0_1_to_0_2");

    // 1. Hand-write a postcard blob keyed at the legacy 0.1 version. The
    //    bytes are intentionally not a valid 0.2 ContourResult — what we
    //    care about is that the row survives the live 0.2 analysis pass.
    //    `get_contour_blocking(.., "0.1")` is expected to decode this row
    //    once the back-compat path lands; until then, the cache layer
    //    only has to store the bytes verbatim under the legacy key.
    let legacy_blob: Vec<u8> = b"pyin v=0.1 placeholder legacy blob bytes".to_vec();
    lib.upsert_analysis(id, PYIN_ANALYZER_NAME, "0.1", &legacy_blob)
        .expect("upsert legacy 0.1 blob must succeed");

    // 2. Run the analyzer at the live (0.2) version. This writes a fresh
    //    row at (id, "pyin", "0.2") without touching the legacy 0.1 row.
    let summary = analyze_recording_blocking(
        &lib,
        id,
        PYIN_ANALYZER_NAME,
        PYIN_ANALYZER_VERSION,
        false,
        None,
        None,
    )
    .expect("analyze_recording_blocking at 0.2 should succeed");
    assert_eq!(summary.analyzer_version, "0.2");
    assert!(!summary.was_cached, "fresh 0.2 row must not be a cache hit");

    // 3. Both rows coexist in the cache.
    let rows = list_analyses_blocking(&lib, id).expect("list_analyses must succeed");
    assert_eq!(
        rows.len(),
        2,
        "both 0.1 and 0.2 rows must coexist after the bump; got rows={rows:?}",
    );
    let mut versions: Vec<&str> = rows.iter().map(|r| r.analyzer_version.as_str()).collect();
    versions.sort_unstable();
    assert_eq!(
        versions,
        vec!["0.1", "0.2"],
        "list_analyses must enumerate both versions verbatim",
    );

    // 4. The legacy 0.1 blob still decodes through `get_contour_blocking`.
    //    This is the back-compat path the spec promises to maintain after
    //    the version bump.
    let legacy_contour = get_contour_blocking(&lib, id, PYIN_ANALYZER_NAME, "0.1").expect(
        "get_contour_blocking(.., \"0.1\") must continue to return the legacy blob \
             after the version bump",
    );
    assert!(
        legacy_contour.is_some(),
        "legacy 0.1 contour must remain fetchable; spec §3: \"get_contour(.., \"0.1\") \
         continues to return the legacy blob\"",
    );
}
