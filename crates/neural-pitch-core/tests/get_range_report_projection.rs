#![allow(missing_docs)]
#![cfg(feature = "pyin")]

//! On-demand `RangeReport` projection over a cached contour.
//!
//! `get_range_report_blocking` is the standalone accessor the
//! `get_range_report` Tauri command wraps. It re-uses the existing
//! `(recording_id, analyzer_name, analyzer_version)` `analysis_cache`
//! row, postcard-decodes the cached `ContourResult`, and projects
//! `compute_range` over it. This pins the three branches the command
//! relies on:
//!   * happy path — a cached blob produced by `analyze_recording_blocking`
//!     projects to a `RangeReport` whose `full_min_midi < full_max_midi`.
//!   * missing row — a non-existent recording id returns `Ok(None)` so the
//!     command can collapse it to its `"not found"` string.
//!   * corrupt blob — an undecodable cache row returns
//!     `Err(AnalysisError::CacheCorrupted)` so the command can map it to
//!     `"not present in cache row"`.

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

use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingSink};
use neural_pitch_core::store::{
    AnalysisError, NewRecording, RecordingId, RecordingsLibrary, analyze_recording_blocking,
    get_range_report_blocking,
};

const SAMPLE_RATE_HZ: u32 = 48_000;
const HOP_SIZE: usize = 256;
const DURATION_SECS: f32 = 2.0;
const A4_HZ: f32 = 440.0;
/// Sweep A3 (220 Hz) → A4 (440 Hz) so the recovered range spans a full
/// octave on the MIDI histogram.
const SWEEP_START_HZ: f32 = 220.0;
const SWEEP_END_HZ: f32 = 440.0;
const ANALYZER_NAME: &str = "pyin";
const ANALYZER_VERSION: &str = "0.2";

fn synth_sweep(start_hz: f32, end_hz: f32, sample_rate_hz: u32, duration_secs: f32) -> Vec<f32> {
    let total = (f64::from(sample_rate_hz) * f64::from(duration_secs)).round() as usize;
    let mut out = Vec::with_capacity(total);
    let sr = sample_rate_hz as f32;
    let mut phase: f32 = 0.0;
    for n in 0..total {
        let t = n as f32 / total as f32;
        let f_inst = start_hz + (end_hz - start_hz) * t;
        phase += TAU * f_inst / sr;
        out.push(0.95 * phase.sin());
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
    for chunk in
        synth_sweep(SWEEP_START_HZ, SWEEP_END_HZ, SAMPLE_RATE_HZ, DURATION_SECS).chunks(HOP_SIZE)
    {
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
fn range_report_projects_cached_contour_into_a_spanning_histogram() {
    let (lib, id) = build_fixture("get_range_report_happy");

    // Seed the analysis_cache row via the production analyze path.
    analyze_recording_blocking(&lib, id, ANALYZER_NAME, ANALYZER_VERSION, false, None, None)
        .expect("analyze_recording_blocking must seed the cache for the swept-sine fixture");

    let report = get_range_report_blocking(&lib, id, ANALYZER_NAME, ANALYZER_VERSION, A4_HZ)
        .expect("projection over a cached contour must not error")
        .expect("a cached row must project to Some(RangeReport)");

    assert!(
        report.full_min_midi < report.full_max_midi,
        "swept-sine range must span more than one MIDI semitone; \
         got full_min_midi={} full_max_midi={}",
        report.full_min_midi,
        report.full_max_midi,
    );
    assert!(
        report.median_hz > 0.0,
        "RangeReport.median_hz must be a real frequency for a voiced take; got {}",
        report.median_hz,
    );
}

#[test]
fn range_report_for_missing_recording_collapses_to_none() {
    let (lib, _id) = build_fixture("get_range_report_missing");

    // A freshly-minted id that was never inserted has no cache row.
    let absent = RecordingId::new_v7();
    let report = get_range_report_blocking(&lib, absent, ANALYZER_NAME, ANALYZER_VERSION, A4_HZ)
        .expect("a missing cache row must not error");

    assert!(
        report.is_none(),
        "a recording with no cached analysis row must project to Ok(None) so the command \
         can map it to its \"not found\" string; got {report:?}",
    );
}

#[test]
fn range_report_over_corrupt_blob_surfaces_cache_corrupted() {
    let (lib, id) = build_fixture("get_range_report_corrupt");

    // Upsert a deliberately-undecodable blob under the live cache key.
    // The version is NOT a recognised pre-0.2 legacy, so the decode
    // failure must surface as a hard CacheCorrupted rather than a
    // back-compat placeholder.
    let garbage: Vec<u8> = vec![0xFF, 0x00, 0xDE, 0xAD, 0xBE, 0xEF, 0x13, 0x37];
    lib.upsert_analysis(id, ANALYZER_NAME, ANALYZER_VERSION, &garbage)
        .expect("upsert of a raw blob must succeed");

    let err = get_range_report_blocking(&lib, id, ANALYZER_NAME, ANALYZER_VERSION, A4_HZ)
        .expect_err("an undecodable cache row must surface a typed error");

    assert!(
        matches!(err, AnalysisError::CacheCorrupted),
        "an undecodable blob must map to AnalysisError::CacheCorrupted so the command \
         can render \"not present in cache row\"; got {err:?}",
    );
}
