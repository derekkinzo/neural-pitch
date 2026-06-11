//! Insufficient-voicing returns the sentinel report.
//!
//! 30 voiced frames is below the 50-frame floor that `compute_range`
//! uses to guard against under-sampled histogram tails. The expected
//! output is exactly `RangeReport::insufficient()` — every numeric field
//! at zero, `voice_type_hint == None`. The `voiced_frame_count` is zero
//! on the sentinel (the report is opaque; callers MUST treat it as
//! "no answer", not "answer with 30 frames").

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::analysis::contour::ContourResult;
use neural_pitch_core::analysis::range::{RangeReport, compute_range};
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
fn range_insufficient_voiced_returns_sentinel() {
    let frames: Vec<F0Frame> = (0..30).map(|i| voiced_frame(220.0, i)).collect();
    let contour = make_contour(frames);

    let report = compute_range(&contour, 440.0);
    let sentinel = RangeReport::insufficient();

    assert_eq!(
        report, sentinel,
        "30 voiced frames (< 50) must return RangeReport::insufficient(); got {report:?}"
    );
    assert!(
        report.voice_type_hint.is_none(),
        "voice_type_hint must be None on the insufficient-data sentinel"
    );
}
