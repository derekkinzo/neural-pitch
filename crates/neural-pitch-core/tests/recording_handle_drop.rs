//! Tier-1 test: dropping a [`RecordingHandle`] without calling `stop()`
//! flips the cancellation token so the encoder thread terminates instead
//! of spinning on `recv_timeout` indefinitely.
//!
//! Mirrors the production "shell shuts down mid-recording" path — Drop
//! cancels but does NOT join (joining could block on a slow finalize and
//! hang application shutdown). The encoder thread observes the
//! cancellation flag and exits within a few `recv_timeout` polls.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    dead_code,
    unused_imports
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

use neural_pitch_core::pipeline::{MockRecordingSink, RecordingId, RecordingWorker};
use tokio_util::sync::CancellationToken;

const SAMPLE_RATE_HZ: u32 = 48_000;

#[test]
fn dropping_handle_cancels_underlying_worker() {
    let (_tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(4);
    let cancel = CancellationToken::new();
    let dropped = Arc::new(AtomicU64::new(0));
    let observer = cancel.clone();

    let sink = Box::new(MockRecordingSink::new(
        PathBuf::from("/tmp/recording_handle_drop.flac"),
        SAMPLE_RATE_HZ,
    ));
    let worker = RecordingWorker::new(sink, rx, cancel, Arc::clone(&dropped));
    let handle = worker
        .spawn(RecordingId::new("drop-test"))
        .expect("spawn worker");

    // Pre-condition: the cancellation token is fresh (not yet cancelled).
    assert!(
        !observer.is_cancelled(),
        "fresh handle must not have cancellation set"
    );

    // Drop the handle without calling stop(). The Drop impl flips the
    // shared cancellation token; we observe it through a clone we kept.
    drop(handle);

    // The Drop impl flips cancel synchronously. We assert the observable
    // state immediately so this is not racy.
    assert!(
        observer.is_cancelled(),
        "dropping a RecordingHandle MUST cancel its CancellationToken"
    );

    // Wait briefly for the encoder thread to observe the cancel and exit.
    // The recv_timeout poll is 2 ms; 200 ms is comfortably more than
    // enough on any non-pathological CI runner. We do not have a join
    // handle anymore (Drop took it), but we can confirm the thread exits
    // by waiting for the producer half to remain disconnected — there is
    // no externally observable signal beyond timing, so we just sleep.
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    // No assertion beyond the cancel flag is feasible here without
    // re-engineering the worker to expose a sentinel; the cancel-flag
    // assertion above is the load-bearing one.
}
