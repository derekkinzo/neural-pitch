//! 8-second windowing + Hann crossfade overlap-add reconstruction.
//!
//! HTDemucs canonical inference window is exactly 8.0 s at 44.1 kHz
//! ([`SEGMENT_SAMPLES`] = 352 800 per channel). Adjacent windows
//! overlap by 50 % ([`HOP_SAMPLES`] = 176 400) so every output
//! sample is covered by exactly two windows except at the head
//! and tail.
//!
//! Each segment's output is multiplied by a Hann window before
//! summation; two adjacent half-overlap Hann-windowed signals sum
//! to a constant in the steady state, so overlap-add is
//! unity-gain. Edge segments are renormalised by dividing by the
//! summed envelope to avoid amplitude rolloff at the head and
//! tail.

#![cfg(feature = "neural")]

use core::f32::consts::PI;

use tokio_util::sync::CancellationToken;
use tracing::instrument;

use crate::stems::StemError;
use crate::stems::htdemucs::Session;

/// Samples per channel in one HTDemucs inference window.
/// `8.0 s × 44 100 Hz = 352 800`.
pub const SEGMENT_SAMPLES: usize = 352_800;

/// Hop between adjacent inference windows, in samples per channel.
/// `4.0 s × 44 100 Hz = 176 400` — exactly 50 % of [`SEGMENT_SAMPLES`].
pub const HOP_SAMPLES: usize = 176_400;

/// Four reconstructed stem buffers, interleaved stereo at 44.1 kHz,
/// truncated to the original (un-padded) input length.
#[derive(Debug)]
#[must_use]
pub struct ReconstructedStems {
    /// Vocals stem, interleaved stereo.
    pub vocals: Vec<f32>,
    /// Drums stem, interleaved stereo.
    pub drums: Vec<f32>,
    /// Bass stem, interleaved stereo.
    pub bass: Vec<f32>,
    /// Other stem, interleaved stereo.
    pub other: Vec<f32>,
}

/// Pre-compute the Hann window of length `SEGMENT_SAMPLES`. Used
/// both as the per-segment crossfade weighting and as the envelope
/// the head/tail edge segments are normalised against.
///
/// Periodic form (`denominator = n`) so two adjacent half-overlap
/// windows sum to exactly `1.0` in the steady state (perfect COLA at
/// hop = n/2). Periodic Hann still hits `w[0] = 0`; the head/tail
/// edge protection is the half-window left/right pad inside
/// [`separate_overlap_add`], not the window shape.
fn hann_window() -> Vec<f32> {
    let n = SEGMENT_SAMPLES;
    let mut w = Vec::with_capacity(n);
    if n == 0 {
        return w;
    }
    let denom = n as f32;
    for k in 0..n {
        let x = 2.0 * PI * (k as f32) / denom;
        // Periodic Hann: 0.5 * (1 - cos(2π k / n)).
        w.push(0.5 - 0.5 * x.cos());
    }
    w
}

/// Run HTDemucs inference over `stereo_44k1` (interleaved stereo at
/// 44.1 kHz), splitting the input into 8 s windows with 50 % overlap
/// and reconstructing the four stems via Hann-window overlap-add.
///
/// `progress` is invoked with values in `[0.0, 1.0]` after each
/// segment. `cancel` is checked on entry and once per segment;
/// firing it returns [`StemError::Cancelled`] without producing a
/// partial result.
#[instrument(skip_all, fields(n_segments))]
pub fn separate_overlap_add<F: FnMut(f32)>(
    session: &mut Session,
    stereo_44k1: &[f32],
    mut progress: F,
    cancel: &CancellationToken,
) -> Result<ReconstructedStems, StemError> {
    if cancel.is_cancelled() {
        return Err(StemError::Cancelled);
    }
    if stereo_44k1.is_empty() {
        return Ok(ReconstructedStems {
            vocals: Vec::new(),
            drums: Vec::new(),
            bass: Vec::new(),
            other: Vec::new(),
        });
    }
    if stereo_44k1.len() % 2 != 0 {
        return Err(StemError::Configuration(format!(
            "stereo input length {} must be even",
            stereo_44k1.len()
        )));
    }
    let n_per_channel = stereo_44k1.len() / 2;
    // Pad with a leading half-window so every output sample of the
    // original input is covered by ≥ 2 windows (envelope > EPS at
    // sample 0). Without this, periodic-Hann's `w[0] = 0` would leave
    // sample 0 of every stem stranded below the EPS gate, producing a
    // tiny click at t=0. The matching tail right-pad keeps the last
    // input sample covered by the second-to-last window. Mirrors the
    // upstream Demucs reference behaviour.
    let lead_pad: usize = HOP_SAMPLES;
    // Right-pad to a multiple of HOP_SAMPLES so the last hop boundary
    // ends on a window that fully covers the tail.
    let body = lead_pad + n_per_channel;
    let n_segments = if body <= SEGMENT_SAMPLES {
        1
    } else {
        // ceil((body - SEGMENT) / HOP) + 1
        let extra = body - SEGMENT_SAMPLES;
        extra.div_ceil(HOP_SAMPLES) + 1
    };
    tracing::Span::current().record("n_segments", n_segments);
    let padded_per_channel = (n_segments - 1) * HOP_SAMPLES + SEGMENT_SAMPLES;
    let mut padded = vec![0.0_f32; padded_per_channel * 2];
    // Copy the input into the buffer starting at `lead_pad` so the
    // leading silence acts as the half-window head pad.
    padded[2 * lead_pad..2 * lead_pad + stereo_44k1.len()].copy_from_slice(stereo_44k1);

    let hann = hann_window();
    // Output buffers (interleaved stereo) and the per-sample summed
    // window envelope (mono — same envelope applies to both channels).
    let mut vocals = vec![0.0_f32; padded_per_channel * 2];
    let mut drums = vec![0.0_f32; padded_per_channel * 2];
    let mut bass = vec![0.0_f32; padded_per_channel * 2];
    let mut other = vec![0.0_f32; padded_per_channel * 2];
    let mut envelope = vec![0.0_f32; padded_per_channel];

    for seg_idx in 0..n_segments {
        if cancel.is_cancelled() {
            return Err(StemError::Cancelled);
        }
        let start = seg_idx * HOP_SAMPLES;
        let end = start + SEGMENT_SAMPLES;
        // Carve out the segment (already padded).
        let segment_slice = &padded[2 * start..2 * end];
        let segment_buf: Vec<f32> = segment_slice.to_vec();
        let stems = session.forward(&segment_buf)?;

        // Window-and-add into the four output buffers + envelope.
        accumulate(&stems.vocals, &hann, start, &mut vocals);
        accumulate(&stems.drums, &hann, start, &mut drums);
        accumulate(&stems.bass, &hann, start, &mut bass);
        accumulate(&stems.other, &hann, start, &mut other);
        for (i, &w) in hann.iter().enumerate() {
            envelope[start + i] += w;
        }

        let p = (seg_idx + 1) as f32 / n_segments as f32;
        progress(p.clamp(0.0, 1.0));
    }

    // Renormalise by the envelope to compensate for the head/tail
    // amplitude rolloff. Where the envelope sums to ~1 in the steady
    // state, this is a no-op; at the edges it lifts the signal back
    // up to unit gain.
    normalise_by_envelope(&mut vocals, &envelope);
    normalise_by_envelope(&mut drums, &envelope);
    normalise_by_envelope(&mut bass, &envelope);
    normalise_by_envelope(&mut other, &envelope);

    // Drop the leading half-window pad and truncate to the original
    // (un-padded) input length.
    let drop_lead = 2 * lead_pad;
    let target = stereo_44k1.len();
    let take = drop_lead + target;
    let trim = |mut v: Vec<f32>| -> Vec<f32> {
        if v.len() > take {
            v.truncate(take);
        }
        v.drain(..drop_lead.min(v.len()));
        if v.len() > target {
            v.truncate(target);
        }
        v
    };
    Ok(ReconstructedStems {
        vocals: trim(vocals),
        drums: trim(drums),
        bass: trim(bass),
        other: trim(other),
    })
}

/// Window-and-add one segment into the destination buffer.
///
/// `segment_stereo` is interleaved stereo of length `2 * SEGMENT_SAMPLES`;
/// `start` is the per-channel offset where this segment begins; `dst`
/// is the (longer) interleaved-stereo output buffer.
fn accumulate(segment_stereo: &[f32], hann: &[f32], start: usize, dst: &mut [f32]) {
    for (i, &w) in hann.iter().enumerate().take(SEGMENT_SAMPLES) {
        let off_src = 2 * i;
        let off_dst = 2 * (start + i);
        if off_dst + 1 >= dst.len() {
            break;
        }
        dst[off_dst] += w * segment_stereo[off_src];
        dst[off_dst + 1] += w * segment_stereo[off_src + 1];
    }
}

/// Divide each sample by the per-position envelope, leaving samples
/// where the envelope is below `EPS` untouched (avoids division by
/// zero at the very edges of the padded buffer).
fn normalise_by_envelope(buf: &mut [f32], envelope: &[f32]) {
    const EPS: f32 = 1e-6;
    let n = buf.len() / 2;
    let m = envelope.len().min(n);
    for i in 0..m {
        let e = envelope[i];
        if e <= EPS {
            continue;
        }
        let g = 1.0 / e;
        buf[2 * i] *= g;
        buf[2 * i + 1] *= g;
    }
}
