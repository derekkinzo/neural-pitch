//! Phase 2.1 — Tauri / persistence integration tests for the analysis
//! surface.
//!
//! The Tauri command layer (`commands::analyze_recording` etc.) is a thin
//! `spawn_blocking` wrapper around `neural_pitch_core::store::*_blocking`.
//! These tests therefore drive the blocking helpers directly: the wire
//! contracts (cache hit shape, idempotent delete, list emptiness, error
//! mapping) are identical and exercising them via the helper avoids
//! standing up a full Tauri runtime.
//!
//! Strategy:
//!
//! * Seed the `analysis_cache` table by hand-crafting a synthetic
//!   `analysis::contour::ContourResult`, postcard-encoding it, and
//!   `library.upsert_analysis(...)`-ing the blob. This sidesteps the PYIN
//!   estimator (whose impl lands in a sibling task) so the cache surface
//!   can be validated independently.
//! * Drive `list_analyses_blocking`, `get_contour_blocking`,
//!   `delete_analysis_blocking` against the seeded row and assert the
//!   round-trip + idempotency contracts.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Mutex;

use neural_pitch_core::analysis::contour::{ContourResult as CoreContour, PYIN_ANALYZER_NAME};
use neural_pitch_core::pitch::F0Frame;
use neural_pitch_core::store::{
    AnalysisProgress, AnalysisSummary, NewRecording, ProgressSink, RecordingsLibrary,
    delete_analysis_blocking, get_contour_blocking, list_analyses_blocking,
};

const ANALYZER_VERSION: &str = "test-1";

/// Test-side `ProgressSink` used to assert the cached-path tick shape.
#[derive(Default)]
struct CapturingSink {
    captured: Mutex<Vec<AnalysisProgress>>,
}

impl ProgressSink for CapturingSink {
    fn emit(&self, progress: AnalysisProgress) {
        self.captured
            .lock()
            .expect("CapturingSink mutex poisoned")
            .push(progress);
    }
}

impl CapturingSink {
    fn snapshot(&self) -> Vec<AnalysisProgress> {
        self.captured
            .lock()
            .expect("CapturingSink mutex poisoned")
            .clone()
    }
}

/// Build a hermetic `(library, recording_id)` pair seeded with one
/// `analysis_cache` row containing a hand-crafted contour.
///
/// The seeded fixture FLAC is intentionally trivial (one hop's worth of
/// silence); the cache-hit path verifies the source FLAC still exists on
/// disk before returning a stale blob, so the file must be present even
/// though tests never decode it.
fn build_fixture(test_name: &str) -> (RecordingsLibrary, neural_pitch_core::store::RecordingId) {
    use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingSink};

    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test_name);
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    // Write a placeholder FLAC at the path the recording row points to.
    // analyze_recording_blocking gates the cache-hit path on
    // `flac_path.exists()` (see store/analysis_runtime.rs:231). Without a
    // real file the seeded test would surface FileMissing instead of the
    // intended cache hit. Using a real FlacRecordingSink keeps the file
    // valid for any future test that does want to decode it.
    let flac_path = tmp_root.join("fixture.flac");
    let mut sink = FlacRecordingSink::create(&flac_path, 48_000).expect("create flac sink");
    sink.write(&vec![0.0_f32; 256]).expect("write hop");
    Box::new(sink).finalize().expect("finalize fixture flac");

    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");

    // The recording row's `filename` is joined to `library.root()` to find
    // the FLAC. `library.root()` is the directory containing the SQLite
    // db; we created the db in the same dir as the FLAC so the join lines
    // up with the file we just wrote.
    let id = lib
        .insert_recording(NewRecording {
            filename: "fixture.flac".to_string(),
            created_at_unix_ms: 1_717_502_580_000,
            duration_ms: 1_000,
            sample_rate_hz: 48_000,
            channels: 1,
            bit_depth: 24,
            format: "flac".to_string(),
            a4_hz: 440.0,
            instrument_profile: "voice".to_string(),
            user_label: None,
        })
        .expect("insert recording");

    // Hand-roll a tiny ContourResult so we can postcard-encode it without
    // running the (still-stubbed) pYIN estimator. Three voiced frames at
    // 440 Hz with full confidence, plus the two scalar fields the cache
    // wire format pins.
    let frames = vec![
        F0Frame {
            f0_hz: 440.0,
            confidence: 0.99,
            voiced: true,
            timestamp_samples: 0,
        },
        F0Frame {
            f0_hz: 441.0,
            confidence: 0.98,
            voiced: true,
            timestamp_samples: 256,
        },
        F0Frame {
            f0_hz: 439.0,
            confidence: 0.97,
            voiced: true,
            timestamp_samples: 512,
        },
    ];
    let core_contour = CoreContour {
        frames,
        frame_rate_hz: 48_000.0 / 256.0,
        smoothed_cents: vec![0.0, 3.93, -3.93],
        voiced_ratio: 1.0,
        sample_count: 48_000,
        source_sample_rate_hz: 48_000,
        hop_size: 256,
        window_size: 1024,
    };
    let blob = postcard::to_allocvec(&core_contour).expect("postcard encode");
    lib.upsert_analysis(id, PYIN_ANALYZER_NAME, ANALYZER_VERSION, &blob)
        .expect("upsert analysis");

    (lib, id)
}

#[test]
fn list_analyses_returns_seeded_row_and_is_empty_after_delete() {
    let (lib, id) = build_fixture("analyze_recording_list_then_delete");

    let listed = list_analyses_blocking(&lib, id).expect("list_analyses must succeed");
    assert_eq!(
        listed.len(),
        1,
        "list_analyses must return the one seeded row; got {listed:?}"
    );
    let only = &listed[0];
    assert_eq!(only.analyzer_name, PYIN_ANALYZER_NAME);
    assert_eq!(only.analyzer_version, ANALYZER_VERSION);
    assert!(
        only.computed_at_unix_ms > 0,
        "computed_at_unix_ms must be a real Unix timestamp"
    );
    assert_eq!(only.result_format_version, 1);

    // Idempotent delete on a non-existent row first — should be Ok(()).
    delete_analysis_blocking(&lib, id, PYIN_ANALYZER_NAME, "no-such-version")
        .expect("delete on missing row must be idempotent Ok(())");

    // Delete the real row and confirm list returns empty.
    delete_analysis_blocking(&lib, id, PYIN_ANALYZER_NAME, ANALYZER_VERSION)
        .expect("delete on existing row must succeed");
    let listed_after = list_analyses_blocking(&lib, id).expect("list after delete must succeed");
    assert!(
        listed_after.is_empty(),
        "list_analyses must be empty after delete; got {listed_after:?}"
    );

    // Second delete is also idempotent.
    delete_analysis_blocking(&lib, id, PYIN_ANALYZER_NAME, ANALYZER_VERSION)
        .expect("second delete must remain idempotent Ok(())");
}

#[test]
fn get_contour_round_trips_cached_blob_and_misses_after_delete() {
    let (lib, id) = build_fixture("analyze_recording_get_contour_roundtrip");

    let contour = get_contour_blocking(&lib, id, PYIN_ANALYZER_NAME, ANALYZER_VERSION)
        .expect("get_contour must succeed against a seeded cache row")
        .expect("seeded row must produce Some(contour)");
    assert_eq!(contour.analyzer_name, PYIN_ANALYZER_NAME);
    assert_eq!(contour.analyzer_version, ANALYZER_VERSION);
    assert_eq!(contour.sample_rate_hz, 48_000);
    assert_eq!(contour.f0_hz.len(), 3);
    assert_eq!(contour.confidence.len(), 3);
    assert_eq!(contour.voiced.len(), 3);
    assert!((contour.f0_hz[0] - 440.0).abs() < f32::EPSILON);
    assert!(contour.voiced.iter().all(|v| *v));

    // Cache miss for an unknown version is `Ok(None)`, not an error.
    let missing = get_contour_blocking(&lib, id, PYIN_ANALYZER_NAME, "no-such-version")
        .expect("get_contour for unknown version must be Ok(None)");
    assert!(missing.is_none(), "missing row must surface as Ok(None)");

    // After delete, the original `(name, version)` is also a miss.
    delete_analysis_blocking(&lib, id, PYIN_ANALYZER_NAME, ANALYZER_VERSION)
        .expect("delete must succeed");
    let after_delete = get_contour_blocking(&lib, id, PYIN_ANALYZER_NAME, ANALYZER_VERSION)
        .expect("get_contour after delete must be Ok(None)");
    assert!(after_delete.is_none());
}

#[test]
fn analyze_recording_cache_hit_emits_single_terminal_tick() {
    use neural_pitch_core::store::analyze_recording_blocking;

    let (lib, id) = build_fixture("analyze_recording_cached_terminal_tick");
    let id_string = id.to_string();
    let sink = CapturingSink::default();

    let summary = analyze_recording_blocking(
        &lib,
        id,
        PYIN_ANALYZER_NAME,
        ANALYZER_VERSION,
        false,
        Some(&sink),
        None,
    )
    .expect("cache-hit analyze must succeed");

    assert!(
        summary.was_cached,
        "first call against a seeded row must report was_cached == true; got {summary:?}",
    );
    assert_eq!(summary.analyzer_name, PYIN_ANALYZER_NAME);
    assert_eq!(summary.analyzer_version, ANALYZER_VERSION);
    let expected_frame_rate = 48_000.0_f64 / 256.0_f64;
    assert!(
        (summary.frame_rate_hz - expected_frame_rate).abs() < 1e-6,
        "frame_rate_hz must equal sample_rate_hz / hop_size; got {}",
        summary.frame_rate_hz,
    );
    assert!(
        (0.0..=1.0).contains(&summary.voiced_ratio),
        "voiced_ratio must be in [0,1]; got {}",
        summary.voiced_ratio,
    );

    let ticks = sink.snapshot();
    assert_eq!(
        ticks.len(),
        1,
        "cache hit must emit exactly one progress tick; got {ticks:?}",
    );
    let only = &ticks[0];
    assert_eq!(only.recording_id, id_string);
    assert!(only.was_cached);
    assert!((only.percent - 1.0).abs() < f32::EPSILON);
    assert_eq!(only.frames_done, only.frames_total);
}

#[test]
fn analysis_summary_wire_shape_matches_ts_normaliser_contract() {
    // Contract test: the JSON the Rust shell produces for `AnalysisSummary`
    // MUST contain the keys the TS `normaliseSummary` adapter reads. This
    // pins the snake_case naming that `serde(rename_all = "snake_case")`
    // already enforces, plus the presence of `median_midi` / `median_cents_off`
    // / `median_hz_voiced` so a future field reshuffle does not silently
    // re-introduce the bug where the front-end card saw "C-1" because the
    // Rust shell did not emit a MIDI median at all.
    use neural_pitch_core::analysis::contour::ContourResult as CoreContour;

    let frames = vec![
        F0Frame {
            f0_hz: 440.0,
            confidence: 0.99,
            voiced: true,
            timestamp_samples: 0,
        },
        F0Frame {
            f0_hz: 441.0,
            confidence: 0.98,
            voiced: true,
            timestamp_samples: 256,
        },
    ];
    let core = CoreContour {
        frames,
        frame_rate_hz: 187.5,
        smoothed_cents: vec![0.0, 3.93],
        voiced_ratio: 1.0,
        sample_count: 96_000,
        source_sample_rate_hz: 48_000,
        hop_size: 256,
        window_size: 1024,
    };

    // Round-trip through postcard → analyze_recording's summarize_cached path.
    // We reach into the public API by seeding the cache and invoking
    // analyze_recording_blocking against a hermetic library.
    let (lib, id) = build_fixture("analysis_summary_wire_contract");
    let blob = postcard::to_allocvec(&core).expect("postcard encode");
    lib.upsert_analysis(id, PYIN_ANALYZER_NAME, "wire-contract-1", &blob)
        .expect("upsert");

    let summary = neural_pitch_core::store::analyze_recording_blocking(
        &lib,
        id,
        PYIN_ANALYZER_NAME,
        "wire-contract-1",
        false,
        None,
        None,
    )
    .expect("cache-hit analyze must succeed");

    let json = serde_json::to_value(&summary).expect("serialise summary to JSON");

    // Exact key set is part of the contract — both the existing snake_case
    // names (`median_hz_voiced`, `median_cents_off`) AND the new
    // `median_midi` field MUST be present.
    let obj = json.as_object().expect("summary must be a JSON object");
    for required_key in [
        "analyzer_name",
        "analyzer_version",
        "frame_rate_hz",
        "voiced_ratio",
        "median_hz_voiced",
        "median_midi",
        "median_cents_off",
        "computed_at_unix_ms",
        "was_cached",
    ] {
        assert!(
            obj.contains_key(required_key),
            "AnalysisSummary JSON missing required key {required_key:?}; \
             got keys: {:?}",
            obj.keys().collect::<Vec<_>>(),
        );
    }

    // median_midi for ~440 Hz (a4_hz=440) MUST be 69 (A4). This catches a
    // future regression where the Rust side stops populating the field.
    let median_midi = obj["median_midi"].as_i64();
    assert_eq!(
        median_midi,
        Some(69),
        "median_midi for 440 Hz with a4_hz=440 must be 69 (A4); got {median_midi:?}",
    );

    // The `AnalysisSummary` typed view must also expose the field directly.
    let via_struct: Option<i32> = summary.median_midi;
    assert!(
        via_struct.is_some(),
        "AnalysisSummary.median_midi must be populated for a voiced fixture"
    );
    let _ = AnalysisSummary {
        analyzer_name: "x".into(),
        analyzer_version: "y".into(),
        frame_rate_hz: 0.0,
        voiced_ratio: 0.0,
        median_hz_voiced: None,
        median_midi: None,
        median_cents_off: None,
        computed_at_unix_ms: 0,
        was_cached: false,
        range: None,
        vibrato: None,
    };
}
