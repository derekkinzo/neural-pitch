#![allow(missing_docs)]
#![cfg(feature = "pyin")]

//! Phase 2.3 TDD-RED — `analyze_recording_blocking` projects the
//! [`RangeReport`] onto the wire `AnalysisSummary`.
//!
//! Spec (Phase 2.3 §1): "Both fields are `Option` so a fully-unvoiced take
//! … can return `None`. … `summarize_cached` projects both fields onto the
//! wire summary so the cache-hit and fresh-run paths return identical
//! shapes."
//!
//! This test drives a swept sine over a known pitch interval through
//! `analyze_recording_blocking` and pins:
//!   * `summary.range.is_some()` — the analyzer populates the range field
//!     for a take with sufficient voiced frames.
//!   * `range.full_min_midi < range.full_max_midi` — the recorded range
//!     spans more than one MIDI semitone (i.e. `min` is strictly below
//!     `max` on the recovered histogram).
//!
//! TDD-RED: `summarize_cached` currently surfaces `range: None`; the
//! `compute_range` impl is `todo!()`. The first assertion fails until the
//! Phase 2.3 wiring lands.

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
    NewRecording, RecordingId, RecordingsLibrary, analyze_recording_blocking,
};

const SAMPLE_RATE_HZ: u32 = 48_000;
const HOP_SIZE: usize = 256;
const DURATION_SECS: f32 = 2.0;
/// Sweep from A3 (220 Hz) up to A4 (440 Hz) so the recovered range spans
/// a full octave on the MIDI histogram.
const SWEEP_START_HZ: f32 = 220.0;
const SWEEP_END_HZ: f32 = 440.0;

/// Generate a linear-frequency-swept sine using a phase accumulator so the
/// instantaneous frequency at sample `n` is exactly
/// `start_hz + (end_hz - start_hz) * n / total`.
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
            a4_hz: 440.0,
            instrument_profile: "voice".to_string(),
            user_label: None,
        })
        .expect("insert recording");

    (lib, id)
}

#[test]
fn analyze_recording_includes_range_report() {
    let (lib, id) = build_fixture("analyze_recording_includes_range");

    let summary = analyze_recording_blocking(&lib, id, "pyin", "0.2", false, None, None)
        .expect("analyze_recording_blocking should succeed for the swept-sine fixture");

    let range = summary
        .range
        .as_ref()
        .expect("Phase 2.3 — fresh analysis must populate summary.range for a voiced take");

    // Recorded range must span more than one MIDI semitone — i.e. the
    // "min" bound is strictly below the "max" bound on the recovered
    // histogram. The swept sine spans a full octave, so this is a
    // healthy lower bound for the assertion.
    assert!(
        range.full_min_midi < range.full_max_midi,
        "RangeReport must report min_hz < max_hz across a swept-sine fixture; \
         got full_min_midi={} full_max_midi={}",
        range.full_min_midi,
        range.full_max_midi,
    );
    assert!(
        range.median_hz > 0.0,
        "RangeReport.median_hz must be a real frequency for a voiced take; got {}",
        range.median_hz,
    );
}
