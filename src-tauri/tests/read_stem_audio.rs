#![allow(missing_docs)]
#![cfg(feature = "neural")]

//! `read_stem_audio_blocking` returns the bytes of the requested stem FLAC.
//!
//! `read_stem_audio_blocking` is the playback escape hatch the
//! `read_stem_audio` Tauri command wraps — the front-end wraps the bytes
//! into a synthetic `blob:` URL for the `PlaybackPanel`. This pins:
//!   * the `get_stem_result` lookup + `StemKind` → path selection for each
//!     of the four buses, and the `std::fs::read` of the on-disk FLAC.
//!   * `StemError::RecordingNotFound` when no `stem_results` row exists.
//!
//! The four FLACs are pre-written here (no ONNX), so this is NOT `#[ignore]`d.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use neural_pitch_core::pipeline::{FlacRecordingSink, RecordingSink};
use neural_pitch_core::store::{NewRecording, RecordingId, RecordingsLibrary};
use neural_pitch_lib::stems::{HTDEMUCS_SEPARATOR_VERSION, StemKind, read_stem_audio_blocking};

const SAMPLE_RATE_HZ: u32 = 48_000;

/// Write a short distinct-frequency mono FLAC so each stem's bytes differ.
fn write_tone_flac(path: &Path, hz: f32) {
    use std::f32::consts::TAU;
    let n = (SAMPLE_RATE_HZ / 4) as usize; // 0.25 s
    let mut sink = FlacRecordingSink::create(path, SAMPLE_RATE_HZ).expect("create flac sink");
    let step = TAU * hz / SAMPLE_RATE_HZ as f32;
    let buf: Vec<f32> = (0..n).map(|i| 0.5 * (step * i as f32).sin()).collect();
    sink.write(&buf).expect("write samples");
    Box::new(sink).finalize().expect("finalize flac");
}

fn build_fixture(test_name: &str) -> (RecordingsLibrary, RecordingId, PathBuf) {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test_name);
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");

    // A backing recording row so the FK on stem_results is satisfied.
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

#[test]
fn read_stem_audio_returns_each_buses_flac_bytes_verbatim() {
    let (lib, id, root) = build_fixture("read_stem_audio_happy");

    let stems_dir = root.join(id.to_string()).join("stems");
    std::fs::create_dir_all(&stems_dir).expect("create stems dir");

    // Four distinct-frequency FLACs so a wrong-bus path selection would
    // surface as mismatched bytes.
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

    for (kind, path) in [
        (StemKind::Vocals, &vocals),
        (StemKind::Drums, &drums),
        (StemKind::Bass, &bass),
        (StemKind::Other, &other),
    ] {
        let bytes = read_stem_audio_blocking(&lib, &root, id, kind).unwrap_or_else(|e| {
            panic!(
                "read_stem_audio_blocking({}) must succeed: {e:#}",
                kind.slug()
            )
        });
        let on_disk = std::fs::read(path).expect("read on-disk flac");
        assert_eq!(
            bytes,
            on_disk,
            "{} bytes must equal the on-disk FLAC verbatim (path-selection must pick the right bus)",
            kind.slug(),
        );
        assert!(
            !bytes.is_empty(),
            "{} FLAC must carry audio bytes",
            kind.slug(),
        );
    }
}

#[test]
fn read_stem_audio_without_a_stem_results_row_is_recording_not_found() {
    let (lib, id, root) = build_fixture("read_stem_audio_missing");

    // No upsert_stem_result — the lookup must miss.
    let err = read_stem_audio_blocking(&lib, &root, id, StemKind::Vocals)
        .expect_err("a recording with no stem_results row must not resolve a stem");

    assert!(
        matches!(
            err,
            neural_pitch_lib::stems::StemError::RecordingNotFound(rid) if rid == id
        ),
        "a missing stem_results row must surface StemError::RecordingNotFound; got {err:?}",
    );
}
