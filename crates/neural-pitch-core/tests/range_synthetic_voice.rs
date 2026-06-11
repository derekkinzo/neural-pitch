//! Vocal-range histogram on a single-pitch fixture.
//!
//! Construct a synthetic `ContourResult` of 100 voiced frames at exactly
//! 220 Hz with `a4_hz = 440.0`. The expected outputs are derived from the
//! algorithm memo:
//!   * `median_midi == 57` (A3 in `a4_hz=440` equal-temperament).
//!   * `comfortable_min_midi == comfortable_max_midi == 57` — every frame
//!     lands in the same 1-semitone histogram bin so the 1% trim cannot
//!     reach a different bin.
//!   * `voiced_frame_count == 100` — above the 50-frame insufficiency
//!     threshold, so the report is *not* the `insufficient()` sentinel.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::analysis::contour::ContourResult;
use neural_pitch_core::analysis::range::compute_range;
use neural_pitch_core::pitch::F0Frame;

fn voiced_frame(f0_hz: f32, idx: u64) -> F0Frame {
    F0Frame {
        f0_hz,
        confidence: 0.95,
        voiced: true,
        timestamp_samples: idx * 512,
    }
}

fn make_contour(frames: Vec<F0Frame>) -> ContourResult {
    let n = frames.len();
    let smoothed_cents = vec![0.0_f32; n];
    let voiced_count = frames.iter().filter(|f| f.voiced).count();
    let voiced_ratio = if n == 0 {
        0.0
    } else {
        voiced_count as f32 / n as f32
    };
    ContourResult {
        frames,
        frame_rate_hz: 93.75,
        smoothed_cents,
        voiced_ratio,
        sample_count: (n as u64) * 512,
        source_sample_rate_hz: 48_000,
        hop_size: 512,
        window_size: 2048,
    }
}

#[test]
fn range_synthetic_voice_single_pitch_220hz() {
    let frames: Vec<F0Frame> = (0..100).map(|i| voiced_frame(220.0, i)).collect();
    let contour = make_contour(frames);

    let report = compute_range(&contour, 440.0);

    assert_eq!(
        report.voiced_frame_count, 100,
        "voiced_frame_count must equal the number of voiced frames in the contour"
    );
    assert_eq!(
        report.median_midi, 57,
        "median_midi for a constant 220 Hz with a4_hz=440 must be 57 (A3); got {}",
        report.median_midi
    );
    assert_eq!(
        report.comfortable_min_midi, 57,
        "comfortable_min_midi for a single-pitch contour must collapse to the median bin"
    );
    assert_eq!(
        report.comfortable_max_midi, 57,
        "comfortable_max_midi for a single-pitch contour must collapse to the median bin"
    );
}
