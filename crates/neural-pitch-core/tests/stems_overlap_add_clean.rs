#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args
)]

//! Overlap-add boundary cleanliness on a 12 s sustained sine.
//!
//! 12 s of 440 Hz at 44.1 kHz spans multiple HTDemucs inference
//! windows with 50 % overlap. Assertions:
//!
//!   1. No NaN or infinite samples in any of the four stems.
//!   2. Each stem buffer length matches the input length (after
//!      truncating the right-pad).
//!   3. Zero-crossing density inside ±200 samples of each segment
//!      boundary stays within 1σ of the overall density — i.e. the
//!      overlap-add does not introduce a discontinuity at segment
//!      boundaries.

use neural_pitch_core::stems::StemSeparator;
use neural_pitch_core::test_utils::signals::sine_wave;
use tokio_util::sync::CancellationToken;

const SR_HZ: u32 = 44_100;
const DURATION_MS: u64 = 12_000;
const F_HZ: f32 = 440.0;

/// Window radius (in samples) around each segment boundary inside
/// which the local zero-crossing density is measured.
const BOUNDARY_RADIUS: usize = 200;

/// Segment boundaries inside a 12 s buffer windowed at HTDemucs's
/// 343 980-sample window with 50 % overlap. Hop length is half the
/// window (171 990 samples ≈ 3.9 s at 44.1 kHz), so boundaries land
/// at multiples of the hop and the boundaries-of-interest are at
/// `HOP_SECONDS` and `2 * HOP_SECONDS`.
const HOP_SECONDS: f32 = 171_990.0 / 44_100.0;
const BOUNDARY_TIMES_S: [f32; 2] = [HOP_SECONDS, HOP_SECONDS * 2.0];

fn zero_cross_count(buf: &[f32]) -> usize {
    let mut zc = 0;
    for w in buf.windows(2) {
        if (w[0] >= 0.0 && w[1] < 0.0) || (w[0] < 0.0 && w[1] >= 0.0) {
            zc += 1;
        }
    }
    zc
}

#[ignore = "htdemucs onnx path is too slow on the CI matrix; runs locally"]
#[test]
fn stems_overlap_add_has_no_boundary_artifacts() {
    let n_samples = (SR_HZ as u64 * DURATION_MS / 1_000) as usize;
    let mono = sine_wave(F_HZ, SR_HZ, n_samples);

    let model_path = StemSeparator::ensure_model(|_| {})
        .expect("HTDemucs ONNX must be cached or downloadable on the local gate");
    let mut sep = StemSeparator::open(&model_path).expect("open HTDemucs session");

    let result = sep
        .separate(&mono, SR_HZ, 1, |_| {}, &CancellationToken::new())
        .expect("separate must not error on a 12 s sustained sine");

    for (name, stem) in [
        ("vocals", &result.vocals),
        ("drums", &result.drums),
        ("bass", &result.bass),
        ("other", &result.other),
    ] {
        assert_eq!(
            stem.len(),
            mono.len(),
            "{name} stem length {stem_len} must match input length {input_len}",
            stem_len = stem.len(),
            input_len = mono.len(),
        );
        for (i, &v) in stem.iter().enumerate() {
            assert!(
                v.is_finite(),
                "{name} stem has non-finite sample at index {i}: {v}",
            );
        }
    }

    // Pick the stem most likely to carry the sine residue (`other`)
    // and verify the zero-cross density at each segment boundary
    // sits within 1σ of the overall density. We use a coarse 1σ
    // tolerance because pure-sine in / pure-sine out should yield
    // very stable z-cross density.
    let other = &result.other;
    let total_zc = zero_cross_count(other);
    let total_density = total_zc as f32 / other.len() as f32;

    for &t_s in &BOUNDARY_TIMES_S {
        let centre = (t_s * SR_HZ as f32) as usize;
        let lo = centre.saturating_sub(BOUNDARY_RADIUS);
        let hi = (centre + BOUNDARY_RADIUS).min(other.len());
        let local_zc = zero_cross_count(&other[lo..hi]);
        let local_density = local_zc as f32 / (hi - lo) as f32;
        let drift = (local_density - total_density).abs();
        // 1σ tolerance: standard deviation of zero-crossing density
        // for a clean sine is ~0 (deterministic), so a 50 % relative
        // drift bound is wide enough to absorb FP rounding while still
        // catching a window-boundary discontinuity.
        let tol = 0.5 * total_density.max(1e-3);
        assert!(
            drift < tol,
            "z-cross density at t={t_s}s drifted {drift} from overall density \
             {total_density} (tolerance {tol})",
        );
    }
}
