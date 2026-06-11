//! Tauri / persistence integration test for the polyphonic Basic Pitch
//! transcribe surface.
//!
//! Drives [`neural_pitch_lib::transcribe::transcribe_recording_blocking`]
//! against a synthesised mono FLAC containing two simultaneous sine tones
//! (A4 = 440 Hz, E5 ≈ 659.255 Hz). A polyphonic transcriber (Bittner et al.,
//! ICASSP 2022) MUST recover at least two distinct MIDI notes from this
//! buffer; a monophonic zero-crossing-rate fallback returns exactly one.
//!
//! Marked `#[ignore]` so it stays out of the default CI matrix — the
//! bundled ONNX session and CPU EP inference push wall-clock past the
//! per-test budget. The local gate runs it via
//! `cargo test -p neural-pitch --features neural -- --include-ignored`.

#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingSink};
use neural_pitch_core::store::{NewRecording, RecordingsLibrary};
use neural_pitch_lib::transcribe::transcribe_recording_blocking;

/// Synthesise a short mono buffer at 48 kHz containing two simultaneous
/// sine tones at `f1_hz` and `f2_hz`, mixed at half amplitude each.
/// `duration_ms` is intentionally tight: Basic Pitch's polyphonic
/// detector recovers two-note onsets from a single inference window at
/// the model's native 22_050 Hz, and the wall-clock for the local gate
/// is dominated by ONNX session warm-up, not by audio length. Keep the
/// fixture short to keep the local pre-push gate fast.
fn write_polyphonic_flac(path: &std::path::Path, f1_hz: f32, f2_hz: f32) {
    use std::f32::consts::TAU;

    let sample_rate_hz: u32 = 48_000;
    let duration_ms: u32 = 1_000;
    let n_samples =
        usize::try_from((u64::from(sample_rate_hz) * u64::from(duration_ms)) / 1_000).unwrap();

    let phase_step_1 = TAU * f1_hz / sample_rate_hz as f32;
    let phase_step_2 = TAU * f2_hz / sample_rate_hz as f32;
    let mut buf = Vec::with_capacity(n_samples);
    for i in 0..n_samples {
        let s1 = (phase_step_1 * i as f32).sin();
        let s2 = (phase_step_2 * i as f32).sin();
        // Half-amplitude mix keeps the peak inside [-1.0, 1.0] without
        // clipping when the two sines align in phase.
        buf.push(0.5 * s1 + 0.5 * s2);
    }

    let mut sink =
        FlacRecordingSink::create(path, sample_rate_hz).expect("create polyphonic flac sink");
    sink.write(&buf).expect("write polyphonic samples");
    Box::new(sink).finalize().expect("finalize polyphonic flac");
}

#[ignore = "loads the bundled Basic Pitch ONNX and runs CPU inference; runs locally"]
#[test]
fn transcribe_recording_recovers_multiple_notes_from_polyphonic_input() {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("transcribe_polyphonic");
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    // The recording row's `filename` is joined to the recordings_dir to
    // resolve the on-disk audio file. Mirrors the analyze_recording.rs
    // fixture shape: db + FLAC live in the same directory so the join
    // resolves to the file we just wrote.
    let flac_path = tmp_root.join("polyphonic.flac");
    // A4 = 440.0 Hz (MIDI 69), E5 ≈ 659.255 Hz (MIDI 76). Two notes a
    // perfect fifth apart — well separated on Basic Pitch's 88-bin
    // activation grid.
    write_polyphonic_flac(&flac_path, 440.0, 659.255_f32);

    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");
    let recordings_dir = tmp_root.clone();

    let id = lib
        .insert_recording(NewRecording {
            filename: "polyphonic.flac".to_string(),
            created_at_unix_ms: 1_717_502_580_000,
            duration_ms: 1_000,
            sample_rate_hz: 48_000,
            channels: 1,
            bit_depth: 24,
            format: "flac".to_string(),
            a4_hz: 440.0,
            instrument_profile: "Imported".to_string(),
            user_label: None,
        })
        .expect("insert recording");

    let summary = transcribe_recording_blocking(&lib, &recordings_dir, id, false, None, None)
        .expect("transcribe must succeed against the synthesised polyphonic FLAC");

    assert!(
        !summary.was_cached,
        "first transcribe is a cache miss; was_cached must be false on the first run; \
         got summary: {summary:?}",
    );
    assert_eq!(summary.analyzer_name, "basic-pitch");
    assert_eq!(summary.analyzer_version, "1.0");
    // The polyphonic transcriber MUST recover at least two distinct notes
    // from a 440 Hz + 659.255 Hz mix. A monophonic ZCR fallback collapses
    // the signal to exactly one note; this assertion fails closed against
    // any regression that drops back to monophonic behaviour.
    assert!(
        summary.note_count >= 2,
        "polyphonic input MUST yield note_count >= 2 (A4 + E5); \
         got note_count = {} from summary {summary:?}",
        summary.note_count,
    );
}
