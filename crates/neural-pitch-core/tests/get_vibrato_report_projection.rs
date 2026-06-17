#![allow(missing_docs)]
#![cfg(feature = "pyin")]

//! On-demand `VibratoReport` projection over a cached contour.
//!
//! `get_vibrato_report_blocking` is the standalone accessor the
//! `get_vibrato_report` Tauri command wraps. It re-uses the existing
//! `(recording_id, analyzer_name, analyzer_version)` `analysis_cache`
//! row, postcard-decodes the cached `ContourResult`, and projects
//! `compute_vibrato` over a raw cents-from-a4 contour. This pins:
//!   * happy path — a cached blob from a 5 Hz / ±50-cent vibrato take
//!     projects to a `VibratoReport` whose `vibrato_ratio` clears `0.5`.
//!   * missing row — a non-existent recording id returns `Ok(None)`.
//!   * corrupt blob — an undecodable cache row returns
//!     `Err(AnalysisError::CacheCorrupted)`.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::path::PathBuf;

use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingSink};
use neural_pitch_core::store::{
    AnalysisError, NewRecording, RecordingId, RecordingsLibrary, analyze_recording_blocking,
    get_vibrato_report_blocking,
};
use neural_pitch_core::test_utils::signals::vibrato_signal;

const SAMPLE_RATE_HZ: u32 = 48_000;
const HOP_SIZE: usize = 256;
const CARRIER_HZ: f32 = 440.0;
const VIBRATO_RATE_HZ: f32 = 5.0;
const VIBRATO_EXTENT_CENTS: f32 = 50.0;
const A4_HZ: f32 = 440.0;
/// 1.5 s gives the offline analyzer at least two 1-second windows with
/// 50% overlap to score vibrato against.
const DURATION_SECS: f32 = 1.5;
const ANALYZER_NAME: &str = "pyin";
const ANALYZER_VERSION: &str = "0.2";

fn build_fixture(test_name: &str) -> (RecordingsLibrary, RecordingId) {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test_name);
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    let total_samples = (f64::from(SAMPLE_RATE_HZ) * f64::from(DURATION_SECS)).round() as usize;
    let buf = vibrato_signal(
        CARRIER_HZ,
        VIBRATO_RATE_HZ,
        VIBRATO_EXTENT_CENTS,
        SAMPLE_RATE_HZ,
        total_samples,
    );

    let flac_path = tmp_root.join("fixture.flac");
    let mut sink = FlacRecordingSink::create(&flac_path, SAMPLE_RATE_HZ).expect("create sink");
    for chunk in buf.chunks(HOP_SIZE) {
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
            a4_hz: f64::from(A4_HZ),
            instrument_profile: "voice".to_string(),
            user_label: None,
        })
        .expect("insert recording");

    (lib, id)
}

#[test]
fn vibrato_report_projects_cached_contour_into_a_populated_report() {
    let (lib, id) = build_fixture("get_vibrato_report_happy");

    analyze_recording_blocking(&lib, id, ANALYZER_NAME, ANALYZER_VERSION, false, None, None)
        .expect("analyze_recording_blocking must seed the cache for the vibrato fixture");

    let report = get_vibrato_report_blocking(&lib, id, ANALYZER_NAME, ANALYZER_VERSION, A4_HZ)
        .expect("projection over a cached contour must not error")
        .expect("a cached row must project to Some(VibratoReport)");

    assert!(
        report.vibrato_ratio > 0.5,
        "VibratoReport.vibrato_ratio must clear 0.5 on a 5 Hz / ±50-cent vibrato fixture; \
         got {}",
        report.vibrato_ratio,
    );
}

#[test]
fn vibrato_report_for_missing_recording_collapses_to_none() {
    let (lib, _id) = build_fixture("get_vibrato_report_missing");

    let absent = RecordingId::new_v7();
    let report = get_vibrato_report_blocking(&lib, absent, ANALYZER_NAME, ANALYZER_VERSION, A4_HZ)
        .expect("a missing cache row must not error");

    assert!(
        report.is_none(),
        "a recording with no cached analysis row must project to Ok(None) so the command \
         can map it to its \"not found\" string; got {report:?}",
    );
}

#[test]
fn vibrato_report_over_corrupt_blob_surfaces_cache_corrupted() {
    let (lib, id) = build_fixture("get_vibrato_report_corrupt");

    let garbage: Vec<u8> = vec![0x01, 0x02, 0x03, 0xFA, 0xCE, 0xB0, 0x0C];
    lib.upsert_analysis(id, ANALYZER_NAME, ANALYZER_VERSION, &garbage)
        .expect("upsert of a raw blob must succeed");

    let err = get_vibrato_report_blocking(&lib, id, ANALYZER_NAME, ANALYZER_VERSION, A4_HZ)
        .expect_err("an undecodable cache row must surface a typed error");

    assert!(
        matches!(err, AnalysisError::CacheCorrupted),
        "an undecodable blob must map to AnalysisError::CacheCorrupted so the command \
         can render \"not present in cache row\"; got {err:?}",
    );
}
