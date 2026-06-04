//! Phase 2.1 TDD-RED: bumping `PYIN_ANALYZER_VERSION` invalidates old rows.
//!
//! Schema invariant: `analysis_cache` rows are keyed on
//! `(recording_id, analyzer_name, analyzer_version)`. A row written at
//! version `"0.0"` MUST appear as a *cache miss* when the caller asks for
//! version `"0.1"` (and vice-versa) — the version bump is what forces a
//! re-analyse on next access. The test sequences:
//!
//! 1. Insert a recording.
//! 2. Upsert a postcard blob produced by `analyze_contour` at version
//!    `"0.0"` (a hand-tagged legacy version).
//! 3. `get_analysis(.., "0.1")` → `None`. (cache miss across versions.)
//! 4. Run `analyze_contour` again, upsert at version `"0.1"`.
//! 5. `get_analysis(.., "0.1")` → `Some(latest_blob)`. (cache hit.)
//! 6. The original `"0.0"` row is still there — versions are independent.
//!
//! `analyze_contour` is `todo!()` until Phase 2.1 implementation lands; the
//! first call here panics, which is the red signal.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::Path;

use neural_pitch_core::analysis::contour::{
    PYIN_ANALYZER_NAME, PYIN_ANALYZER_VERSION, analyze_contour,
};
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint};
use neural_pitch_core::store::{NewRecording, RecordingsLibrary};
use neural_pitch_core::test_utils::signals::sine_wave;

#[test]
fn pyin_analyzer_version_bump_invalidates_cache() {
    // Assert the active wire-version constant. Phase 2.1 cold-start shipped
    // at "0.1"; the bump to "0.2" added `hop_size` / `window_size` fields
    // to `ContourResult` (the per-blob analyzer params, not the live-tuner
    // defaults). Any further bump MUST move in lock-step with a wire-shape
    // change to ContourResult so old blobs do not silently decode against
    // a new layout.
    assert_eq!(
        PYIN_ANALYZER_VERSION, "0.2",
        "PYIN_ANALYZER_VERSION must match the current ContourResult wire shape; \
         bumping the constant requires a corresponding wire-format-version note"
    );
    assert_eq!(
        PYIN_ANALYZER_NAME, "pyin",
        "PYIN_ANALYZER_NAME is part of the cache key; renaming it would \
         orphan every previously-cached row"
    );

    let cfg = EstimatorConfig {
        sample_rate_hz: 48_000,
        window_size: 4096,
        hop_size: 1024,
        fmin_hz: 60.0,
        fmax_hz: 1100.0,
        instrument_hint: Some(InstrumentHint::Voice),
    };
    // 0.5 s of clean 440 Hz keeps the test fast while still producing a
    // non-trivial contour blob.
    let samples = sine_wave(440.0, cfg.sample_rate_hz, (cfg.sample_rate_hz / 2) as usize);

    // Build the postcard-encoded blob via the production analyzer. This
    // panics until Phase 2.1 is wired — the TDD-RED signal.
    let result = analyze_contour(&samples, &cfg, 440.0)
        .expect("analyze_contour should succeed once Phase 2.1 ships");
    let blob_v0_1: Vec<u8> =
        postcard::to_allocvec(&result).expect("postcard::to_allocvec must serialise ContourResult");

    // Synthesise a "legacy" 0.0 blob — this would be a previously-cached
    // result from before the version bump. The actual bytes are
    // intentionally distinct from the 0.1 blob so the cache-miss assertion
    // cannot pass on accident.
    let blob_v0_0: Vec<u8> = b"pyin v=0.0 placeholder legacy bytes".to_vec();

    let lib = RecordingsLibrary::new(Path::new(":memory:"))
        .expect("opening :memory: library should succeed");

    let id = lib
        .insert_recording(NewRecording {
            filename: "version_bump_target.flac".to_string(),
            created_at_unix_ms: 1_717_500_000_000,
            duration_ms: 500,
            sample_rate_hz: 48_000,
            channels: 1,
            bit_depth: 24,
            format: "flac".to_string(),
            a4_hz: 440.0,
            instrument_profile: "voice".to_string(),
            user_label: None,
        })
        .expect("insert_recording should succeed");

    // 1. Write the legacy 0.0 blob.
    lib.upsert_analysis(id, PYIN_ANALYZER_NAME, "0.0", &blob_v0_0)
        .expect("upsert_analysis(v=0.0) should succeed");

    // 2. Asking for the active version is a miss — the row is at 0.0.
    let miss = lib
        .get_analysis(id, PYIN_ANALYZER_NAME, PYIN_ANALYZER_VERSION)
        .expect("get_analysis(v=0.1) should not error");
    assert!(
        miss.is_none(),
        "0.0-only cache returned a hit when caller asked for 0.1; \
         analyzer_version bump did not invalidate the row"
    );

    // 3. Write the active 0.1 blob.
    lib.upsert_analysis(id, PYIN_ANALYZER_NAME, PYIN_ANALYZER_VERSION, &blob_v0_1)
        .expect("upsert_analysis(v=0.1) should succeed");

    // 4. Asking for the active version is now a hit with the latest bytes.
    let hit = lib
        .get_analysis(id, PYIN_ANALYZER_NAME, PYIN_ANALYZER_VERSION)
        .expect("get_analysis(v=0.1) should not error");
    assert_eq!(
        hit.as_deref(),
        Some(blob_v0_1.as_slice()),
        "0.1 round-trip must return the exact bytes that were upserted"
    );

    // 5. The legacy 0.0 row survives — versions are independent.
    let legacy = lib
        .get_analysis(id, PYIN_ANALYZER_NAME, "0.0")
        .expect("get_analysis(v=0.0) should not error after the 0.1 upsert");
    assert_eq!(
        legacy.as_deref(),
        Some(blob_v0_0.as_slice()),
        "v=0.0 row must survive a v=0.1 upsert; cache keys on (id, name, version)"
    );
}
