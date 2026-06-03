//! Tier-2 deterministic test: dropping the [`PitchUpdate`] receiver causes
//! the worker to exit cleanly with [`FrameSinkError::Disconnected`].
//!
//! The disconnect path is the only non-cancellation way for the worker to
//! exit on its own. A regression that turns the disconnect into a busy loop
//! or a panic would not be caught by the cancellation test, hence this
//! dedicated case.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use neural_pitch_core::audio::{AudioBackend, AudioBackendConfig, MockAudioBackend, SampleSource};
use neural_pitch_core::pipeline::{ChannelFrameSink, DspError, DspWorker, FrameSinkError};
use neural_pitch_core::pitch::factory::{Backend, make_estimator};
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};
use neural_pitch_core::smoothing::ContourSmoother;
use neural_pitch_core::voicing::VoiceActivityGate;
use tokio_util::sync::CancellationToken;

#[test]
fn dsp_pipeline_disconnect_exits_with_disconnected_error() {
    let cfg = AudioBackendConfig {
        sample_rate: 48_000,
        channels: 1,
        hop: 512,
        window: 2048,
    };
    let est_cfg = EstimatorConfig {
        sample_rate_hz: cfg.sample_rate,
        window_size: cfg.window,
        hop_size: cfg.hop,
        fmin_hz: 50.0,
        fmax_hz: 1500.0,
        instrument_hint: Some(InstrumentHint::Voice),
    };
    let estimator = make_estimator(Backend::YinMpm, est_cfg, None).expect("estimator");

    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(cfg.ring_capacity());
    let mut backend = MockAudioBackend::new(cfg.clone(), SampleSource::Sine { hz: 440.0 });
    backend.start(producer).expect("start backend");

    let (tx, rx) = mpsc::channel();
    let sink = Box::new(ChannelFrameSink::new(tx));
    let cancel = CancellationToken::new();
    let worker = DspWorker::new(
        cfg.clone(),
        estimator,
        ContourSmoother::new(50.0, cfg.sample_rate),
        VoiceActivityGate::new(0.005, 4),
        consumer,
        sink,
        cancel.clone(),
    );
    let handle = worker.spawn().expect("spawn worker");

    // Feed enough samples so the worker fills its window and emits at least
    // one update before we drop the receiver.
    backend.feed(cfg.hop * 8);

    // Wait for the first update to confirm the channel is live.
    let _ = rx
        .recv_timeout(Duration::from_millis(500))
        .expect("worker emitted at least one update");

    // Drop the receiver. The next `sink.send(...)` will return Disconnected.
    drop(rx);

    // Feed a few more hops so the worker advances past the next analysis
    // boundary and tries to send.
    backend.feed(cfg.hop * 8);

    // The worker must exit on its own (without `cancel.cancel()`) within a
    // bounded budget.
    let t0 = Instant::now();
    let join_deadline = t0 + Duration::from_millis(500);
    while Instant::now() < join_deadline {
        if handle.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    assert!(
        handle.is_finished(),
        "worker should have exited within 500 ms after rx was dropped",
    );

    let result = handle.join().expect("worker panicked");
    backend.stop();

    match result {
        Err(DspError::Sink(FrameSinkError::Disconnected)) => {}
        other => panic!("expected DspError::Sink(Disconnected), got {other:?}"),
    }
}
