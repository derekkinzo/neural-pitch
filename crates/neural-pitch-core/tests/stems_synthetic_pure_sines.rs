#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args
)]

//! Stems separation on synthetic pure-sine input.
//!
//! 4 s of `sine 220 Hz @ 0.5` summed with `sine 440 Hz @ 0.5` at
//! 44.1 kHz, duplicated to interleaved stereo. There is no vocal
//! content in the input, so the `vocals` stem must be near-silent;
//! the residual content (drums + bass + other, summed) must
//! reconstruct the input within a small RMSE.

use neural_pitch_core::stems::StemSeparator;
use neural_pitch_core::test_utils::signals::sine_wave;
use tokio_util::sync::CancellationToken;

const SR_HZ: u32 = 44_100;
const TONE_DURATION_MS: u64 = 4_000;

fn rms(buf: &[f32]) -> f32 {
    if buf.is_empty() {
        return 0.0;
    }
    let s: f32 = buf.iter().map(|v| v * v).sum();
    (s / buf.len() as f32).sqrt()
}

#[ignore = "ort cpu-fallback path is too slow on the CI matrix; HTDEMUCS_MODEL_URL/SHA256 are also placeholders until the upstream commit is pinned, so this test only exercises a sideloaded model on the local gate"]
#[test]
fn stems_pure_sines_have_silent_vocals_and_reconstructable_residual() {
    let n_samples = (SR_HZ as u64 * TONE_DURATION_MS / 1_000) as usize;
    let a = sine_wave(220.0, SR_HZ, n_samples);
    let b = sine_wave(440.0, SR_HZ, n_samples);
    // Equal-amplitude sum at 0.5 each, then interleave to stereo.
    let mut stereo = Vec::with_capacity(n_samples * 2);
    for i in 0..n_samples {
        let s = 0.5 * a[i] + 0.5 * b[i];
        stereo.push(s);
        stereo.push(s);
    }

    let model_path = StemSeparator::ensure_model(|_| {})
        .expect("HTDemucs ONNX must be cached or downloadable on the local gate");
    let mut sep = StemSeparator::open(&model_path).expect("open HTDemucs session");

    let result = sep
        .separate(&stereo, SR_HZ, 2, |_| {}, &CancellationToken::new())
        .expect("separate must not error on a clean pure-sine stereo input");

    assert_eq!(result.sample_rate_hz, SR_HZ);
    assert_eq!(result.channels, 2);

    let input_rms = rms(&stereo);
    let vocals_rms = rms(&result.vocals);
    assert!(
        vocals_rms < 0.1 * input_rms,
        "vocals stem must be near-silent on a pure-sine input: \
         vocals_rms={vocals_rms} input_rms={input_rms}",
    );

    // Sum drums + bass + other; must reconstruct the input within a
    // small RMSE. We do not include `vocals` in the residual because
    // a model that bleeds tonal content into vocals would otherwise
    // pass this assertion via cancellation against the residual.
    let mut residual = vec![0.0_f32; stereo.len()];
    for (i, slot) in residual.iter_mut().enumerate() {
        *slot = result.drums[i] + result.bass[i] + result.other[i];
    }
    let mut diff = vec![0.0_f32; stereo.len()];
    for (i, slot) in diff.iter_mut().enumerate() {
        *slot = stereo[i] - residual[i];
    }
    let rmse = rms(&diff);
    assert!(
        rmse < 0.05,
        "drums+bass+other must reconstruct the pure-sine input within RMSE 0.05; got {rmse}",
    );
}
