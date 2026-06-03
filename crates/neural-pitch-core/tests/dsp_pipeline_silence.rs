//! Tier-2 deterministic test: pure silence through the full pipeline must
//! never be reported as voiced.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use neural_pitch_core::audio::{AudioBackend, AudioBackendConfig, MockAudioBackend, SampleSource};
use neural_pitch_core::pipeline::{ChannelFrameSink, DspWorker, PitchUpdate};
use neural_pitch_core::pitch::factory::{Backend, make_estimator};
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};
use neural_pitch_core::smoothing::ContourSmoother;
use neural_pitch_core::voicing::VoiceActivityGate;
use tokio_util::sync::CancellationToken;

#[test]
fn dsp_pipeline_silence_never_voiced() {
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
    let mut backend = MockAudioBackend::new(cfg.clone(), SampleSource::Silence);
    backend.start(producer).expect("start backend");

    let (tx, rx) = mpsc::channel::<PitchUpdate>();
    let sink = Box::new(ChannelFrameSink::new(tx));
    let cancel = CancellationToken::new();
    let worker = DspWorker::new(
        cfg.clone(),
        estimator,
        ContourSmoother::new(50.0, cfg.sample_rate),
        VoiceActivityGate::new(0.01, 4),
        consumer,
        sink,
        cancel.clone(),
    );
    let handle = worker.spawn().expect("spawn worker");

    // Push 200 ms of silence.
    let total = 9_600_usize;
    let mut fed = 0_usize;
    while fed < total {
        let chunk = cfg.hop.min(total - fed);
        backend.feed(chunk);
        fed += chunk;
    }

    // Drain everything the worker emits within a 500 ms budget.
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut updates: Vec<PitchUpdate> = Vec::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(u) => updates.push(u),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Make sure the worker has had a chance to drain everything
                // by feeding a few more silent hops before we give up.
                backend.feed(cfg.hop * 4);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    cancel.cancel();
    backend.stop();
    let _ = handle.join();

    assert!(
        !updates.is_empty(),
        "worker should emit at least one update during silence"
    );
    for (i, u) in updates.iter().enumerate() {
        assert!(
            !u.voiced,
            "silence must never be reported as voiced (update #{i}: {u:?})"
        );
    }
}
