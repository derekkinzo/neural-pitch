//! Tier-2 tests for the Phase 1.3 audio-backend fallbacks.
//!
//! These tests do **not** open a real cpal stream; they exercise the
//! pure-function seams that the Phase 1.3 spec defines:
//!
//! - [`pick_buffer_size`] (Windows WASAPI buffer-size clamp)
//! - [`AudioBackendEvent`] serialisation (the wire format the Tauri shell
//!   sees over `Channel<AudioBackendEvent>`)
//! - [`AudioEventEmitter`] callable from background threads (the cpal
//!   `err_fn` runs on the platform audio thread)
//!
//! The cpal `Stream` itself remains unmocked — the Phase 1.3 spec calls
//! out the config-query seam as the only mockable surface.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cpal::{BufferSize, SupportedBufferSize};
use neural_pitch_core::audio::{
    AudioBackendConfig, AudioBackendEvent, AudioEventEmitter, pick_buffer_size,
};
use parking_lot::Mutex;

/// Spec §3: WASAPI rejects `Fixed(256)` when the device's range starts at
/// 480; the backend must clamp to the lower bound.
#[test]
fn picks_lower_bound_when_request_below_min() {
    let chosen = pick_buffer_size(
        256,
        SupportedBufferSize::Range {
            min: 480,
            max: 1920,
        },
    );
    match chosen {
        BufferSize::Fixed(n) => assert_eq!(n, 480),
        BufferSize::Default => panic!("expected Fixed(480), got Default"),
    }
}

/// Spec §3: when the request is in-range, the backend honours it verbatim
/// — important so the latency budget in DESIGN §6.3 is achieved on every
/// device that supports it.
#[test]
fn picks_request_when_in_range() {
    let chosen = pick_buffer_size(
        960,
        SupportedBufferSize::Range {
            min: 256,
            max: 1920,
        },
    );
    match chosen {
        BufferSize::Fixed(n) => assert_eq!(n, 960),
        BufferSize::Default => panic!("expected Fixed(960), got Default"),
    }
}

/// Spec §3: `SupportedBufferSize::Unknown` (Linux ALSA default-only paths)
/// must fall back to `BufferSize::Default` rather than guessing a value.
#[test]
fn falls_back_to_default_on_unknown_range() {
    let chosen = pick_buffer_size(256, SupportedBufferSize::Unknown);
    assert!(
        matches!(chosen, BufferSize::Default),
        "expected Default for Unknown range, got {chosen:?}",
    );
}

/// Spec §5: `AudioBackendEvent::Disconnected` must serialise with a
/// stable `kind: "disconnected"` discriminator so the JS-side `switch`
/// statement does not regex-match free-form strings.
#[test]
fn disconnected_event_has_stable_kind_tag() {
    let json = serde_json::to_value(AudioBackendEvent::Disconnected).expect("serialize");
    assert_eq!(json["kind"], "disconnected");
}

/// `FormatChanged { new }` must round-trip through the Tauri channel JSON
/// shape — including the nested `AudioBackendConfig`.
#[test]
fn format_changed_event_roundtrips_config() {
    let cfg = AudioBackendConfig {
        sample_rate: 44_100,
        channels: 2,
        hop: 512,
        window: 2048,
    };
    let ev = AudioBackendEvent::FormatChanged { new: cfg.clone() };
    let json = serde_json::to_value(&ev).expect("serialize");
    assert_eq!(json["kind"], "format_changed");
    assert_eq!(json["new"]["sample_rate"], 44_100);
    assert_eq!(json["new"]["channels"], 2);

    let back: AudioBackendEvent = serde_json::from_value(json).expect("deserialize");
    match back {
        AudioBackendEvent::FormatChanged { new } => assert_eq!(new, cfg),
        other => panic!("round-trip changed variant: {other:?}"),
    }
}

/// Spec §5: `Underrun { count }` carries the cumulative dropped-sample
/// counter so the front-end can decide whether the underrun is escalating.
#[test]
fn underrun_event_carries_count() {
    let ev = AudioBackendEvent::Underrun { count: 12_345 };
    let json = serde_json::to_value(&ev).expect("serialize");
    assert_eq!(json["kind"], "underrun");
    assert_eq!(json["count"], 12_345);
}

/// The emitter is invoked from cpal's audio-thread error callback, so the
/// closure must be `Send + Sync` and callable from a non-main thread.
#[test]
fn emitter_invokable_from_background_thread() {
    let received = Arc::new(Mutex::new(Vec::<AudioBackendEvent>::new()));
    let received_for_emitter = Arc::clone(&received);
    let emitter: AudioEventEmitter = Arc::new(move |ev: AudioBackendEvent| {
        received_for_emitter.lock().push(ev);
    });

    let counter = Arc::new(AtomicU64::new(0));
    let counter_for_thread = Arc::clone(&counter);
    let emitter_for_thread = Arc::clone(&emitter);
    let join = std::thread::spawn(move || {
        emitter_for_thread(AudioBackendEvent::Disconnected);
        counter_for_thread.fetch_add(1, Ordering::Relaxed);
        emitter_for_thread(AudioBackendEvent::Underrun { count: 7 });
        counter_for_thread.fetch_add(1, Ordering::Relaxed);
    });
    join.join().expect("background thread");

    assert_eq!(counter.load(Ordering::Relaxed), 2);
    let got = received.lock().clone();
    assert_eq!(got.len(), 2);
    assert!(matches!(got[0], AudioBackendEvent::Disconnected));
    assert!(matches!(got[1], AudioBackendEvent::Underrun { count: 7 }));
}
