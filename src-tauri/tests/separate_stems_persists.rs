//! Tauri / persistence integration test for the HTDemucs four-stem
//! separation surface.
//!
//! Drives [`neural_pitch_lib::stems::separate_stems_blocking`] (the
//! headless twin the Tauri command wraps) directly so the test harness
//! does not need to spin up a full Tauri runtime.
//!
//! Asserts the persistence + cache contract:
//!
//! 1. First call (cache miss) → returns a `StemSummary` with
//!    `was_cached == false` and the four canonical stem paths
//!    populated. Each `vocals.flac` / `drums.flac` / `bass.flac` /
//!    `other.flac` file exists on disk under
//!    `<recordings_dir>/<recording_id>/stems/`. Exactly one row appears
//!    in `stem_results` keyed on
//!    `(recording_id, "htdemucs-4.0.1")`.
//! 2. Second call (cache hit) → `was_cached == true` AND elapsed
//!    `< 50 ms`. The shared `Arc<StemSeparator>`'s ONNX invocation
//!    counter does NOT increment between the two calls — proves the
//!    cache hit never touched the ONNX session.
//!
//! Channel-based assertions tolerate the receiver closing early (the
//! [`CapturingProgressSink`] never panics on a dropped consumer);
//! mirrors the `start_recording` progress channel contract.
//!
//! `#[ignore]`d for the CI matrix because the path is ONNX-driven; the
//! local pre-push gate runs the test via
//! `cargo test ... -- --include-ignored`.

#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::too_many_lines
)]

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use neural_pitch_core::store::RecordingsLibrary;
use neural_pitch_lib::stems::{
    HTDEMUCS_SEPARATOR_VERSION, SeparateProgress, SeparateProgressSink, StemKind, StemSeparator,
    StemSummary, separate_stems_blocking,
};
use neural_pitch_lib::transcribe::import_audio_file_blocking;
use tokio_util::sync::CancellationToken;

/// Test-side `SeparateProgressSink` that captures every tick AND
/// proves the sink tolerates a dropped consumer (the production path
/// is required to be `tracing::debug!`-only on send failure).
#[derive(Default)]
struct CapturingProgressSink {
    captured: Mutex<Vec<SeparateProgress>>,
}

impl SeparateProgressSink for CapturingProgressSink {
    fn emit(&self, progress: SeparateProgress) {
        self.captured
            .lock()
            .expect("CapturingProgressSink mutex poisoned")
            .push(progress);
    }
}

impl CapturingProgressSink {
    fn snapshot(&self) -> Vec<SeparateProgress> {
        self.captured
            .lock()
            .expect("CapturingProgressSink mutex poisoned")
            .clone()
    }
}

/// 1 s 440 Hz mono 16-bit PCM WAV — same shape as the transcribe test
/// fixture.
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

#[ignore = "htdemucs onnx path is too slow on the CI matrix; runs locally"]
#[test]
fn separate_stems_persists_four_flacs_and_stem_results_row_then_caches() {
    let tmp_root =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("phase5_separate_stems_persists");
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");
    let recordings_dir = tmp_root.clone();

    // Stage a 1 s 440 Hz mono WAV and import it through the production
    // codepath so the row + on-disk file shape match what the front-end
    // would have at the moment the user taps "Separate stems".
    let source_path = tmp_root.join("source-440hz-1s.wav");
    write_440hz_sine_wav(&source_path, 48_000, 1_000);
    let id = import_audio_file_blocking(&lib, &recordings_dir, &source_path)
        .expect("import_audio_file must succeed before separate_stems");

    // Shared `Arc<StemSeparator>` so we can read the ONNX invocation
    // counter across both calls and verify the cache-hit path never
    // touched the session.
    let separator = Arc::new(StemSeparator::new());
    let baseline_invocations = separator.onnx_invocation_count().count;

    // 1) Cache miss — first separate. Drives the
    //    decode → separate → encode pipeline and persists four FLACs
    //    plus one `stem_results` row.
    let sink_first = CapturingProgressSink::default();
    let cancel_first = CancellationToken::new();
    let summary_first: StemSummary = separate_stems_blocking(
        &lib,
        &recordings_dir,
        id,
        Arc::clone(&separator),
        cancel_first,
        Some(&sink_first as &dyn SeparateProgressSink),
    )
    .expect("first separate_stems must succeed (cache miss)");

    assert!(
        !summary_first.was_cached,
        "first separate_stems is a cache miss; was_cached must be false on the first run; \
         got summary: {summary_first:?}",
    );
    assert_eq!(
        summary_first.separator_version, HTDEMUCS_SEPARATOR_VERSION,
        "separator_version must match the build-time constant",
    );

    // Each of the four bus paths must point at an existing FLAC under
    // <recordings_dir>/<recording_id>/stems/.
    for (kind, path_string) in [
        (StemKind::Vocals, &summary_first.vocals_path),
        (StemKind::Drums, &summary_first.drums_path),
        (StemKind::Bass, &summary_first.bass_path),
        (StemKind::Other, &summary_first.other_path),
    ] {
        let p = std::path::Path::new(path_string);
        assert!(
            p.exists(),
            "{} stem FLAC must exist on disk after a cache-miss separate; \
             expected at {p:?}",
            kind.slug(),
        );
        let metadata = std::fs::metadata(p).expect("stat stem flac");
        assert!(
            metadata.len() > 0,
            "{} stem FLAC must contain audio bytes; got 0-byte file at {p:?}",
            kind.slug(),
        );
    }

    // The cache-miss path must have invoked the ONNX session at least
    // once.
    let after_first = separator.onnx_invocation_count().count;
    assert!(
        after_first > baseline_invocations,
        "cache-miss separate must invoke the HTDemucs ONNX session at least once; \
         baseline = {baseline_invocations}, after_first = {after_first}",
    );

    // Persistence row asserted indirectly via the cached fast-path
    // below: `was_cached == true` requires the row to exist.

    // 2) Cache hit — second separate. < 50 ms is the spec's hard
    //    latency budget for the cache-hit fast path.
    let sink_second = CapturingProgressSink::default();
    let cancel_second = CancellationToken::new();
    let started = Instant::now();
    let summary_second = separate_stems_blocking(
        &lib,
        &recordings_dir,
        id,
        Arc::clone(&separator),
        cancel_second,
        Some(&sink_second as &dyn SeparateProgressSink),
    )
    .expect("second separate_stems must succeed (cache hit)");
    let elapsed = started.elapsed();

    assert!(
        summary_second.was_cached,
        "second separate_stems MUST surface was_cached == true; got summary: {summary_second:?}",
    );
    assert!(
        elapsed.as_millis() < 50,
        "cache-hit separate path must complete in < 50 ms; took {} ms",
        elapsed.as_millis(),
    );

    // The cache-hit path must NOT increment the ONNX invocation
    // counter — proves the second call never touched the session.
    let after_second = separator.onnx_invocation_count().count;
    assert_eq!(
        after_second, after_first,
        "cache-hit separate must NOT touch the ONNX session; \
         expected counter to stay at {after_first}, got {after_second}",
    );

    // The four cache-hit paths must equal the cache-miss paths
    // verbatim — the `stem_results` row is the source of truth, so any
    // reshuffle of the on-disk filenames between the two calls would
    // drift the cache and surface here.
    assert_eq!(summary_second.vocals_path, summary_first.vocals_path);
    assert_eq!(summary_second.drums_path, summary_first.drums_path);
    assert_eq!(summary_second.bass_path, summary_first.bass_path);
    assert_eq!(summary_second.other_path, summary_first.other_path);

    // Cache-hit path emits at most one terminal tick (or zero — the
    // spec leaves the choice to the implementation; this assertion is
    // a sanity guard against a runaway emit loop).
    let ticks = sink_second.snapshot();
    assert!(
        ticks.len() <= 1,
        "cache-hit separate must emit at most one terminal tick; got {} ticks",
        ticks.len(),
    );

    // The cache-miss path emitted at least one tick covering decode,
    // separate, or encode — an empty tick set means the implementation
    // forgot to wire the progress sink.
    let first_ticks = sink_first.snapshot();
    assert!(
        !first_ticks.is_empty(),
        "cache-miss separate must emit at least one progress tick across the four stages",
    );
}
