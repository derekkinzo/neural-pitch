//! Phase 3 — Tauri / persistence integration test for the MIDI export
//! surface.
//!
//! Drives [`neural_pitch_lib::transcribe::export_midi_blocking`] (the
//! headless twin the Tauri command wraps) directly so the test harness
//! does not need to spin up a full Tauri runtime.
//!
//! Round-trip strategy:
//! 1. Synthesise a 1 s 440 Hz sine into a temp WAV.
//! 2. Import → transcribe (the latter populates `analysis_cache`).
//! 3. Export to a temp `.mid` path.
//! 4. Re-parse with [`midly::Smf::parse`] and assert at least one
//!    `MidiMessage::NoteOn` event survives. The acceptance criterion is
//!    "the SMF byte stream is structurally valid AND carries actual
//!    note content"; the exact pitch / channel / velocity arithmetic
//!    lives in `crates/neural-pitch-core/tests/poly_*.rs`.
//!
//! TDD-RED status — `export_midi_blocking` returns
//! `Err(TranscribeError::NotImplemented)` until Phase 3 GREEN ships.

#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::Write;
use std::path::PathBuf;

use midly::{MidiMessage, Smf, TrackEventKind};
use neural_pitch_core::store::RecordingsLibrary;
use neural_pitch_lib::transcribe::{
    export_midi_blocking, import_audio_file_blocking, transcribe_recording_blocking,
};

/// 1 s 440 Hz mono 16-bit PCM WAV — same shape as the import test fixture.
fn write_440hz_sine_wav(path: &std::path::Path, sample_rate_hz: u32, duration_ms: u32) {
    use std::f32::consts::TAU;

    let n_samples =
        usize::try_from((u64::from(sample_rate_hz) * u64::from(duration_ms)) / 1_000).unwrap();
    let bytes_per_sample = 2_u32;
    let num_channels = 1_u32;
    let byte_rate = sample_rate_hz * num_channels * bytes_per_sample;
    let block_align = u16::try_from(num_channels * bytes_per_sample).unwrap();
    let data_size = u32::try_from(n_samples).unwrap() * bytes_per_sample;
    let chunk_size = 36 + data_size;

    let mut f = std::fs::File::create(path).expect("create wav");
    f.write_all(b"RIFF").unwrap();
    f.write_all(&chunk_size.to_le_bytes()).unwrap();
    f.write_all(b"WAVE").unwrap();
    f.write_all(b"fmt ").unwrap();
    f.write_all(&16_u32.to_le_bytes()).unwrap();
    f.write_all(&1_u16.to_le_bytes()).unwrap();
    f.write_all(&u16::try_from(num_channels).unwrap().to_le_bytes())
        .unwrap();
    f.write_all(&sample_rate_hz.to_le_bytes()).unwrap();
    f.write_all(&byte_rate.to_le_bytes()).unwrap();
    f.write_all(&block_align.to_le_bytes()).unwrap();
    f.write_all(&16_u16.to_le_bytes()).unwrap();
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
fn export_midi_writes_valid_smf_with_at_least_one_note_on() {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("phase3_export_midi_roundtrip");
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");
    let recordings_dir = tmp_root.clone();

    let source_path = tmp_root.join("source-440hz-1s.wav");
    write_440hz_sine_wav(&source_path, 48_000, 1_000);

    let id = import_audio_file_blocking(&lib, &recordings_dir, &source_path)
        .expect("import_audio_file must succeed before transcribe");

    let _ = transcribe_recording_blocking(&lib, &recordings_dir, id, false, None)
        .expect("transcribe must succeed before export");

    let dest_path = tmp_root.join("export.mid");
    let bytes_written = export_midi_blocking(&lib, id, &dest_path)
        .expect("export_midi must succeed against a transcribed recording");
    assert!(
        bytes_written > 0,
        "export_midi must write a non-empty SMF; got {bytes_written} bytes",
    );

    // Atomic-write contract: the partial path must NOT linger on success.
    let partial_path = dest_path.with_extension("mid.partial");
    assert!(
        !partial_path.exists(),
        "atomic write must rename .partial to the final path on success; \
         partial still exists at {}",
        partial_path.display(),
    );

    let bytes = std::fs::read(&dest_path).expect("read written SMF");
    assert_eq!(
        u64::try_from(bytes.len()).unwrap(),
        bytes_written,
        "export_midi return value must match the on-disk file size",
    );

    // Round-trip the SMF through midly. `Smf::parse` returns Err on any
    // structural malformation (header / track-chunk size mismatch, etc.).
    let smf = Smf::parse(&bytes).expect("written SMF must round-trip through midly::Smf::parse");
    assert!(
        !smf.tracks.is_empty(),
        "SMF must have at least one track; got {} tracks",
        smf.tracks.len(),
    );

    // Walk every track and assert at least one NoteOn fires. The exact
    // (pitch, channel, velocity) tuple is checked in the core poly
    // tests; here we only need to know the export pipeline is emitting
    // note content rather than just the meta-event preamble.
    let mut note_on_count = 0_usize;
    for track in &smf.tracks {
        for event in track {
            if let TrackEventKind::Midi {
                message: MidiMessage::NoteOn { vel, .. },
                ..
            } = event.kind
            {
                if vel.as_int() > 0 {
                    note_on_count += 1;
                }
            }
        }
    }
    assert!(
        note_on_count >= 1,
        "exported SMF must carry at least one MIDI NoteOn (vel > 0); got {note_on_count}",
    );
}
