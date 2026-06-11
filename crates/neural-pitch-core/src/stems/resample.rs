//! Sample-rate conversion + channel reshaping for the stem
//! separator.
//!
//! HTDemucs ONNX expects 44.1 kHz stereo at `[1, 2, T]`. The caller
//! may hand us audio at any sample rate (typical 48 kHz capture, or
//! 22.05 kHz from a Basic Pitch round-trip) at any channel count
//! (1 or 2; >2 is rejected with [`StemError::Configuration`]).
//!
//! Uses [`rubato::FftFixedIn`] for the sample-rate conversion (chosen
//! for compatibility with the existing `poly::basic_pitch` resampler
//! call site — handles arbitrary input lengths via `process_partial`).
//! Mono → stereo is a channel duplication; >2 channels is an error.

#![cfg(feature = "neural")]

use rubato::{FftFixedIn, Resampler};

use crate::stems::{HTDEMUCS_SR_HZ, StemError};

/// Constant exposed for tests: the model's required input rate.
pub const TARGET_SAMPLE_RATE_HZ: u32 = HTDEMUCS_SR_HZ;

/// Internal resampler chunk size, in input samples per channel.
/// Matches the constant `poly::basic_pitch` uses, keeping FFT cost
/// bounded per `process` call.
const RESAMPLE_CHUNK_IN: usize = 4_096;

/// Resample one mono channel from `src_rate` to `dst_rate`. Pass
/// through (zero-copy clone) when the rates match.
fn resample_mono(input: &[f32], src_rate: u32, dst_rate: u32) -> Result<Vec<f32>, StemError> {
    if src_rate == 0 || dst_rate == 0 {
        return Err(StemError::Configuration(
            "sample rate must be greater than zero".to_string(),
        ));
    }
    if input.is_empty() {
        return Ok(Vec::new());
    }
    if src_rate == dst_rate {
        return Ok(input.to_vec());
    }
    let mut resampler = FftFixedIn::<f32>::new(
        src_rate as usize,
        dst_rate as usize,
        RESAMPLE_CHUNK_IN,
        2,
        1,
    )
    .map_err(|e| StemError::Configuration(format!("rubato construct: {e}")))?;
    let expected =
        input.len().saturating_mul(dst_rate as usize) / src_rate as usize + RESAMPLE_CHUNK_IN;
    let mut out: Vec<f32> = Vec::with_capacity(expected);
    let mut idx: usize = 0;
    while idx + RESAMPLE_CHUNK_IN <= input.len() {
        let waves_in: [&[f32]; 1] = [&input[idx..idx + RESAMPLE_CHUNK_IN]];
        let waves_out = resampler
            .process(&waves_in, None)
            .map_err(|e| StemError::Configuration(format!("rubato process: {e}")))?;
        out.extend_from_slice(&waves_out[0]);
        idx += RESAMPLE_CHUNK_IN;
    }
    if idx < input.len() {
        let waves_in: [&[f32]; 1] = [&input[idx..]];
        let waves_out = resampler
            .process_partial(Some(&waves_in), None)
            .map_err(|e| StemError::Configuration(format!("rubato process_partial: {e}")))?;
        out.extend_from_slice(&waves_out[0]);
    }
    // Always flush the FFT tail buffer — when `input.len()` is an exact
    // multiple of RESAMPLE_CHUNK_IN, the loop above never calls
    // `process_partial`, so a few output samples retained inside the
    // resampler's internal buffer would otherwise be lost.
    //
    // Pass `None` for `wave_in` so rubato pads with silence internally
    // (an explicit `Some(empty)` trips a stricter buffer-size check on
    // some FftFixedIn shapes). We only consume the flushed output up to
    // the rate-ratio target length so independent per-channel resamples
    // do not accumulate sub-sample drift.
    let target_len = input.len().saturating_mul(dst_rate as usize) / src_rate as usize;
    if out.len() < target_len {
        let waves_out: Vec<Vec<f32>> = resampler
            .process_partial::<&[f32]>(None, None)
            .map_err(|e| StemError::Configuration(format!("rubato flush: {e}")))?;
        let take = target_len.saturating_sub(out.len()).min(waves_out[0].len());
        out.extend_from_slice(&waves_out[0][..take]);
    }
    if out.len() > target_len {
        out.truncate(target_len);
    }
    Ok(out)
}

/// De-interleave an interleaved stereo buffer into two mono planes.
fn de_interleave_stereo(input: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let n = input.len() / 2;
    let mut l = Vec::with_capacity(n);
    let mut r = Vec::with_capacity(n);
    for chunk in input.chunks_exact(2) {
        l.push(chunk[0]);
        r.push(chunk[1]);
    }
    (l, r)
}

/// Interleave two equal-length mono planes into a single stereo
/// buffer.
fn interleave_stereo(left: &[f32], right: &[f32]) -> Vec<f32> {
    let n = left.len().min(right.len());
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        out.push(left[i]);
        out.push(right[i]);
    }
    out
}

/// Convert an input PCM buffer at `(sample_rate_hz, channels)` to
/// 44.1 kHz interleaved stereo, ready for HTDemucs inference.
///
/// Accepted channel counts: `1` (mono — duplicated to stereo) or
/// `2` (stereo — passthrough channel count). >2 channels returns
/// [`StemError::Configuration`].
pub fn to_htdemucs_input(
    input: &[f32],
    sample_rate_hz: u32,
    channels: u32,
) -> Result<Vec<f32>, StemError> {
    if !matches!(channels, 1 | 2) {
        return Err(StemError::Configuration(format!(
            "unsupported channel count: {channels} (expected 1 or 2)"
        )));
    }
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let (left, right) = if channels == 1 {
        let mono = resample_mono(input, sample_rate_hz, TARGET_SAMPLE_RATE_HZ)?;
        (mono.clone(), mono)
    } else {
        if input.len() % 2 != 0 {
            return Err(StemError::Configuration(format!(
                "stereo buffer length {} must be even",
                input.len()
            )));
        }
        let (l, r) = de_interleave_stereo(input);
        let l = resample_mono(&l, sample_rate_hz, TARGET_SAMPLE_RATE_HZ)?;
        let r = resample_mono(&r, sample_rate_hz, TARGET_SAMPLE_RATE_HZ)?;
        let n = l.len().min(r.len());
        (l[..n].to_vec(), r[..n].to_vec())
    };

    Ok(interleave_stereo(&left, &right))
}

/// Convert a 44.1 kHz stereo (interleaved) buffer back to the
/// caller's `(sample_rate_hz, channels)` contract.
///
/// Stereo → mono: average channels. Stereo → stereo: pass through
/// (with optional rate change). Stereo → mono with a rate change:
/// down-mix first, then resample.
pub fn from_htdemucs_output(
    stereo_44k1: &[f32],
    target_sample_rate_hz: u32,
    target_channels: u32,
) -> Result<Vec<f32>, StemError> {
    if !matches!(target_channels, 1 | 2) {
        return Err(StemError::Configuration(format!(
            "unsupported target channel count: {target_channels} (expected 1 or 2)"
        )));
    }
    if stereo_44k1.is_empty() {
        return Ok(Vec::new());
    }
    if stereo_44k1.len() % 2 != 0 {
        return Err(StemError::Configuration(format!(
            "stereo input length {} must be even",
            stereo_44k1.len()
        )));
    }

    let (left, right) = de_interleave_stereo(stereo_44k1);
    if target_channels == 1 {
        // Down-mix first, then resample (cheaper than two separate
        // resamples followed by a sum).
        let n = left.len().min(right.len());
        let mut mono = Vec::with_capacity(n);
        for i in 0..n {
            mono.push(0.5 * (left[i] + right[i]));
        }
        return resample_mono(&mono, TARGET_SAMPLE_RATE_HZ, target_sample_rate_hz);
    }
    // Stereo → stereo.
    let l = resample_mono(&left, TARGET_SAMPLE_RATE_HZ, target_sample_rate_hz)?;
    let r = resample_mono(&right, TARGET_SAMPLE_RATE_HZ, target_sample_rate_hz)?;
    let n = l.len().min(r.len());
    Ok(interleave_stereo(&l[..n], &r[..n]))
}
