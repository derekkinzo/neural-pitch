//! Tauri-side adapter that forwards [`PitchUpdate`] frames through a
//! `tauri::ipc::Channel`.
//!
//! `TauriChannelFrameSink` implements [`FrameSink`] from `neural-pitch-core`,
//! keeping `tauri::*` out of the core crate.
//!
//! ## RT-safety caveat
//!
//! `tauri::ipc::Channel::send` is NOT a non-allocating, non-blocking call:
//! it synchronously serialises the payload to JSON (`serde_json::to_string`,
//! heap allocation) on the calling thread, then synchronously invokes the
//! channel's inner `on_message` closure (which `webview.eval`s a
//! `format!`-built JS snippet, dispatching the IPC into the WebView). All
//! of this runs on the DSP worker thread — it does NOT trampoline onto the
//! Tauri/tokio runtime. At 48 kHz / hop=512 we send ~93 frames/sec, which
//! is well within the budget even with the per-call allocation, but
//! reviewers should not assume the worker's hot loop is allocation-free.
//!
//! `tauri::ipc::Channel<T: Serialize + Clone + Send + 'static>` is `Send`,
//! so the sink trivially satisfies the [`FrameSink: Send`] bound.

use neural_pitch_core::pipeline::{FrameSink, FrameSinkError, PitchUpdate};
use tauri::ipc::Channel;

/// Forwards every frame produced by the DSP worker through a Tauri IPC
/// channel.
///
/// Construct with [`TauriChannelFrameSink::new`] passing the channel handed
/// to `start_capture` by the JavaScript side. The sink is `Send`-bound (per
/// the [`FrameSink`] contract) and is safe to move into the boxed worker.
pub struct TauriChannelFrameSink {
    channel: Channel<PitchUpdate>,
}

impl TauriChannelFrameSink {
    /// Wrap a [`tauri::ipc::Channel`] in a [`FrameSink`].
    pub fn new(channel: Channel<PitchUpdate>) -> Self {
        Self { channel }
    }
}

impl core::fmt::Debug for TauriChannelFrameSink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TauriChannelFrameSink")
            .field("channel_id", &self.channel.id())
            .finish()
    }
}

impl FrameSink for TauriChannelFrameSink {
    fn send(&mut self, update: PitchUpdate) -> Result<(), FrameSinkError> {
        // `Channel::send` returns `tauri::Result<()>`. The only failure
        // surface that matters to us is a disconnected webview / dropped
        // channel; we collapse all variants to `Disconnected`, matching
        // the contract for the day-1 `ChannelFrameSink` mpsc adapter.
        self.channel
            .send(update)
            .map_err(|_| FrameSinkError::Disconnected)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Compile-time assertion that [`TauriChannelFrameSink`] is `Send`.
    /// The `FrameSink` trait already requires `Send`, but we still want a
    /// dedicated assertion so a future refactor cannot accidentally regress
    /// the bound silently.
    #[test]
    fn sink_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<TauriChannelFrameSink>();
    }

    /// A minimal [`FrameSink`] used to verify the trait-impl shape used by
    /// `TauriChannelFrameSink`. We capture every update in a `Vec` so tests
    /// can introspect after the worker has run. This intentionally does not
    /// drive a real `tauri::ipc::Channel` — instantiating one outside an
    /// active runtime is awkward. End-to-end correctness is covered by the
    /// existing worker unit tests plus the Playwright MCP
    /// suite.
    #[derive(Default)]
    struct MockChannelSink {
        sent: Arc<Mutex<Vec<PitchUpdate>>>,
        disconnected: bool,
    }

    impl FrameSink for MockChannelSink {
        fn send(&mut self, update: PitchUpdate) -> Result<(), FrameSinkError> {
            if self.disconnected {
                return Err(FrameSinkError::Disconnected);
            }
            self.sent.lock().expect("mock lock").push(update);
            Ok(())
        }
    }

    fn fake_update() -> PitchUpdate {
        PitchUpdate {
            timestamp_samples: 1,
            f0_hz: 440.0,
            confidence: 0.9,
            voiced: true,
            smoothed_cents: 0.0,
            target_midi: 69,
            target_hz: 440.0,
        }
    }

    #[test]
    fn mock_records_send_calls() {
        let mut sink = MockChannelSink::default();
        sink.send(fake_update()).expect("send ok");
        sink.send(fake_update()).expect("send ok");
        let captured = sink.sent.lock().expect("lock");
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].target_midi, 69);
    }

    #[test]
    fn disconnect_propagates() {
        let mut sink = MockChannelSink {
            sent: Arc::default(),
            disconnected: true,
        };
        let err = sink.send(fake_update()).expect_err("expected disconnect");
        assert!(matches!(err, FrameSinkError::Disconnected));
    }
}
