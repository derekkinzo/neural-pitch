//! Unit test: [`RecordingHandle::stop_with_timeout`] returns
//! `RecordingError::Join("timeout")` when the encoder thread is wedged in
//! a slow `finalize()` longer than the supplied budget. Mirrors the
//! `commands::stop_capture` DSP_JOIN_BUDGET pattern in the Tauri shell.

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
use std::time::Duration;

use neural_pitch_core::pipeline::{
    MockRecordingSink, RecordingArtifact, RecordingError, RecordingId, RecordingSink,
    RecordingSinkError, RecordingWorker,
};
use tokio_util::sync::CancellationToken;

const SAMPLE_RATE_HZ: u32 = 48_000;

/// Sink whose `finalize()` sleeps for `delay` before deferring to the
/// inner mock sink. Models a slow disk fsync at recording stop.
struct WedgedFinalizeSink {
    inner: Box<dyn RecordingSink>,
    delay: Duration,
}

impl RecordingSink for WedgedFinalizeSink {
    fn write(&mut self, samples: &[f32]) -> Result<(), RecordingSinkError> {
        self.inner.write(samples)
    }

    fn finalize(self: Box<Self>) -> Result<RecordingArtifact, RecordingSinkError> {
        std::thread::sleep(self.delay);
        self.inner.finalize()
    }
}

#[test]
fn stop_with_timeout_returns_join_timeout() {
    let (_tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(4);
    let cancel = CancellationToken::new();
    let dropped = Arc::new(AtomicU64::new(0));

    let inner = Box::new(MockRecordingSink::new(
        PathBuf::from("/tmp/recording_handle_stop_timeout.flac"),
        SAMPLE_RATE_HZ,
    ));
    let sink = Box::new(WedgedFinalizeSink {
        inner,
        // 500 ms > our 50 ms budget, by an order of magnitude.
        delay: Duration::from_millis(500),
    });

    let worker = RecordingWorker::new(sink, rx, cancel, Arc::clone(&dropped));
    let handle = worker
        .spawn(RecordingId::new("stop-timeout-test"))
        .expect("spawn worker");

    // Bound the wait at 50 ms; the worker's finalize sleeps 500 ms, so we
    // must observe a timeout error.
    let started = std::time::Instant::now();
    let result = handle.stop_with_timeout(Duration::from_millis(50));
    let elapsed = started.elapsed();

    match result {
        Err(RecordingError::Join(reason)) => {
            assert!(
                reason.contains("timeout"),
                "expected timeout reason, got {reason:?}",
            );
        }
        other => panic!("expected Join(\"timeout\"), got {other:?}"),
    }

    // Sanity: the call returned shortly after the budget elapsed (with
    // some slack for the polling cadence).
    assert!(
        elapsed < Duration::from_millis(500),
        "stop_with_timeout must return promptly after the budget; elapsed = {elapsed:?}",
    );
}
