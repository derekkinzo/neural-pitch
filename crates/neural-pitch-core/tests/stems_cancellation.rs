#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args
)]

//! Cancellation token short-circuits the stems segment loop.
//!
//! Feed 30 s of silence (forces multiple inference segments), spawn
//! a background thread that fires [`CancellationToken::cancel`]
//! after 100 ms, and assert the call returns
//! [`StemError::Cancelled`].
//!
//! Hard rule: channel-based receivers in this test path must
//! tolerate the receiver closing early — we do not block on a
//! progress channel because the cancel may fire before any
//! progress is reported.

use std::thread;
use std::time::Duration;

use neural_pitch_core::stems::{StemError, StemSeparator};
use neural_pitch_core::test_utils::signals::silence;
use tokio_util::sync::CancellationToken;

const SR_HZ: u32 = 44_100;
const DURATION_MS: u64 = 30_000;

#[ignore = "htdemucs onnx path is too slow on the CI matrix; runs locally"]
#[test]
fn stems_cancellation_returns_cancelled_error() {
    let n_samples = (SR_HZ as u64 * DURATION_MS / 1_000) as usize;
    let mono = silence(n_samples);

    let model_path = StemSeparator::ensure_model(|_| {})
        .expect("HTDemucs ONNX must be cached or downloadable on the local gate");
    let mut sep = StemSeparator::open(&model_path).expect("open HTDemucs session");

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    // Best-effort progress channel: tolerate the receiver closing
    // early. Use try_send semantics by sending into an
    // mpsc::Sender that drops cleanly when the receiver is gone.
    let (tx, rx) = std::sync::mpsc::channel::<f32>();
    let progress = move |p: f32| {
        // Ignore SendError — receiver may have already been dropped.
        let _ = tx.send(p);
    };

    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        cancel_clone.cancel();
    });

    let result = sep.separate(&mono, SR_HZ, 1, progress, &cancel);

    canceller.join().expect("cancel thread must not panic");
    drop(rx);

    assert!(
        matches!(result, Err(StemError::Cancelled)),
        "expected StemError::Cancelled, got {result:?}",
    );
}
