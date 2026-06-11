#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args,
    clippy::cast_possible_wrap
)]

//! 48 kHz → 44.1 kHz internal resample, then back to 48 kHz at the
//! boundary.
//!
//! Generate 2 s of pink-noise-ish content at 48 kHz, run the
//! separator, and assert:
//!   1. `result.sample_rate_hz == 48_000` (the separator round-tripped
//!      back to the source rate).
//!   2. The summed-stems length matches the input length to within
//!      one sample (resampler boundary tolerance).

use neural_pitch_core::stems::StemSeparator;
use neural_pitch_core::test_utils::signals::white_noise;
use tokio_util::sync::CancellationToken;

const CAPTURE_SR_HZ: u32 = 48_000;
const DURATION_MS: u64 = 2_000;

#[ignore = "ort cpu-fallback path is too slow on the CI matrix; HTDEMUCS_MODEL_URL/SHA256 are also placeholders until the upstream commit is pinned, so this test only exercises a sideloaded model on the local gate"]
#[test]
fn stems_resamples_48khz_input_back_to_48khz_output() {
    let n_samples = (CAPTURE_SR_HZ as u64 * DURATION_MS / 1_000) as usize;
    let mono = white_noise(CAPTURE_SR_HZ, n_samples, 0xCAFE_BABE);

    let model_path = StemSeparator::ensure_model(|_| {})
        .expect("HTDemucs ONNX must be cached or downloadable on the local gate");
    let mut sep = StemSeparator::open(&model_path).expect("open HTDemucs session");

    let result = sep
        .separate(&mono, CAPTURE_SR_HZ, 1, |_| {}, &CancellationToken::new())
        .expect("separate must not error on 48 kHz pink noise");

    assert_eq!(
        result.sample_rate_hz,
        CAPTURE_SR_HZ,
        "result.sample_rate_hz must round-trip back to the source rate \
         (got {actual} Hz, expected {expected} Hz)",
        actual = result.sample_rate_hz,
        expected = CAPTURE_SR_HZ,
    );
    assert_eq!(result.channels, 1);

    // Summed-stems length must match the input length to within one
    // sample (rubato boundary tolerance).
    let drift = (result.vocals.len() as i64 - mono.len() as i64).abs();
    assert!(
        drift <= 1,
        "stems length drifted {drift} samples from the input length \
         (input={input}, vocals={vocals})",
        input = mono.len(),
        vocals = result.vocals.len(),
    );
    assert_eq!(result.drums.len(), result.vocals.len());
    assert_eq!(result.bass.len(), result.vocals.len());
    assert_eq!(result.other.len(), result.vocals.len());
}
