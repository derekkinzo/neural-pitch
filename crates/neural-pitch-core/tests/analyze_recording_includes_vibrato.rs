#![allow(missing_docs)]
#![cfg(feature = "pyin")]

//! Phase 2.3 TDD-RED — `analyze_recording_blocking` projects the
//! [`VibratoReport`] onto the wire `AnalysisSummary`.
//!
//! Spec (Phase 2.3 §1): the analyzer's vibrato detector must surface a
//! populated [`VibratoReport`] for a take that contains a clean 4–7 Hz /
//! ≥5-cents vibrato across the full recording.
//!
//! Fixture: 1.5 s of a 440 Hz carrier modulated at 5 Hz / ±50 cents — the
//! same shape used by the live-pipeline `dsp_pipeline_vibrato_*` test.
//! Almost every 1-second analysis window should detect vibrato, so the
//! aggregate `vibrato_ratio` must clear `0.5`.
//!
//! TDD-RED: `summarize_cached` currently surfaces `vibrato: None`; the
//! `compute_vibrato` impl is `todo!()`. The first assertion fails until
//! the Phase 2.3 wiring lands.

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
    NewRecording, RecordingId, RecordingsLibrary, analyze_recording_blocking,
};
use neural_pitch_core::test_utils::signals::vibrato_signal;

const SAMPLE_RATE_HZ: u32 = 48_000;
const HOP_SIZE: usize = 256;
const CARRIER_HZ: f32 = 440.0;
const VIBRATO_RATE_HZ: f32 = 5.0;
const VIBRATO_EXTENT_CENTS: f32 = 50.0;
/// 1.5 s gives the offline analyzer at least two 1-second windows with
/// 50% overlap to score vibrato against.
const DURATION_SECS: f32 = 1.5;

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
            a4_hz: 440.0,
            instrument_profile: "voice".to_string(),
            user_label: None,
        })
        .expect("insert recording");

    (lib, id)
}

#[test]
fn analyze_recording_includes_vibrato_report() {
    let (lib, id) = build_fixture("analyze_recording_includes_vibrato");

    let summary = analyze_recording_blocking(&lib, id, "pyin", "0.2", false, None, None)
        .expect("analyze_recording_blocking should succeed for the vibrato fixture");

    let vibrato = summary
        .vibrato
        .as_ref()
        .expect("Phase 2.3 — fresh analysis must populate summary.vibrato for a vibrato take");

    assert!(
        vibrato.vibrato_ratio > 0.5,
        "VibratoReport.vibrato_ratio must clear 0.5 on a 5 Hz / ±50-cent vibrato fixture; \
         got {}",
        vibrato.vibrato_ratio,
    );
}
