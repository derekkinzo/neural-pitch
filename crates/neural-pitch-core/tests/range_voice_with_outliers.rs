//! Comfortable-range trim discards 1% outliers.
//!
//! 990 voiced frames at 220 Hz (MIDI 57, A3) plus 10 voiced frames at
//! 880 Hz (MIDI 81, A5). The 1% trim used by the comfortable range MUST
//! discard the 10 high frames, leaving:
//!   * `comfortable_max_midi == 57` (1% of 1000 = 10 frames trimmed off
//!     the top tail; the 10 outliers fall just inside the trim).
//!   * `full_max_midi == 81` (0.1% of 1000 = 1 frame trimmed; the 10
//!     outliers survive the full-range trim).

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
    ContourResult {
        frames,
        frame_rate_hz: 93.75,
        smoothed_cents,
        voiced_ratio: 1.0,
        sample_count: (n as u64) * 512,
        source_sample_rate_hz: 48_000,
        hop_size: 512,
        window_size: 2048,
    }
}

#[test]
fn range_voice_with_outliers_trims_one_percent() {
    let mut frames: Vec<F0Frame> = (0..990).map(|i| voiced_frame(220.0, i)).collect();
    frames.extend((990..1000).map(|i| voiced_frame(880.0, i)));
    let contour = make_contour(frames);

    let report = compute_range(&contour, 440.0);

    assert_eq!(
        report.voiced_frame_count, 1000,
        "voiced_frame_count must include all voiced frames before any trimming"
    );

    // 1% trim: the 10 outliers at 880 Hz are exactly at the edge of the
    // top tail; the comfortable_max bin must collapse back to MIDI 57.
    assert_eq!(
        report.comfortable_max_midi, 57,
        "comfortable_max_midi must clip the 1% top tail back to MIDI 57; got {}",
        report.comfortable_max_midi
    );

    // 0.1% trim: only 1 frame discarded; the 10 outliers survive so the
    // full-range top end reaches MIDI 81 (A5).
    assert_eq!(
        report.full_max_midi, 81,
        "full_max_midi must keep the 880 Hz tail (0.1% trim); got {}",
        report.full_max_midi
    );
}
