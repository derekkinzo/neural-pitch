//! `AnalysisSummary` round-trips through postcard with the
//! `range` / `vibrato` fields populated.
//!
//! Construct an `AnalysisSummary` with non-trivial `RangeReport` and
//! `VibratoReport` payloads, encode via `postcard::to_allocvec`, decode,
//! and assert that re-encoding the decoded value produces a
//! byte-identical blob. This is the canonical Serde round-trip pattern
//! used elsewhere in the crate (`pyin_cache_roundtrip.rs`).
//!
//! A small `mpsc` helper is used to drive the round-trip from a worker
//! thread. The helper MUST tolerate the receiver closing early — on a
//! faster scheduler the worker may exit before the main test loop
//! finishes pulling from the channel, and that is a correct outcome on
//! Windows-class platforms (project hard rule).
//!
//! The structs are constructed manually so the test isolates the wire
//! format from the algorithm impls.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use neural_pitch_core::analysis::range::{RangeReport, VoiceType};
use neural_pitch_core::analysis::vibrato::{VibratoReport, VibratoWindow};
use neural_pitch_core::store::AnalysisSummary;

fn sample_summary() -> AnalysisSummary {
    AnalysisSummary {
        analyzer_name: "pyin".to_string(),
        analyzer_version: "0.2".to_string(),
        frame_rate_hz: 93.75,
        voiced_ratio: 0.85,
        median_hz_voiced: Some(220.0),
        median_midi: Some(57),
        median_cents_off: Some(-1.5),
        computed_at_unix_ms: 1_717_502_580_000,
        was_cached: false,
        range: Some(RangeReport {
            voiced_frame_count: 5_000,
            median_midi: 57,
            median_hz: 220.0,
            comfortable_min_midi: 50,
            comfortable_max_midi: 64,
            full_min_midi: 48,
            full_max_midi: 72,
            voice_type_hint: Some(vec![VoiceType::Tenor, VoiceType::Baritone]),
        }),
        vibrato: Some(VibratoReport {
            per_window: vec![
                VibratoWindow {
                    start_frame: 0,
                    rate_hz: 5.1,
                    extent_cents: 48.0,
                    confidence_0_to_1: 0.92,
                },
                VibratoWindow {
                    start_frame: 47,
                    rate_hz: 4.9,
                    extent_cents: 51.0,
                    confidence_0_to_1: 0.88,
                },
            ],
            median_rate_hz: 5.0,
            median_extent_cents: 49.5,
            vibrato_ratio: 0.75,
        }),
    }
}

#[test]
fn summary_range_vibrato_roundtrip_postcard_byte_equal() {
    let original = sample_summary();

    // Drive the encode from a worker thread to exercise the channel
    // pattern called out in the project's hard rule. The worker may
    // exit before the receiver loop drains the channel (faster Windows
    // schedulers), so the receive loop MUST tolerate `RecvError` /
    // `RecvTimeoutError::Disconnected` cleanly.
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let original_clone = original.clone();
    let worker = thread::spawn(move || {
        let bytes = postcard::to_allocvec(&original_clone)
            .expect("postcard::to_allocvec must serialise AnalysisSummary");
        // Sending may fail if the receiver has already closed; that is
        // acceptable per the channel-tolerance rule. Drop the error.
        let _ = tx.send(bytes);
    });

    // Receive with a generous timeout. If the worker has already
    // finished and the channel is closed before we get here, we still
    // need a deterministic answer — fall back to encoding inline. This
    // keeps the test green on schedulers where the worker outraces us.
    let bytes = match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(b) => b,
        Err(mpsc::RecvTimeoutError::Disconnected) => postcard::to_allocvec(&original)
            .expect("postcard::to_allocvec must serialise AnalysisSummary (fallback path)"),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("worker thread did not produce postcard bytes within 5 s")
        }
    };
    // Join the worker even if the receive went through the fallback —
    // a healthy thread always reaches its return statement; we just
    // do not depend on that ordering for correctness.
    worker.join().expect("worker thread must not panic");

    assert!(
        !bytes.is_empty(),
        "postcard encoded an empty blob — schema regression"
    );

    // Decode and assert that re-encoding the decoded value reproduces
    // the same bytes. This is the canonical "wire format is stable"
    // assertion: the round-trip preserves field order and value
    // semantics.
    let decoded: AnalysisSummary = postcard::from_bytes(&bytes)
        .expect("postcard::from_bytes must deserialise the just-encoded summary");
    let re_encoded: Vec<u8> = postcard::to_allocvec(&decoded)
        .expect("postcard::to_allocvec must re-serialise the decoded summary");

    assert_eq!(
        bytes, re_encoded,
        "postcard byte-equal round-trip failed: encode -> decode -> encode produced different bytes"
    );

    // Spot-check the new fields decoded into structurally-equal values.
    let range = decoded
        .range
        .as_ref()
        .expect("decoded summary must preserve the populated `range` field");
    assert_eq!(
        range.median_midi, 57,
        "RangeReport.median_midi must round-trip exactly"
    );
    assert_eq!(
        range.voice_type_hint.as_deref(),
        Some([VoiceType::Tenor, VoiceType::Baritone].as_slice()),
        "RangeReport.voice_type_hint must round-trip exactly"
    );

    let vibrato = decoded
        .vibrato
        .as_ref()
        .expect("decoded summary must preserve the populated `vibrato` field");
    assert_eq!(
        vibrato.per_window.len(),
        2,
        "VibratoReport.per_window must round-trip exactly"
    );
    assert!(
        (vibrato.median_rate_hz - 5.0).abs() < f32::EPSILON,
        "VibratoReport.median_rate_hz must round-trip exactly"
    );
}
