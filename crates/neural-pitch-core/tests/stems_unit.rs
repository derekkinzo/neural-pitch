#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap
)]

//! Phase 5 — deterministic non-ONNX stems unit tests.
//!
//! Covers the pure-Rust paths that do NOT need a real HTDemucs model
//! and therefore must run on the default CI matrix (no `#[ignore]`):
//!
//!   * `resample::to_htdemucs_input` / `from_htdemucs_output`
//!     - mono / stereo passthrough at matching sample rates
//!     - 48 kHz → 44.1 kHz length math is within ±1 sample
//!     - input length that is an exact multiple of the resampler
//!       chunk size (the FftFixedIn tail-flush regression)
//!     - channel-count rejection (channels = 0 / 3) → Configuration
//!     - empty-input → empty-output passthrough
//!   * `download::ensure_at` cache-hit branch when the SHA constant is
//!     a placeholder (the path that today returns the on-disk file
//!     unchecked because the verified upstream URL is not pinned yet).

use std::fs;

use neural_pitch_core::stems::resample::{
    TARGET_SAMPLE_RATE_HZ, from_htdemucs_output, to_htdemucs_input,
};
use neural_pitch_core::stems::{HTDEMUCS_SR_HZ, StemError, download};

const SR_44K1: u32 = 44_100;
const SR_48K: u32 = 48_000;

fn ramp(n: usize) -> Vec<f32> {
    (0..n).map(|i| (i as f32) / (n as f32)).collect()
}

#[test]
fn resample_mono_passthrough_when_rates_match() {
    // Mono at 44.1 kHz duplicated to interleaved stereo at 44.1 kHz.
    let mono = ramp(1024);
    let out = to_htdemucs_input(&mono, SR_44K1, 1).expect("to_htdemucs_input");
    assert_eq!(out.len(), mono.len() * 2);
    for (i, sample) in mono.iter().enumerate() {
        assert!((out[2 * i] - *sample).abs() < f32::EPSILON);
        assert!((out[2 * i + 1] - *sample).abs() < f32::EPSILON);
    }
    assert_eq!(TARGET_SAMPLE_RATE_HZ, HTDEMUCS_SR_HZ);
}

#[test]
fn resample_stereo_passthrough_when_rates_match() {
    // Interleaved stereo at 44.1 kHz: passthrough.
    let stereo: Vec<f32> = (0..1024)
        .flat_map(|i| {
            let v = (i as f32) / 1024.0;
            [v, -v]
        })
        .collect();
    let out = to_htdemucs_input(&stereo, SR_44K1, 2).expect("stereo passthrough");
    assert_eq!(out, stereo);
}

#[test]
fn resample_48k_to_44k1_length_within_one_sample() {
    // Real resample arc — the 48 kHz mono buffer is duplicated to
    // stereo and resampled to 44.1 kHz. Length must equal
    // `n * 44_100 / 48_000` to within ±1 sample per channel.
    let mono = ramp(48_000); // 1 s at 48 kHz
    let out = to_htdemucs_input(&mono, SR_48K, 1).expect("48k → 44.1k");
    assert!(!out.is_empty());
    assert!(out.len() % 2 == 0, "stereo interleaved buffer must be even");
    let per_channel = out.len() / 2;
    let expected = mono.len() * SR_44K1 as usize / SR_48K as usize;
    let drift = (per_channel as i64 - expected as i64).abs();
    assert!(
        drift <= 1,
        "expected {expected} samples per channel; got {per_channel} (drift {drift})",
    );
}

#[test]
fn resample_input_length_exact_chunk_multiple_does_not_drop_tail() {
    // FftFixedIn's chunk size is 4 096; a length that is an exact
    // multiple of 4 096 used to skip the trailing process_partial flush
    // and silently lose a handful of output samples. The fix:
    // unconditionally flush after the loop — verify the output length
    // tracks the rate ratio within the documented ±1 sample window.
    let n = 4_096 * 6; // 6 chunks exactly
    let mono = ramp(n);
    let out = to_htdemucs_input(&mono, SR_48K, 1).expect("exact-chunk resample");
    let per_channel = out.len() / 2;
    let expected = n * SR_44K1 as usize / SR_48K as usize;
    let drift = (per_channel as i64 - expected as i64).abs();
    assert!(
        drift <= 1,
        "exact-chunk-multiple resample lost samples: expected {expected}, got {per_channel}",
    );
}

#[test]
fn resample_rejects_channels_zero() {
    let buf = ramp(64);
    let err = to_htdemucs_input(&buf, SR_44K1, 0).expect_err("channels=0 must error");
    assert!(matches!(err, StemError::Configuration(_)));
}

#[test]
fn resample_rejects_channels_three() {
    let buf = ramp(64);
    let err = to_htdemucs_input(&buf, SR_44K1, 3).expect_err("channels=3 must error");
    assert!(matches!(err, StemError::Configuration(_)));
}

#[test]
fn resample_empty_input_returns_empty_output() {
    let out = to_htdemucs_input(&[], SR_48K, 1).expect("empty input");
    assert!(out.is_empty());
    let out = from_htdemucs_output(&[], SR_48K, 1).expect("empty stereo input");
    assert!(out.is_empty());
}

#[test]
fn from_htdemucs_output_rejects_odd_length() {
    // Stereo interleaved buffer with odd length is malformed.
    let buf = ramp(7);
    let err = from_htdemucs_output(&buf, SR_44K1, 2).expect_err("odd stereo length must error");
    assert!(matches!(err, StemError::Configuration(_)));
}

#[test]
fn ensure_at_cache_hit_with_placeholder_sha_returns_existing_file() {
    // The placeholder-SHA branch trusts an existing on-disk file and
    // returns it verbatim. This is the only path that lets the integration
    // tests work today — verify it actually finds a pre-staged file.
    let dir = std::env::temp_dir().join(format!("stems_unit_cache_hit_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp models dir");
    let target = dir.join(download::MODEL_FILENAME);
    fs::write(&target, b"sentinel-bytes").expect("seed cached model");
    let resolved = download::ensure_at(&dir, |_| {}).expect("placeholder cache hit");
    assert_eq!(resolved, target, "ensure_at must return the cached path");
    let _ = fs::remove_dir_all(&dir);
}
