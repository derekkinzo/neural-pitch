//! Fixture-driven test: confirm that [`MockAudioBackend`] increments its
//! `dropped_samples` counter when the SPSC ring overflows. That counter
//! is the only underrun-visibility signal the audio loop exposes;
//! without this test, regressions that silently zero the counter (or
//! skip the increment entirely) would not be caught.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::atomic::Ordering;

use neural_pitch_core::audio::{AudioBackend, AudioBackendConfig, MockAudioBackend, SampleSource};

#[test]
fn mock_backend_overflow_advances_dropped_counter() {
    // Tiny ring with no consumer attached so we can force overflow.
    let cfg = AudioBackendConfig {
        sample_rate: 48_000,
        channels: 1,
        hop: 16,
        window: 64,
    };
    // Ring capacity from `next_pow2(3 * window)` = 256 samples.
    let cap = cfg.ring_capacity();
    let (producer, _consumer) = rtrb::RingBuffer::<f32>::new(cap);

    let mut backend = MockAudioBackend::new(cfg.clone(), SampleSource::Sine { hz: 440.0 });
    backend.start(producer).expect("start backend");

    // Push 4 * cap samples; only `cap` (or `cap - 1` on some rtrb releases)
    // can fit, so well over half must be dropped.
    let to_push = cap * 4;
    let accepted = backend.feed(to_push);
    let dropped = backend.dropped_samples().load(Ordering::Relaxed);

    assert!(
        dropped > 0,
        "dropped counter should advance when the ring overflows; got accepted={accepted} dropped={dropped}",
    );
    assert_eq!(
        accepted as u64 + dropped,
        to_push as u64,
        "every fed sample must be either accepted or dropped (accepted={accepted} dropped={dropped} pushed={to_push})",
    );
    // Sanity: we cannot accept more than the ring's capacity without a
    // draining consumer.
    assert!(
        accepted <= cap,
        "accepted ({accepted}) must not exceed ring capacity ({cap})",
    );
    assert_eq!(backend.samples_emitted(), to_push as u64);
}

#[test]
fn mock_backend_no_drops_when_consumer_drains() {
    let cfg = AudioBackendConfig {
        sample_rate: 48_000,
        channels: 1,
        hop: 16,
        window: 64,
    };
    let cap = cfg.ring_capacity();
    let (producer, mut consumer) = rtrb::RingBuffer::<f32>::new(cap);

    let mut backend = MockAudioBackend::new(cfg, SampleSource::Sine { hz: 440.0 });
    backend.start(producer).expect("start backend");

    // Feed in chunks of `cap / 2`, draining fully between chunks. No drops
    // should ever happen.
    for _ in 0..8 {
        let _accepted = backend.feed(cap / 2);
        while consumer.pop().is_ok() {}
    }
    assert_eq!(backend.dropped_samples().load(Ordering::Relaxed), 0);
}
