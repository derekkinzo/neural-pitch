//! Tauri / persistence integration test for the file-import surface.
//!
//! Drives [`neural_pitch_lib::transcribe::import_audio_file_blocking`]
//! (the headless twin the Tauri command wraps) directly so the test
//! harness does not need to spin up a full Tauri runtime — same shape as
//! the existing `analyze_recording.rs` test.

#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::Write;
use std::path::PathBuf;

use neural_pitch_core::store::{ListFilter, RecordingsLibrary};
use neural_pitch_lib::transcribe::import_audio_file_blocking;

/// Synthesise a 1 s 440 Hz sine and write a minimal 16-bit PCM `.wav`
/// (RIFF/WAVE/fmt /data) to `path`. Inline rather than a shared
/// `test_utils::write_wav` helper because writing the WAV byte stream
/// out of one library function would couple the integration tests to
/// an internal helper that has no production caller — imports only
/// need the file to exist and Symphonia-probe cleanly.
fn write_440hz_sine_wav(path: &std::path::Path, sample_rate_hz: u32, duration_ms: u32) {
    use std::f32::consts::TAU;

    let n_samples =
        usize::try_from((u64::from(sample_rate_hz) * u64::from(duration_ms)) / 1_000).unwrap();
    let bytes_per_sample = 2_u32; // 16-bit PCM
    let num_channels = 1_u32;
    let byte_rate = sample_rate_hz * num_channels * bytes_per_sample;
    let block_align = u16::try_from(num_channels * bytes_per_sample).unwrap();
    let data_size = u32::try_from(n_samples).unwrap() * bytes_per_sample;
    let chunk_size = 36 + data_size;

    let mut f = std::fs::File::create(path).expect("create wav");
    // RIFF header.
    f.write_all(b"RIFF").unwrap();
    f.write_all(&chunk_size.to_le_bytes()).unwrap();
    f.write_all(b"WAVE").unwrap();
    // fmt subchunk (PCM, 16-bit).
    f.write_all(b"fmt ").unwrap();
    f.write_all(&16_u32.to_le_bytes()).unwrap();
    f.write_all(&1_u16.to_le_bytes()).unwrap(); // PCM
    f.write_all(&u16::try_from(num_channels).unwrap().to_le_bytes())
        .unwrap();
    f.write_all(&sample_rate_hz.to_le_bytes()).unwrap();
    f.write_all(&byte_rate.to_le_bytes()).unwrap();
    f.write_all(&block_align.to_le_bytes()).unwrap();
    f.write_all(&16_u16.to_le_bytes()).unwrap(); // bits per sample
    // data subchunk.
    f.write_all(b"data").unwrap();
    f.write_all(&data_size.to_le_bytes()).unwrap();

    let phase_step = TAU * 440.0 / sample_rate_hz as f32;
    for i in 0..n_samples {
        let s = (phase_step * i as f32).sin();
        let pcm = (s * 0.6 * f32::from(i16::MAX)) as i16;
        f.write_all(&pcm.to_le_bytes()).unwrap();
    }
    f.sync_all().expect("fsync wav");
}

#[test]
fn import_audio_file_inserts_imported_row_and_returns_id() {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("phase3_import_audio_file");
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    // Library + recordings_dir share the same parent so the imports/
    // sub-directory the import path creates lives under the same
    // app-data root the production shell uses.
    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");
    let recordings_dir = tmp_root.clone();

    // 1 s @ 48 kHz mono 16-bit PCM.
    let source_path = tmp_root.join("source-440hz-1s.wav");
    write_440hz_sine_wav(&source_path, 48_000, 1_000);

    let id = import_audio_file_blocking(&lib, &recordings_dir, &source_path)
        .expect("import_audio_file must succeed against a valid 1s 16-bit PCM WAV");

    let listed = lib
        .list_recordings(ListFilter::ActiveOnly)
        .expect("list_recordings must succeed");
    assert_eq!(
        listed.len(),
        1,
        "exactly one row must be present after a single import; got {} rows",
        listed.len(),
    );
    let only = &listed[0];
    assert_eq!(
        only.id, id,
        "list_recordings must surface the same id import_audio_file returned",
    );
    assert_eq!(
        only.instrument_profile, "Imported",
        "imported rows MUST stamp instrument_profile = \"Imported\" so the UI can \
         distinguish them from live-captured takes",
    );
    assert_eq!(
        only.format, "wav",
        "format column must mirror the source extension"
    );
    assert_eq!(
        only.sample_rate_hz, 48_000,
        "sample_rate_hz must round-trip from the Symphonia probe",
    );
    assert_eq!(
        only.channels, 1,
        "channels must round-trip from the Symphonia probe",
    );
    assert!(
        only.duration_ms >= 990 && only.duration_ms <= 1_010,
        "duration_ms must be within ±10 ms of the synthesised 1 s clip; got {}",
        only.duration_ms,
    );
    assert!(
        (only.a4_hz - 440.0).abs() < f64::EPSILON,
        "imported rows default to a4_hz = 440.0 (no instrument-specific tuning context); \
         got {}",
        only.a4_hz,
    );
}

#[test]
fn import_audio_file_rejects_unsupported_extension() {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("phase3_import_unsupported_ext");
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");
    let recordings_dir = tmp_root.clone();

    // `.ogg` is intentionally outside Symphonia's WAV/FLAC/MP3 gate.
    let source_path = tmp_root.join("source.ogg");
    std::fs::write(&source_path, b"not really an ogg").expect("write fake ogg");

    let err = import_audio_file_blocking(&lib, &recordings_dir, &source_path)
        .expect_err("`.ogg` extension MUST be refused before any decode work happens");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("unsupported extension"),
        "rejection error MUST mention the unsupported-extension contract; got: {msg}",
    );

    // No row should have been inserted on the rejection path.
    let listed = lib
        .list_recordings(ListFilter::IncludingDeleted)
        .expect("list_recordings");
    assert!(
        listed.is_empty(),
        "import rejection MUST NOT mutate the recordings table; got {} rows",
        listed.len(),
    );
}
