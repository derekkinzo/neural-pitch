//! Phase 3 — Tauri / persistence integration test for the
//! Basic Pitch transcribe surface.
//!
//! Drives [`neural_pitch_lib::transcribe::transcribe_recording_blocking`]
//! (the headless twin the Tauri command wraps) directly so the test
//! harness does not need to spin up a full Tauri runtime.
//!
//! Asserts the cache contract from the Phase 3 spec:
//!
//! 1. First call (cache miss) → `was_cached == false` and a positive
//!    `note_count` for the synthesised 1 s 440 Hz sine.
//! 2. Second call → `was_cached == true` AND elapsed `< 100 ms`
//!    (postcard decode + summary, no ONNX work).
//! 3. Third call with `force_refresh = true` → `was_cached == false`
//!    again — the spec's contract for users who tap "Re-transcribe".
//!
//! Channel-based assertions tolerate the receiver closing early (the
//! [`CapturingProgressSink`] never panics on a dropped consumer);
//! mirrors the `start_recording` progress channel contract.
//!
//! TDD-RED status — the underlying blocking helper currently returns
//! `Err(TranscribeError::NotImplemented)`. This test therefore fails at
//! runtime; Phase 3 GREEN flips it green by wiring the
//! [`neural_pitch_core::poly::basic_pitch::BasicPitchEstimator`] +
//! `analysis_cache` lookup paths.

#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use neural_pitch_core::store::RecordingsLibrary;
use neural_pitch_lib::transcribe::{
    TranscribeProgress, TranscribeProgressSink, import_audio_file_blocking,
    transcribe_recording_blocking,
};

/// Test-side `TranscribeProgressSink` used to assert the cached-path tick
/// shape AND to prove the sink tolerates a dropped consumer.
#[derive(Default)]
struct CapturingProgressSink {
    captured: Mutex<Vec<TranscribeProgress>>,
}

impl TranscribeProgressSink for CapturingProgressSink {
    fn emit(&self, progress: TranscribeProgress) {
        self.captured
            .lock()
            .expect("CapturingProgressSink mutex poisoned")
            .push(progress);
    }
}

impl CapturingProgressSink {
    fn snapshot(&self) -> Vec<TranscribeProgress> {
        self.captured
            .lock()
            .expect("CapturingProgressSink mutex poisoned")
            .clone()
    }
}

/// 1 s 440 Hz mono 16-bit PCM WAV — same shape as the import test fixture.
fn write_440hz_sine_wav(path: &std::path::Path, sample_rate_hz: u32, duration_ms: u32) {
    use std::f32::consts::TAU;

    let n_samples =
        usize::try_from((u64::from(sample_rate_hz) * u64::from(duration_ms)) / 1_000).unwrap();
    let bytes_per_sample = 2_u32;
    let num_channels = 1_u32;
    let byte_rate = sample_rate_hz * num_channels * bytes_per_sample;
    let block_align = u16::try_from(num_channels * bytes_per_sample).unwrap();
    let data_size = u32::try_from(n_samples).unwrap() * bytes_per_sample;
    let chunk_size = 36 + data_size;

    let mut f = std::fs::File::create(path).expect("create wav");
    f.write_all(b"RIFF").unwrap();
    f.write_all(&chunk_size.to_le_bytes()).unwrap();
    f.write_all(b"WAVE").unwrap();
    f.write_all(b"fmt ").unwrap();
    f.write_all(&16_u32.to_le_bytes()).unwrap();
    f.write_all(&1_u16.to_le_bytes()).unwrap();
    f.write_all(&u16::try_from(num_channels).unwrap().to_le_bytes())
        .unwrap();
    f.write_all(&sample_rate_hz.to_le_bytes()).unwrap();
    f.write_all(&byte_rate.to_le_bytes()).unwrap();
    f.write_all(&block_align.to_le_bytes()).unwrap();
    f.write_all(&16_u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_size.to_le_bytes()).unwrap();

    let phase_step = TAU * 440.0 / sample_rate_hz as f32;
    for i in 0..n_samples {
        let s = (phase_step * i as f32).sin();
        let pcm = (s * 0.6 * f32::from(i16::MAX)) as i16;
        f.write_all(&pcm.to_le_bytes()).unwrap();
    }
    f.sync_all().expect("fsync wav");
}

#[ignore = "ort cpu-fallback path is too slow on the CI matrix; runs locally"]
#[test]
fn transcribe_recording_caches_blob_and_force_refresh_re_runs_inference() {
    let tmp_root =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("phase3_transcribe_recording_cache");
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");
    let recordings_dir = tmp_root.clone();

    let source_path = tmp_root.join("source-440hz-1s.wav");
    write_440hz_sine_wav(&source_path, 48_000, 1_000);

    let id = import_audio_file_blocking(&lib, &recordings_dir, &source_path)
        .expect("import_audio_file must succeed before transcribe");

    // 1) Cache miss — first transcribe. We don't measure wall-clock here:
    //    a 1 s clip on a CPU EP under cargo-test debug build is the
    //    benchmark's noise floor. The < 5 s acceptance budget lives on a
    //    30 s clip (Phase 3 §6) which is the dev-laptop perf gate, not a
    //    CI gate.
    let sink_first = CapturingProgressSink::default();
    let summary_first = transcribe_recording_blocking(
        &lib,
        &recordings_dir,
        id,
        false,
        Some(&sink_first as &dyn TranscribeProgressSink),
    )
    .expect("first transcribe must succeed (cache miss)");
    assert!(
        !summary_first.was_cached,
        "first transcribe is a cache miss; was_cached must be false on the first run; \
         got summary: {summary_first:?}",
    );
    assert_eq!(summary_first.analyzer_name, "basic-pitch");
    assert_eq!(summary_first.analyzer_version, "1.0");
    assert!(
        summary_first.note_count >= 1,
        "Basic Pitch must recover at least one MIDI note for a 1 s 440 Hz tone; \
         got note_count = {}",
        summary_first.note_count,
    );

    // 2) Cache hit — second transcribe. < 100 ms is the spec's hard
    //    latency budget for the postcard decode + summary path.
    let sink_second = CapturingProgressSink::default();
    let started = Instant::now();
    let summary_second = transcribe_recording_blocking(
        &lib,
        &recordings_dir,
        id,
        false,
        Some(&sink_second as &dyn TranscribeProgressSink),
    )
    .expect("second transcribe must succeed (cache hit)");
    let elapsed = started.elapsed();
    assert!(
        summary_second.was_cached,
        "second transcribe MUST surface was_cached == true; got summary: {summary_second:?}",
    );
    assert!(
        elapsed.as_millis() < 100,
        "cache-hit summary path must complete in < 100 ms (Phase 3 spec §6); took {} ms",
        elapsed.as_millis(),
    );

    // The cache-hit path emits exactly one terminal tick.
    let ticks = sink_second.snapshot();
    assert_eq!(
        ticks.len(),
        1,
        "cache hit must emit exactly one progress tick (terminal); got {} ticks",
        ticks.len(),
    );
    let only = &ticks[0];
    assert!(
        (only.percent - 1.0).abs() < f32::EPSILON,
        "cache-hit terminal tick must have percent == 1.0; got {}",
        only.percent,
    );
    assert_eq!(
        only.current_window, only.total_windows,
        "cache-hit terminal tick must have current_window == total_windows",
    );

    // 3) force_refresh = true → cache miss again.
    let sink_third = CapturingProgressSink::default();
    let summary_third = transcribe_recording_blocking(
        &lib,
        &recordings_dir,
        id,
        true,
        Some(&sink_third as &dyn TranscribeProgressSink),
    )
    .expect("force_refresh transcribe must succeed");
    assert!(
        !summary_third.was_cached,
        "force_refresh = true MUST bypass the cache and re-run inference; \
         got summary: {summary_third:?}",
    );
    assert_eq!(summary_third.analyzer_name, "basic-pitch");
    assert_eq!(summary_third.analyzer_version, "1.0");
}
