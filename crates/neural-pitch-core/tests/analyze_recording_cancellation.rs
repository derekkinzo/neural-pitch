#![allow(missing_docs)]
#![cfg(feature = "pyin")]

//! `analyze_recording_blocking` honours a flipped cancel token.
//!
//! The cancel token is the only thing `cancel_analysis` does — it flips an
//! `AtomicBool` the analyzer polls at three checkpoints (pre-flight,
//! post-analyze, pre-persist). This proves the token actually interrupts
//! the run:
//!   * pre-flight — a token already set to `true` returns
//!     `AnalysisError::Cancelled` without running the analyzer or touching
//!     the cache.
//!   * mid-run — flipping the token from a separate thread while a longer
//!     fixture is analyzing returns `Cancelled` within a bounded budget and
//!     leaves no cache row behind.

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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingSink};
use neural_pitch_core::store::{
    AnalysisError, AnalysisProgress, NewRecording, ProgressSink, RecordingId, RecordingsLibrary,
    analyze_recording_blocking, list_analyses_blocking,
};

const SAMPLE_RATE_HZ: u32 = 48_000;
const HOP_SIZE: usize = 256;
const TONE_HZ: f32 = 220.0;
const ANALYZER_NAME: &str = "pyin";
const ANALYZER_VERSION: &str = "0.2";

/// Progress sink that flips a cancel token on the first mid-run tick.
///
/// Using the progress callback as the synchronisation point removes the
/// wall-clock race: the analyzer emits ticks synchronously from inside its
/// hop loop, so flipping the token here guarantees the flag is set while
/// the worker is still iterating hops — before the uninterruptible
/// `finalize()` pass. The very next per-hop cancel poll (or the
/// pre-`finalize` poll) then trips, proving the token actually interrupts
/// the run rather than letting it complete.
///
/// Tolerates being called many times (a no-op after the first flip) and
/// never blocks, mirroring the production `ProgressSink` contract that a
/// slow or dropped consumer must not stall the ticker.
struct CancelOnFirstTick {
    cancel: Arc<AtomicBool>,
    ticks: AtomicU64,
}

impl CancelOnFirstTick {
    fn new(cancel: Arc<AtomicBool>) -> Self {
        Self {
            cancel,
            ticks: AtomicU64::new(0),
        }
    }
}

impl ProgressSink for CancelOnFirstTick {
    fn emit(&self, _progress: AnalysisProgress) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
        // Flip on the first observed tick. The hop loop's next poll trips.
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn synth_tone(hz: f32, sample_rate_hz: u32, duration_secs: f32) -> Vec<f32> {
    let total = (f64::from(sample_rate_hz) * f64::from(duration_secs)).round() as usize;
    let mut out = Vec::with_capacity(total);
    let step = TAU * hz / sample_rate_hz as f32;
    for n in 0..total {
        out.push(0.9 * (step * n as f32).sin());
    }
    out
}

fn build_fixture(test_name: &str, duration_secs: f32) -> (RecordingsLibrary, RecordingId) {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test_name);
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    let flac_path = tmp_root.join("fixture.flac");
    let mut sink = FlacRecordingSink::create(&flac_path, SAMPLE_RATE_HZ).expect("create sink");
    for chunk in synth_tone(TONE_HZ, SAMPLE_RATE_HZ, duration_secs).chunks(HOP_SIZE) {
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
            duration_ms: (duration_secs * 1_000.0) as i64,
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
fn preflight_cancel_returns_cancelled_without_analyzing() {
    let (lib, id) = build_fixture("analyze_cancel_preflight", 0.5);

    // Token already tripped before the call begins.
    let cancel = AtomicBool::new(true);
    let err = analyze_recording_blocking(
        &lib,
        id,
        ANALYZER_NAME,
        ANALYZER_VERSION,
        false,
        None,
        Some(&cancel),
    )
    .expect_err("a pre-tripped cancel token must short-circuit the analyzer");

    assert!(
        matches!(err, AnalysisError::Cancelled),
        "pre-flight cancel must return AnalysisError::Cancelled; got {err:?}",
    );

    // No cache row may have been written — the analyzer never ran.
    let rows = list_analyses_blocking(&lib, id).expect("list analyses");
    assert!(
        rows.is_empty(),
        "a pre-flight cancel must not persist a cache row; got {} rows",
        rows.len(),
    );
}

#[test]
fn midrun_cancel_interrupts_within_budget_and_persists_nothing() {
    // A multi-second fixture so the uninterrupted run would take far
    // longer than the budget below: 6 s at 48 kHz / hop 256 is ~1 125
    // hops feeding a pYIN pass whose `finalize()` alone runs for several
    // seconds. The cancel must short-circuit well before that.
    let (lib, id) = build_fixture("analyze_cancel_midrun", 6.0);

    // The sink flips this token on the first mid-run progress tick, which
    // the analyzer emits synchronously from inside its hop loop — so the
    // flag is set while the worker is still iterating hops, and the next
    // per-hop / pre-`finalize` poll trips before the heavy work runs.
    let cancel = Arc::new(AtomicBool::new(false));
    let sink = CancelOnFirstTick::new(Arc::clone(&cancel));

    let started = Instant::now();
    let result = {
        let sink_ref: &dyn ProgressSink = &sink;
        analyze_recording_blocking(
            &lib,
            id,
            ANALYZER_NAME,
            ANALYZER_VERSION,
            false,
            Some(sink_ref),
            Some(cancel.as_ref()),
        )
    };
    let elapsed = started.elapsed();

    assert!(
        matches!(result, Err(AnalysisError::Cancelled)),
        "a mid-run cancel must return AnalysisError::Cancelled; got {result:?}",
    );
    assert!(
        sink.ticks.load(Ordering::Relaxed) >= 1,
        "the analyzer must have emitted at least one mid-run tick (the flip trigger); \
         got 0 ticks",
    );
    // Bounded budget: the analyzer polls between hops and before
    // `finalize()`, so it must observe the flag well before it would
    // finish the full 6 s fixture. A generous 3 s ceiling keeps this
    // robust on a loaded CI host while still proving the token interrupts
    // rather than letting the run complete (an uncancelled 6 s pYIN run
    // takes substantially longer than this on the same host).
    assert!(
        elapsed < Duration::from_secs(3),
        "cancel must interrupt within a bounded budget, not run to completion; took {elapsed:?}",
    );

    // A cancelled run persists nothing — the pre-persist poll fires before
    // the SQLite write.
    let rows = list_analyses_blocking(&lib, id).expect("list analyses");
    assert!(
        rows.is_empty(),
        "a cancelled run must not persist a cache row; got {} rows",
        rows.len(),
    );
}
