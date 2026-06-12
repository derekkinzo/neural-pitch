//! Backpressure unit test: when the encoder cannot keep up with the DSP
//! worker's hop-aligned fan-out, the bounded `sync_channel` fills up and
//! windows are dropped at the producer side — but the recording itself
//! still finalizes cleanly.
//!
//! The test wraps a [`MockRecordingSink`] in a `SlowSink` adapter that
//! sleeps 50 ms on every `write()` and feeds it through a bounded
//! `sync_channel(4)` directly into the [`RecordingWorker`]. The producer
//! pushes windows at ~1 ms intervals; whenever `try_send` returns
//! `TrySendError::Full`, the producer increments the shared
//! `dropped_windows` counter — exactly the contract the production DSP
//! worker fan-out implements.
//!
//! Asserts:
//! - `dropped_windows.load() > 0` — backpressure was actually exercised at
//!   the producer side (not, as in the prior version of this test, by
//!   the worker counting its own recv-batching drains);
//! - `finalize()` still returns `Ok` — backpressure is non-fatal.

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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{TrySendError, sync_channel};
use std::thread;
use std::time::Duration;

use neural_pitch_core::pipeline::{
    MockRecordingSink, RecordingArtifact, RecordingSink, RecordingSinkError, RecordingWorker,
};
use tokio_util::sync::CancellationToken;

const SAMPLE_RATE_HZ: u32 = 48_000;
const HOP: usize = 512;
const TOTAL_WINDOWS: usize = 80;

/// Wraps an inner sink and sleeps 50 ms on every `write()`. Models a slow
/// FLAC encoder; combined with a bounded channel of size 4 it forces the
/// producer to drop windows.
struct SlowSink {
    inner: Box<dyn RecordingSink>,
    delay: Duration,
}

impl SlowSink {
    fn new(inner: Box<dyn RecordingSink>, delay: Duration) -> Self {
        Self { inner, delay }
    }
}

impl RecordingSink for SlowSink {
    fn write(&mut self, samples: &[f32]) -> Result<(), RecordingSinkError> {
        thread::sleep(self.delay);
        self.inner.write(samples)
    }

    fn finalize(self: Box<Self>) -> Result<RecordingArtifact, RecordingSinkError> {
        self.inner.finalize()
    }
}

#[test]
fn recording_worker_drops_windows_under_backpressure() {
    // Production wires the DSP worker directly into a bounded
    // `sync_channel`; the worker's `mpsc::Receiver<Vec<f32>>` accepts the
    // bounded receiver type unchanged. We mirror that here so the test
    // exercises the same channel topology as production: a bounded
    // sync_channel of capacity 4, a producer that try_sends and counts
    // failures, and the worker draining at slow-sink pace.
    let (worker_tx, worker_rx) = sync_channel::<Vec<f32>>(4);
    let cancel = CancellationToken::new();
    let dropped = Arc::new(AtomicU64::new(0));

    let inner_sink = Box::new(MockRecordingSink::new(
        PathBuf::from("/tmp/recording_worker_dropped_windows.flac"),
        SAMPLE_RATE_HZ,
    ));
    let slow_sink = Box::new(SlowSink::new(inner_sink, Duration::from_millis(50)));

    let worker = RecordingWorker::new(slow_sink, worker_rx, cancel.clone(), Arc::clone(&dropped));
    let worker_join = thread::spawn(move || worker.run());

    // Producer: push at ~1 ms intervals. On `TrySendError::Full`,
    // increment `dropped` and continue — this models the spec's §3
    // "windows dropped when the channel is full" path on the production
    // DSP worker. The bounded channel will saturate within a few hops
    // because the slow sink only drains one window every 50 ms.
    for _ in 0..TOTAL_WINDOWS {
        let window = vec![0.0_f32; HOP];
        match worker_tx.try_send(window) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => break,
        }
        thread::sleep(Duration::from_millis(1));
    }

    // Stop the producer side so the worker's `recv_timeout` sees
    // `Disconnected` and finalizes the (slow) sink. Cancellation is
    // belt-and-suspenders; the disconnect alone would also do it.
    drop(worker_tx);
    cancel.cancel();
    let artifact_result = worker_join.join().expect("worker thread must not panic");

    assert!(
        dropped.load(Ordering::Relaxed) > 0,
        "backpressure must have caused at least one dropped window; got 0"
    );
    let artifact = artifact_result.expect("finalize must still succeed under backpressure");
    assert_eq!(
        artifact.sample_rate_hz, SAMPLE_RATE_HZ,
        "artifact must echo the sink's sample rate"
    );
}
