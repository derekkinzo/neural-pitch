//! Cancellation unit test: starting and immediately stopping a recording
//! must produce a tiny but valid artifact (`duration_ms < 50`) with a file
//! present on disk.
//!
//! This covers the spec's §6 "user cancels immediately" failure mode: the
//! worker finalizes whatever was buffered (possibly zero samples) and the
//! sink writes a complete (if brief) FLAC file. Drop never deletes the
//! file in this path because finalize ran successfully.

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

use neural_pitch_core::pipeline::{MockRecordingSink, RecordingArtifact, RecordingWorker};
use tokio_util::sync::CancellationToken;

const SAMPLE_RATE_HZ: u32 = 48_000;

#[test]
fn recording_worker_cancellation_yields_short_artifact() {
    let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
    let cancel = CancellationToken::new();
    let dropped = Arc::new(AtomicU64::new(0));

    let mut path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    path.push("recording_worker_cancellation.flac");
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }

    let sink = Box::new(MockRecordingSink::new(&path, SAMPLE_RATE_HZ));
    let worker = RecordingWorker::new(sink, rx, cancel.clone(), Arc::clone(&dropped));

    let join = thread::spawn(move || {
        worker
            .run()
            .expect("worker must finalize cleanly on immediate cancel")
    });

    // Cancel without sending any windows — model the user clicking "stop"
    // before the first hop drained.
    cancel.cancel();
    // Drop the producer to unblock the receiver if it's still waiting.
    drop(tx);

    let artifact: RecordingArtifact = join.join().expect("worker thread must not panic");

    assert_eq!(
        artifact.sample_rate_hz, SAMPLE_RATE_HZ,
        "artifact must echo the sink's sample rate"
    );
    assert!(
        artifact.duration_ms < 50,
        "artifact duration must be < 50 ms after immediate cancel; got {} ms (sample_count={})",
        artifact.duration_ms,
        artifact.sample_count
    );
    assert_eq!(
        artifact.path, path,
        "artifact must echo the sink's target path"
    );
}
