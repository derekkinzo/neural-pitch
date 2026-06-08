//! Phase 4 — TDD-RED test for [`PromptSynth::render_wav`].
//!
//! Renders A4 (MIDI 69) for 500 ms at 48 kHz mono PCM16, parses the WAV
//! byte stream that comes back, runs the always-on [`YinMpmEstimator`] on
//! three centred analysis windows, and asserts the median estimated
//! frequency is within 1 cent of 440 Hz.
//!
//! TDD-RED status — [`PromptSynth::render_wav`] currently returns
//! `Err(SynthError::NotImplemented)`. This test therefore fails at
//! runtime; the Phase 4 GREEN step flips it green by wiring the actual
//! additive synth + WAV writer.
//!
//! Drill / training surface is default-on (no `feature = "neural"`
//! gate), so this test compiles against both the all-features and the
//! no-default-features matrices.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::pitch::yin::{YinAlgorithm, YinMpmEstimator};
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint, PitchEstimator};
use neural_pitch_core::poly::synth::{PROMPT_SAMPLE_RATE_HZ, PromptSynth};

/// Parse a minimal RIFF/WAVE PCM16 mono stream and return the decoded
/// `f32` samples plus the sample rate. Mirrors the inline WAV writer
/// the import-audio-file integration test uses; we re-implement the
/// reader here rather than pull in an audio decoder dev-dep.
fn parse_wav_pcm16_mono(bytes: &[u8]) -> (Vec<f32>, u32) {
    assert!(bytes.len() >= 44, "wav too short: {} bytes", bytes.len());
    assert_eq!(&bytes[0..4], b"RIFF", "missing RIFF magic");
    assert_eq!(&bytes[8..12], b"WAVE", "missing WAVE magic");
    assert_eq!(&bytes[12..16], b"fmt ", "missing fmt subchunk");
    let audio_format = u16::from_le_bytes([bytes[20], bytes[21]]);
    assert_eq!(audio_format, 1, "expected PCM (1), got {audio_format}");
    let num_channels = u16::from_le_bytes([bytes[22], bytes[23]]);
    assert_eq!(num_channels, 1, "expected mono, got {num_channels}");
    let sample_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    let bits_per_sample = u16::from_le_bytes([bytes[34], bytes[35]]);
    assert_eq!(
        bits_per_sample, 16,
        "expected 16-bit PCM, got {bits_per_sample}"
    );
    assert_eq!(&bytes[36..40], b"data", "missing data subchunk");
    let data_size = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as usize;
    assert!(
        bytes.len() >= 44 + data_size,
        "wav data length mismatch: header says {data_size} bytes, buffer has {}",
        bytes.len() - 44
    );
    let mut samples = Vec::with_capacity(data_size / 2);
    for chunk in bytes[44..44 + data_size].chunks_exact(2) {
        let raw = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(f32::from(raw) / f32::from(i16::MAX));
    }
    (samples, sample_rate)
}

#[test]
fn synthesize_prompt_a4_500ms_estimates_within_one_cent_of_440hz() {
    let mut synth = PromptSynth::new();
    let bytes = synth
        .render_wav(69, 500)
        .expect("PromptSynth::render_wav must succeed for A4 / 500 ms");

    let (samples, sample_rate) = parse_wav_pcm16_mono(&bytes);
    assert_eq!(
        sample_rate, PROMPT_SAMPLE_RATE_HZ,
        "synth must emit at PROMPT_SAMPLE_RATE_HZ; got {sample_rate}",
    );
    let expected_n = (u64::from(sample_rate) * 500) / 1_000;
    assert_eq!(
        samples.len() as u64,
        expected_n,
        "500 ms @ 48 kHz must produce {expected_n} samples; got {}",
        samples.len(),
    );

    let window_size = 2048_usize;
    let cfg = EstimatorConfig {
        sample_rate_hz: sample_rate,
        window_size,
        hop_size: window_size,
        fmin_hz: 60.0,
        fmax_hz: 1100.0,
        instrument_hint: Some(InstrumentHint::Voice),
    };
    let mut estimator =
        YinMpmEstimator::with_algorithm(cfg, YinAlgorithm::Yin).expect("construct YinMpm");

    // Run on three centred windows so any startup-envelope artefact
    // does not bias the median.
    assert!(
        samples.len() >= 3 * window_size,
        "need at least 3 windows of audio; got {} samples",
        samples.len(),
    );
    let centre = samples.len() / 2;
    let starts = [
        centre.saturating_sub(window_size + window_size / 2),
        centre.saturating_sub(window_size / 2),
        centre + window_size / 2,
    ];
    let mut estimates = Vec::with_capacity(starts.len());
    for &start in &starts {
        let window = &samples[start..start + window_size];
        let frame = estimator
            .process(window)
            .expect("process must succeed on synth window")
            .expect("voiced frame expected on a 440 Hz tone");
        assert!(
            frame.voiced,
            "frame {start} must be voiced on a 440 Hz tone; got {frame:?}",
        );
        estimates.push(frame.f0_hz);
    }
    estimates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let median_hz = estimates[estimates.len() / 2];

    let cents = 1200.0_f32 * (median_hz / 440.0).log2();
    assert!(
        cents.abs() < 1.0,
        "median estimated f0 must be within 1 cent of 440 Hz; got {median_hz} Hz \
         ({cents:+.3} cents) from samples {estimates:?}",
    );
}
