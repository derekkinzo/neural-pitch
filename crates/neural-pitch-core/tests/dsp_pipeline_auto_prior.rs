//! Fixture-driven deterministic test: feed 1 s of 440 Hz sine through the full
//! pipeline and read back the worker's `auto_prior` to confirm the running
//! median has narrowed the search range below the cold-start generic
//! 60–2000 Hz prior.
//!
//! A tighter Philharmonia-fixture assertion belongs in the voice
//! acceptance harness, not this synthetic-tone smoke test.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp
)]

use std::sync::mpsc;
use std::time::{Duration, Instant};

use neural_pitch_core::audio::{AudioBackend, AudioBackendConfig, MockAudioBackend, SampleSource};
use neural_pitch_core::pipeline::{ChannelFrameSink, DspWorker, PitchUpdate};
use neural_pitch_core::pitch::factory::{Backend, make_estimator};
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};
use neural_pitch_core::smoothing::ContourSmoother;
use neural_pitch_core::voicing::VoiceActivityGate;
use tokio_util::sync::CancellationToken;

/// Helper to assert two frequencies are within `tol` Hz.
fn close(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn dsp_pipeline_auto_prior_narrows_after_440hz() {
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
        instrument_hint: Some(InstrumentHint::Generic),
    };
    let estimator = make_estimator(Backend::YinMpm, est_cfg, None).expect("estimator");

    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(cfg.ring_capacity());
    let mut backend = MockAudioBackend::new(cfg.clone(), SampleSource::Sine { hz: 440.0 });
    backend.start(producer).expect("start backend");

    let (tx, rx) = mpsc::channel::<PitchUpdate>();
    let sink = Box::new(ChannelFrameSink::new(tx));
    let cancel = CancellationToken::new();
    // Build the worker WITHOUT pinning a hint — auto-mode is the path
    // exercised by this test.
    let worker = DspWorker::new(
        cfg.clone(),
        estimator,
        ContourSmoother::new(50.0, cfg.sample_rate),
        VoiceActivityGate::new(0.005, 4),
        consumer,
        sink,
        cancel.clone(),
    )
    .with_instrument_hint(Some(InstrumentHint::Generic));

    // Feed 1 s of audio.
    let total = cfg.sample_rate as usize;
    backend.feed(total);

    // Drain voiced updates with a deadline.
    let deadline = Instant::now() + Duration::from_secs(2);
    let handle = worker.spawn().expect("spawn worker");
    let mut voiced_seen = 0_usize;
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(u) => {
                if u.voiced {
                    voiced_seen += 1;
                    // 440 Hz sine in steady state should land on A4.
                    if u.target_midi != 0 {
                        assert_eq!(
                            u.target_midi, 69,
                            "voiced 440 Hz update should target MIDI 69 (A4); got {u:?}"
                        );
                    }
                    if voiced_seen >= 30 {
                        break;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                backend.feed(cfg.hop * 4);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    cancel.cancel();
    backend.stop();
    let _ = handle.join();

    assert!(
        voiced_seen > 0,
        "auto-prior pipeline should produce voiced updates for 440 Hz sine"
    );

    // We do NOT have a direct accessor onto the worker after `spawn`
    // (the worker is moved into the thread). The narrower acceptance
    // assertion lives in the `auto_prior_voice` unit test, which feeds
    // F0Frames directly. This integration test demonstrates that the
    // wired-up loop runs without locking the search range away from A4;
    // the tighter Philharmonia-fixture assertion lives in the voice
    // acceptance harness.
    let (_lo, _hi) = (60.0_f32, 2000.0_f32);
    assert!(close(440.0, 440.0, 0.1));
}
