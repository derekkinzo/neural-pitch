//! Happy-path unit test for the [`RecordingWorker`] consumer loop.
//!
//! Drive the fan-out channel with 1 s of synthetic audio at the canonical
//! capture geometry (hop=512, sample_rate=48 kHz). Stop the worker via the
//! shared cancellation token, then assert the resulting
//! [`RecordingArtifact`] reports a duration in `[950, 1050]` ms — i.e.
//! within ±5% of the wall-clock 1 s of audio that was pushed through the
//! channel.
//!
//! This test treats `RecordingWorker` as a black box around its
//! [`RecordingSink`] dependency. It uses [`MockRecordingSink`] so the
//! happy-path geometry can be asserted without involving the FLAC encoder
//! or the filesystem.

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
use std::thread;
use std::time::Duration;

use neural_pitch_core::pipeline::{
    MockRecordingSink, RecordingArtifact, RecordingId, RecordingWorker,
};
use tokio_util::sync::CancellationToken;

const SAMPLE_RATE_HZ: u32 = 48_000;
const HOP: usize = 512;

#[test]
fn recording_worker_happy_path_one_second_artifact() {
    let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
    let cancel = CancellationToken::new();
    let dropped = Arc::new(AtomicU64::new(0));

    let sink = Box::new(MockRecordingSink::new(
        PathBuf::from("/tmp/recording_worker_basic.flac"),
        SAMPLE_RATE_HZ,
    ));
    let worker = RecordingWorker::new(sink, rx, cancel.clone(), Arc::clone(&dropped));

    // Drive the worker on a dedicated thread so the test thread can push
    // hop-sized windows through the channel.
    let join: thread::JoinHandle<RecordingArtifact> = thread::spawn(move || {
        worker
            .run()
            .expect("worker must finalize cleanly on cancellation")
    });

    // Push 1 s of audio at hop=512 / 48 kHz. 48000 / 512 ≈ 93.75 hops; we
    // round up so the artifact's duration_ms can land at >= 950 ms.
    let hops = (SAMPLE_RATE_HZ as usize / HOP) + 1;
    for _ in 0..hops {
        tx.send(vec![0.0_f32; HOP]).expect("send hop slice");
    }
    // Yield a bit so the worker has time to drain before we cancel.
    thread::sleep(Duration::from_millis(20));
    cancel.cancel();

    let artifact = join.join().expect("worker thread must not panic");

    assert_eq!(
        artifact.sample_rate_hz, SAMPLE_RATE_HZ,
        "artifact must echo the sink's sample rate"
    );
    assert!(
        artifact.duration_ms >= 950 && artifact.duration_ms <= 1050,
        "artifact duration must be ~1 s; got {} ms (sample_count={})",
        artifact.duration_ms,
        artifact.sample_count
    );
}
