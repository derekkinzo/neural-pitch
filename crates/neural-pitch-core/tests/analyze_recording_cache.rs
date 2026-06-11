#![allow(missing_docs)]
#![cfg(feature = "flac")]

//! `analyze_recording` cache lifecycle integration test.
//!
//! Drives the public `analyze_recording_blocking` surface through its four
//! observable transitions:
//!
//! 1. **Fresh** — first call on a new recording row runs the analyzer and
//!    persists a cache row; the returned summary has `was_cached == false`
//!    and `computed_at_unix_ms` is non-zero.
//! 2. **Cached** — a second call (same recording, same analyzer name +
//!    version, `force_refresh = false`) short-circuits to the cache; the
//!    returned summary has `was_cached == true` and the same
//!    `computed_at_unix_ms` as the fresh run (cache hits do NOT bump the
//!    timestamp).
//! 3. **Force refresh** — `force_refresh = true` ignores the cache row,
//!    re-runs the analyzer, and overwrites the row with a fresh
//!    `computed_at_unix_ms` (strictly greater than the previous run).
//!    `was_cached == false` again.
//! 4. **Cached again** — after the force-refresh, a fourth call with
//!    `force_refresh = false` once more short-circuits to the cache;
//!    `was_cached == true` and the timestamp matches the force-refresh
//!    run.
//!
//! Field invariants asserted across every transition: `frame_rate_hz` is
//! `sample_rate_hz / hop_size`, `voiced_ratio` is in `[0.0, 1.0]`, and
//! `analyzer_name` / `analyzer_version` round-trip exactly.
//!
//! Fixture strategy: synthesize 1.0 s of a 440 Hz sine, write it through
//! `FlacRecordingSink`, insert a matching `recordings` row, and drive
//! `analyze_recording_blocking` against the resulting library.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};

use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingSink};
use neural_pitch_core::store::{
    AnalysisSummary, NewRecording, RecordingsLibrary, analyze_recording_blocking,
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

/// Assemble a hermetic `(library, recording_id, recording_path)` fixture
/// keyed off the calling test name so parallel tests do not collide on
/// the cargo target tmp dir.
fn build_fixture(test_name: &str) -> (RecordingsLibrary, neural_pitch_core::store::RecordingId) {
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

    // `RecordingsLibrary` resolves on-disk paths relative to its db parent,
    // so place the SQLite db next to the fixture FLAC. (`recordings.filename`
    // stores just the basename.)
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

    // Sanity: the file the analyzer will decode has to exist where the
    // library expects it.
    assert!(
        Path::new(&flac_path).exists(),
        "fixture FLAC missing: {}",
        flac_path.display(),
    );

    (lib, id)
}

fn assert_summary_invariants(summary: &AnalysisSummary, want_cached: bool) {
    assert_eq!(
        summary.analyzer_name, "pyin",
        "analyzer_name must round-trip exactly through the cache"
    );
    assert_eq!(
        summary.analyzer_version, "1",
        "analyzer_version must round-trip exactly through the cache"
    );

    let expected_frame_rate = f64::from(SAMPLE_RATE_HZ) / HOP_SIZE as f64;
    assert!(
        (summary.frame_rate_hz - expected_frame_rate).abs() < 1e-6,
        "frame_rate_hz must be sample_rate_hz / hop_size: expected {expected_frame_rate}, got {}",
        summary.frame_rate_hz,
    );

    assert!(
        (0.0..=1.0).contains(&summary.voiced_ratio),
        "voiced_ratio must be in [0,1]; got {}",
        summary.voiced_ratio,
    );

    assert!(
        summary.computed_at_unix_ms > 0,
        "computed_at_unix_ms must be a real Unix timestamp; got {}",
        summary.computed_at_unix_ms,
    );

    assert_eq!(
        summary.was_cached, want_cached,
        "was_cached: expected {want_cached}, got {}",
        summary.was_cached,
    );
}

#[test]
fn analyze_recording_fresh_then_cached_then_force_refresh_then_cached() {
    let (lib, id) = build_fixture("analyze_recording_cache_lifecycle");

    // 1. Fresh — analyzer runs, cache row is written.
    let fresh = analyze_recording_blocking(&lib, id, "pyin", "1", false, None, None)
        .expect("fresh analysis must succeed");
    assert_summary_invariants(&fresh, /* want_cached = */ false);

    // 2. Cached — same call, no force, must short-circuit.
    let cached = analyze_recording_blocking(&lib, id, "pyin", "1", false, None, None)
        .expect("cached read must succeed");
    assert_summary_invariants(&cached, /* want_cached = */ true);
    assert_eq!(
        cached.computed_at_unix_ms, fresh.computed_at_unix_ms,
        "cache hit MUST NOT advance the stored computed_at_unix_ms"
    );

    // The medians are computed deterministically off the same input, so
    // the cached summary must equal the fresh one (modulo `was_cached`).
    assert_eq!(
        cached.median_hz_voiced, fresh.median_hz_voiced,
        "median_hz_voiced must be byte-identical across cache hit / fresh run",
    );
    assert_eq!(
        cached.median_cents_off, fresh.median_cents_off,
        "median_cents_off must be byte-identical across cache hit / fresh run",
    );

    // 3. Force refresh — analyzer re-runs, row is overwritten with a new
    //    timestamp.
    let refreshed = analyze_recording_blocking(&lib, id, "pyin", "1", true, None, None)
        .expect("force_refresh analysis must succeed");
    assert_summary_invariants(&refreshed, /* want_cached = */ false);
    assert!(
        refreshed.computed_at_unix_ms >= fresh.computed_at_unix_ms,
        "force_refresh must not roll back the timestamp; got {} after {}",
        refreshed.computed_at_unix_ms,
        fresh.computed_at_unix_ms,
    );

    // 4. Cached again — picks up the freshly persisted row.
    let cached_again = analyze_recording_blocking(&lib, id, "pyin", "1", false, None, None)
        .expect("post-refresh cache read must succeed");
    assert_summary_invariants(&cached_again, /* want_cached = */ true);
    assert_eq!(
        cached_again.computed_at_unix_ms, refreshed.computed_at_unix_ms,
        "post-refresh cache row must be the one we just wrote",
    );
}
