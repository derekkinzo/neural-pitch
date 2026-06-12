//! Fixture-driven deterministic test: feed a 440 Hz sine through the full
//! pipeline (MockAudioBackend → rtrb → DspWorker → ChannelFrameSink)
//! and assert the first voiced [`PitchUpdate`] is locked to A4.
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
fn dsp_pipeline_440hz_locks_to_a4() {
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

    // 200 ms of audio at 48 kHz = 9600 samples; feed in chunks the size of
    // a hop so the worker has steady-state input.
    let total = 9_600_usize;
    let mut fed = 0_usize;
    while fed < total {
        let chunk = cfg.hop.min(total - fed);
        backend.feed(chunk);
        fed += chunk;
    }

    // Wait for the first voiced update or 1 s wall clock, whichever comes
    // first.
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut first_voiced: Option<PitchUpdate> = None;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(update) => {
                if update.voiced {
                    first_voiced = Some(update);
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Top-up the ring; if we drained the whole feed already,
                // push a couple more hops so the worker has work.
                backend.feed(cfg.hop * 4);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    cancel.cancel();
    backend.stop();
    let _ = handle.join();

    let update = first_voiced.expect("at least one voiced PitchUpdate");
    assert_eq!(
        update.target_midi, 69,
        "first voiced update should target MIDI 69 (A4); got {update:?}"
    );
    assert!(
        update.smoothed_cents.abs() < 5.0,
        "smoothed_cents should be < 5; got {}",
        update.smoothed_cents
    );
}
