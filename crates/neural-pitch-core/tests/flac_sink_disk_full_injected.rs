//! Tier-1 disk-full path with fault injection.
//!
//! The pre-existing `flac_sink_full_disk.rs` points the sink at
//! `/dev/full`, but the sink derives `<path>.partial = /dev/full.partial`
//! which (on most user accounts) errors with `EACCES` at create time and
//! never exercises the mid-write ENOSPC path. This test injects a
//! `RecordingSink` whose `write()` returns `ErrorKind::StorageFull` after
//! N samples, then drives the [`RecordingWorker`] over a bounded
//! `sync_channel` and asserts the worker surfaces the typed
//! [`RecordingError::DiskFull`] (NOT a generic `Sink(Io)`).

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
    RecordingArtifact, RecordingError, RecordingSink, RecordingSinkError, RecordingWorker,
};
use tokio_util::sync::CancellationToken;

const SAMPLE_RATE_HZ: u32 = 48_000;
const HOP: usize = 512;

/// Fault-injecting sink: returns `Ok(())` for the first `enospc_after`
/// `write()` calls, then returns `Io(StorageFull)` on every subsequent
/// call. Used to force the worker's typed-error path.
struct EnospcSink {
    writes: usize,
    enospc_after: usize,
}

impl RecordingSink for EnospcSink {
    fn write(&mut self, _samples: &[f32]) -> Result<(), RecordingSinkError> {
        self.writes += 1;
        if self.writes > self.enospc_after {
            return Err(RecordingSinkError::Io(std::io::Error::from(
                std::io::ErrorKind::StorageFull,
            )));
        }
        Ok(())
    }

    fn finalize(self: Box<Self>) -> Result<RecordingArtifact, RecordingSinkError> {
        Ok(RecordingArtifact {
            path: PathBuf::from("/tmp/enospc_unreachable.flac"),
            duration_ms: 0,
            sample_count: 0,
            sample_rate_hz: SAMPLE_RATE_HZ,
        })
    }
}

#[test]
fn worker_maps_enospc_to_typed_disk_full() {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(8);
    let cancel = CancellationToken::new();
    let dropped = Arc::new(AtomicU64::new(0));

    let sink = Box::new(EnospcSink {
        writes: 0,
        enospc_after: 2, // first 2 writes succeed, 3rd surfaces StorageFull
    });

    let worker = RecordingWorker::new(sink, rx, cancel.clone(), Arc::clone(&dropped));
    let join = std::thread::spawn(move || worker.run());

    // Push enough windows that the third write fires ENOSPC.
    for _ in 0..5 {
        tx.send(vec![0.0_f32; HOP]).expect("send window");
    }

    // The worker exits with an error on the ENOSPC write; we don't need
    // to cancel.
    let result = join.join().expect("worker thread must not panic");
    match result {
        Err(RecordingError::DiskFull) => {} // expected
        other => {
            panic!("expected RecordingError::DiskFull from typed ENOSPC mapping, got {other:?}")
        }
    }
}
