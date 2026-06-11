#![allow(missing_docs)]
#![cfg(feature = "flac")]

//! `AnalysisProgress` channel emit-shape test.
//!
//! Cached path emits exactly one message with `percent: 1.0,
//! was_cached: true`. Fresh runs spawn a `tokio::time::interval(...)`
//! ticker that snapshots an `Arc<AtomicU64>` updated by the analyzer
//! worker. The ticker exits when `frames_done == frames_total` or the
//! cancel token flips.
//!
//! This test pins those assertions on the public surface
//! (`analyze_recording_blocking` + `ProgressSink` trait):
//!
//! - **Fresh run** — at least one mid-run tick is observed (`percent` in
//!   `(0.0, 1.0)` and `was_cached == false`), and the *final* observed
//!   tick has `percent == 1.0`, `frames_done == frames_total`, and
//!   `was_cached == false`. Every emitted tick carries the same
//!   stringified `recording_id`.
//! - **Cached run** — *exactly one* tick is observed, with
//!   `percent == 1.0`, `was_cached == true`, and
//!   `frames_done == frames_total`.

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
use std::sync::Mutex;

use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingSink};
use neural_pitch_core::store::{
    AnalysisProgress, NewRecording, ProgressSink, RecordingId, RecordingsLibrary,
    analyze_recording_blocking,
};

const SAMPLE_RATE_HZ: u32 = 48_000;
const HOP_SIZE: usize = 256;
const FREQ_HZ: f32 = 440.0;
// Pick a duration long enough that the ~5 Hz progress ticker has time to
// land at least one mid-run tick before the analyzer finishes.
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

/// Test-side `ProgressSink` that captures every emitted [`AnalysisProgress`]
/// for post-run inspection.
#[derive(Default)]
struct CapturingSink {
    captured: Mutex<Vec<AnalysisProgress>>,
}

impl ProgressSink for CapturingSink {
    fn emit(&self, progress: AnalysisProgress) {
        self.captured
            .lock()
            .expect("CapturingSink mutex poisoned")
            .push(progress);
    }
}

impl CapturingSink {
    fn snapshot(&self) -> Vec<AnalysisProgress> {
        self.captured
            .lock()
            .expect("CapturingSink mutex poisoned")
            .clone()
    }
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
fn analyze_recording_fresh_run_emits_progress_with_terminal_full_tick() {
    let (lib, id) = build_fixture("analyze_recording_progress_fresh");
    let id_string = id.to_string();
    let sink = CapturingSink::default();

    let summary = analyze_recording_blocking(&lib, id, "pyin", "1", false, Some(&sink), None)
        .expect("fresh analyze_recording must succeed");
    assert!(
        !summary.was_cached,
        "fresh run must report was_cached == false; got {summary:?}"
    );

    let ticks = sink.snapshot();
    assert!(
        !ticks.is_empty(),
        "fresh run must emit at least one progress tick"
    );

    // Every emitted tick must carry the same stringified recording id.
    for (i, t) in ticks.iter().enumerate() {
        assert_eq!(
            t.recording_id, id_string,
            "tick {i} must carry the analyzed recording id (got {:?})",
            t.recording_id,
        );
        assert!(
            !t.was_cached,
            "fresh run ticks must have was_cached == false; tick {i} = {t:?}",
        );
        assert!(
            (0.0..=1.0).contains(&t.percent),
            "percent must be in [0,1]; tick {i} = {t:?}",
        );
        assert!(
            t.frames_done <= t.frames_total,
            "frames_done must never exceed frames_total; tick {i} = {t:?}",
        );
    }

    // At least one mid-run tick should land in (0.0, 1.0). We pick a
    // strictly-less-than-one bound so the terminal `1.0` tick does not
    // count toward "mid-run".
    let saw_midrun_tick = ticks.iter().any(|t| t.percent > 0.0 && t.percent < 1.0);
    assert!(
        saw_midrun_tick,
        "fresh run must emit at least one mid-run progress tick (0.0 < percent < 1.0); got {ticks:?}",
    );

    // The final tick is the terminal one; must report 100 %.
    let last = ticks.last().expect("non-empty ticks asserted above");
    assert!(
        (last.percent - 1.0).abs() < f32::EPSILON,
        "final fresh-run tick must report percent == 1.0; got {last:?}",
    );
    assert_eq!(
        last.frames_done, last.frames_total,
        "final fresh-run tick must report frames_done == frames_total; got {last:?}",
    );
}

#[test]
fn analyze_recording_cached_path_emits_exactly_one_terminal_tick() {
    let (lib, id) = build_fixture("analyze_recording_progress_cached");
    let id_string = id.to_string();

    // Prime the cache. Headless first run — no progress sink, so this
    // does not influence the assertion below.
    let _ = analyze_recording_blocking(&lib, id, "pyin", "1", false, None, None)
        .expect("priming run must succeed");

    // Now read from the cache *with* a progress sink and assert the
    // shape of what lands on the channel.
    let sink = CapturingSink::default();
    let summary = analyze_recording_blocking(&lib, id, "pyin", "1", false, Some(&sink), None)
        .expect("cached analyze_recording must succeed");
    assert!(
        summary.was_cached,
        "second call with !force_refresh must report was_cached == true; got {summary:?}",
    );

    let ticks = sink.snapshot();
    assert_eq!(
        ticks.len(),
        1,
        "cached path must emit exactly one progress tick; got {ticks:?}",
    );

    let only = &ticks[0];
    assert_eq!(
        only.recording_id, id_string,
        "cached-path tick must carry the analyzed recording id",
    );
    assert!(
        only.was_cached,
        "cached-path tick must have was_cached == true; got {only:?}",
    );
    assert!(
        (only.percent - 1.0).abs() < f32::EPSILON,
        "cached-path tick must report percent == 1.0; got {only:?}",
    );
    assert_eq!(
        only.frames_done, only.frames_total,
        "cached-path tick must report frames_done == frames_total; got {only:?}",
    );
}
