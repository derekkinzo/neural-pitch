#![allow(missing_docs)]
#![cfg(feature = "neural")]

//! `export_stem_blocking` copies a cached stem FLAC to a destination via
//! an atomic partial-write + rename.
//!
//! `export_stem_blocking` is the headless twin the `export_stem` Tauri
//! command wraps (the copy logic used to live inline inside the command's
//! `spawn_blocking` closure). This pins:
//!   * the `get_stem_result` lookup + `StemKind` → path selection, and the
//!     byte-count return matching the source size for each bus.
//!   * the atomic-replace-over-existing-file guarantee — the reason for the
//!     `.partial` dance — by exporting over a destination pre-seeded with
//!     different bytes and asserting it is replaced wholesale.
//!   * `StemError::RecordingNotFound` when no `stem_results` row exists
//!     (the command renders "run separate_stems first").
//!
//! The stem FLACs are pre-written here (no ONNX), so this is NOT `#[ignore]`d.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingSink};
use neural_pitch_core::store::{NewRecording, RecordingId, RecordingsLibrary};
use neural_pitch_lib::stems::{
    HTDEMUCS_SEPARATOR_VERSION, StemError, StemKind, export_stem_blocking,
};

const SAMPLE_RATE_HZ: u32 = 48_000;

fn write_tone_flac(path: &Path, hz: f32) {
    use std::f32::consts::TAU;
    let n = (SAMPLE_RATE_HZ / 4) as usize; // 0.25 s
    let mut sink = FlacRecordingSink::create(path, SAMPLE_RATE_HZ).expect("create flac sink");
    let step = TAU * hz / SAMPLE_RATE_HZ as f32;
    let buf: Vec<f32> = (0..n).map(|i| 0.5 * (step * i as f32).sin()).collect();
    sink.write(&buf).expect("write samples");
    Box::new(sink).finalize().expect("finalize flac");
}

/// Seed a backing recording + four stem FLACs + the `stem_results` row;
/// return the library, id, and the root dir.
fn build_fixture(test_name: &str) -> (RecordingsLibrary, RecordingId, PathBuf) {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test_name);
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");

    let id = lib
        .insert_recording(NewRecording {
            filename: "source.flac".to_string(),
            created_at_unix_ms: 1_717_502_580_000,
            duration_ms: 250,
            sample_rate_hz: i64::from(SAMPLE_RATE_HZ),
            channels: 1,
            bit_depth: 24,
            format: "flac".to_string(),
            a4_hz: 440.0,
            instrument_profile: "voice".to_string(),
            user_label: None,
        })
        .expect("insert recording");

    (lib, id, tmp_root)
}

fn seed_stems(lib: &RecordingsLibrary, id: RecordingId, root: &Path) -> [PathBuf; 4] {
    let stems_dir = root.join(id.to_string()).join("stems");
    std::fs::create_dir_all(&stems_dir).expect("create stems dir");
    let vocals = stems_dir.join("vocals.flac");
    let drums = stems_dir.join("drums.flac");
    let bass = stems_dir.join("bass.flac");
    let other = stems_dir.join("other.flac");
    write_tone_flac(&vocals, 440.0);
    write_tone_flac(&drums, 110.0);
    write_tone_flac(&bass, 82.0);
    write_tone_flac(&other, 660.0);

    lib.upsert_stem_result(
        id,
        HTDEMUCS_SEPARATOR_VERSION,
        1_717_502_580_000,
        vocals.to_str().unwrap(),
        drums.to_str().unwrap(),
        bass.to_str().unwrap(),
        other.to_str().unwrap(),
    )
    .expect("upsert stem_results row");

    [vocals, drums, bass, other]
}

#[test]
fn export_stem_copies_each_bus_byte_for_byte() {
    let (lib, id, root) = build_fixture("export_stem_each_bus");
    let [vocals, drums, bass, other] = seed_stems(&lib, id, &root);

    for (kind, src) in [
        (StemKind::Vocals, &vocals),
        (StemKind::Drums, &drums),
        (StemKind::Bass, &bass),
        (StemKind::Other, &other),
    ] {
        let dest = root.join(format!("export-{}.flac", kind.slug()));
        let bytes = export_stem_blocking(&lib, id, kind, &dest).unwrap_or_else(|e| {
            panic!("export_stem_blocking({}) must succeed: {e:#}", kind.slug())
        });

        let src_bytes = std::fs::read(src).expect("read source flac");
        assert_eq!(
            bytes as usize,
            src_bytes.len(),
            "{} export must report a byte count equal to the source FLAC size",
            kind.slug(),
        );
        let dest_bytes = std::fs::read(&dest).expect("read exported flac");
        assert_eq!(
            dest_bytes,
            src_bytes,
            "{} export must produce a byte-identical copy at the destination",
            kind.slug(),
        );
        // The `.partial` scratch file must be gone after the atomic rename.
        let partial = {
            let mut p = dest.clone().into_os_string();
            p.push(".partial");
            PathBuf::from(p)
        };
        assert!(
            !partial.exists(),
            "the .partial scratch file must be renamed away, not left behind",
        );
    }
}

#[test]
fn export_stem_atomically_replaces_a_preexisting_destination() {
    let (lib, id, root) = build_fixture("export_stem_atomic_replace");
    let [vocals, ..] = seed_stems(&lib, id, &root);

    // Pre-seed the destination with unrelated bytes whose length differs
    // from the source so a partial / append bug would surface.
    let dest = root.join("preexisting.flac");
    std::fs::write(&dest, b"these are stale bytes that must be fully replaced")
        .expect("seed preexisting destination");

    let bytes = export_stem_blocking(&lib, id, StemKind::Vocals, &dest)
        .expect("export over a pre-existing file must succeed");

    let src_bytes = std::fs::read(&vocals).expect("read source flac");
    let dest_bytes = std::fs::read(&dest).expect("read replaced destination");
    assert_eq!(
        bytes as usize,
        src_bytes.len(),
        "byte count must equal the source size, not the stale destination size",
    );
    assert_eq!(
        dest_bytes, src_bytes,
        "the atomic rename must replace the destination wholesale, not append/merge",
    );
}

#[test]
fn export_stem_without_a_stem_results_row_is_recording_not_found() {
    let (lib, id, root) = build_fixture("export_stem_missing");

    // No seed_stems — the lookup must miss.
    let dest = root.join("never-written.flac");
    let err = export_stem_blocking(&lib, id, StemKind::Vocals, &dest)
        .expect_err("a recording with no stem_results row must not export");

    assert!(
        matches!(err, StemError::RecordingNotFound(rid) if rid == id),
        "a missing stem_results row must surface StemError::RecordingNotFound \
         (the command renders \"run separate_stems first\"); got {err:?}",
    );
    assert!(
        !dest.exists(),
        "no destination file may be created when the lookup misses",
    );
}
