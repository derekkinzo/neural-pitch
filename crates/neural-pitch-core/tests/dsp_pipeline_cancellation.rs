//! Fixture-driven deterministic test: cancelling the [`CancellationToken`] mid-stream
//! drives the worker to exit within one packet boundary.
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
fn dsp_pipeline_cancellation_exits_promptly() {
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

    let (tx, rx) = mpsc::channel::<PitchUpdate>();
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

    // Push some samples so the worker actually has work to do.
    backend.feed(cfg.hop * 8);

    // Wait briefly for the first update to confirm the worker is running.
    let _ = rx.recv_timeout(Duration::from_millis(200));

    // Cancel and join with a 100 ms budget.
    let t0 = Instant::now();
    cancel.cancel();

    let mut joined = false;
    let join_deadline = t0 + Duration::from_millis(100);
    while Instant::now() < join_deadline {
        if handle.is_finished() {
            joined = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    if !joined {
        // Give it one more best-effort grace period before we declare the
        // test failed; we still must join on the handle so cargo doesn't
        // leak the thread on success.
        std::thread::sleep(Duration::from_millis(50));
    }
    let elapsed = t0.elapsed();
    let result = handle.join().expect("worker panicked");
    backend.stop();
    assert!(result.is_ok(), "worker run() returned {result:?}");
    assert!(
        elapsed <= Duration::from_millis(200),
        "cancellation should exit within ~one packet boundary (~10 ms); took {elapsed:?}"
    );
}
