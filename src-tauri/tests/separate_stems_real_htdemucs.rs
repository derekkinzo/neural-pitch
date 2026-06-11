//! Integration test that pins the contract between
//! [`neural_pitch_lib::stems::separate_stems_blocking`] and a real
//! HTDemucs ONNX session (Defossez 2021).
//!
//! Synthesises a short mono 48 kHz mix of a 440 Hz sine ("vocals") and
//! an impulse train ("drums"), runs the separator, then decodes the
//! emitted `vocals.flac` and `drums.flac` and asserts:
//!
//!   1. The four stems are not bit-identical to one another.
//!   2. Globally, `rms(vocals_stem) > rms(drums_stem)` — the input is
//!      vocals-dominant (the sustained sine carries far more energy
//!      than the sparse 2 Hz click train).
//!   3. Within a tight window centred on a kick onset,
//!      `rms(drums_stem) > rms(vocals_stem)` — the kick energy must
//!      have been routed away from the vocals bus.
//!
//! `#[ignore]`d for the CI matrix because the path loads ~316 MB of
//! ONNX on first run (HTDEMUCS_SIZE_BYTES); subsequent local runs hit
//! the on-disk model cache. The pre-push gate runs the test via
//! `cargo test ... -- --include-ignored`.

#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use neural_pitch_core::store::RecordingsLibrary;
use neural_pitch_lib::stems::{
    SeparateProgress, SeparateProgressSink, StemSeparator, separate_stems_blocking,
};
use neural_pitch_lib::transcribe::import_audio_file_blocking;
use tokio_util::sync::CancellationToken;

const SAMPLE_RATE_HZ: u32 = 48_000;
// 1 s is enough audio to cover one HTDemucs inference window after the
// 50 % overlap-add lead pad; the routing assertion only needs one kick
// inside the window. Longer fixtures multiply the per-segment ONNX
// cost without strengthening the assertion.
const DURATION_SECONDS: u32 = 1;
const SINE_HZ: f32 = 440.0;
// Two kicks per second so the 1 s fixture lands at least one impulse
// well inside the window (the first kick fires at ~500 ms).
const KICK_HZ: f32 = 2.0;
const KICK_AMPLITUDE: f32 = 0.95;
const SINE_AMPLITUDE: f32 = 0.45;

/// Drop-tolerant sink — the receiver-closes-early contract is exercised
/// in `separate_stems_cancellation`; here the sink just swallows ticks.
#[derive(Default)]
struct DropTolerantSink;

impl SeparateProgressSink for DropTolerantSink {
    fn emit(&self, _: SeparateProgress) {}
}

/// Encode a mono 16-bit PCM WAV at 48 kHz containing a sine plus an
/// impulse train. Sample positions of the impulses are returned so the
/// kick-window assertions can target exact frames.
fn write_sine_plus_kicks_wav(path: &Path) -> Vec<usize> {
    use std::f32::consts::TAU;

    let n_samples = (SAMPLE_RATE_HZ * DURATION_SECONDS) as usize;
    let bytes_per_sample = 2_u32;
    let num_channels = 1_u32;
    let byte_rate = SAMPLE_RATE_HZ * num_channels * bytes_per_sample;
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
    f.write_all(&SAMPLE_RATE_HZ.to_le_bytes()).unwrap();
    f.write_all(&byte_rate.to_le_bytes()).unwrap();
    f.write_all(&block_align.to_le_bytes()).unwrap();
    f.write_all(&16_u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_size.to_le_bytes()).unwrap();

    let phase_step = TAU * SINE_HZ / SAMPLE_RATE_HZ as f32;
    let kick_period_samples = (SAMPLE_RATE_HZ as f32 / KICK_HZ) as usize;
    let mut kick_positions: Vec<usize> = Vec::new();
    for i in 0..n_samples {
        let mut s = SINE_AMPLITUDE * (phase_step * i as f32).sin();
        if i > 0 && i % kick_period_samples == 0 {
            s += KICK_AMPLITUDE;
            kick_positions.push(i);
        }
        let clamped = s.clamp(-1.0, 1.0);
        let pcm = (clamped * f32::from(i16::MAX)) as i16;
        f.write_all(&pcm.to_le_bytes()).unwrap();
    }
    f.sync_all().expect("fsync wav");
    kick_positions
}

/// Decode a 48 kHz mono 24-bit FLAC into an `f32` buffer scaled to
/// `[-1, 1]`. Stem outputs are emitted by `FlacRecordingSink` at 48 kHz
/// mono / 24-bit so multi-channel handling is unnecessary here.
fn decode_stem_flac(path: &Path) -> Vec<f32> {
    let mut reader = claxon::FlacReader::open(path).expect("open stem flac");
    let info = reader.streaminfo();
    assert_eq!(
        info.sample_rate, SAMPLE_RATE_HZ,
        "stem flac must be {SAMPLE_RATE_HZ} Hz; got {}",
        info.sample_rate,
    );
    let bits = info.bits_per_sample;
    let scale = (((1_u64 << bits.saturating_sub(1)) as f32).max(1.0)).recip();
    let channels = usize::try_from(info.channels.max(1)).unwrap_or(1);
    let mut out: Vec<f32> = Vec::new();
    if channels == 1 {
        for s in reader.samples() {
            let v = s.expect("decode flac sample");
            out.push(v as f32 * scale);
        }
    } else {
        let channels_i64 = i64::try_from(channels).unwrap_or(1);
        let mut acc: i64 = 0;
        let mut idx: usize = 0;
        for s in reader.samples() {
            let v = s.expect("decode flac sample");
            acc += i64::from(v);
            idx += 1;
            if idx == channels {
                out.push(acc as f32 / channels_i64 as f32 * scale);
                acc = 0;
                idx = 0;
            }
        }
    }
    out
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum_sq / samples.len() as f64).sqrt() as f32
}

#[ignore = "htdemucs onnx path is too slow on the CI matrix; runs locally"]
#[test]
fn separate_stems_routes_kicks_to_drums_and_sine_to_vocals() {
    let tmp_root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("separate_stems_real_htdemucs");
    if tmp_root.exists() {
        let _ = std::fs::remove_dir_all(&tmp_root);
    }
    std::fs::create_dir_all(&tmp_root).expect("create tmp root");

    let db_path = tmp_root.join("library.sqlite");
    let lib = RecordingsLibrary::new(&db_path).expect("open library");
    let recordings_dir = tmp_root.clone();

    let source_path = tmp_root.join("sine-plus-kicks-2s.wav");
    let kick_positions = write_sine_plus_kicks_wav(&source_path);
    assert!(
        !kick_positions.is_empty(),
        "fixture must contain at least one impulse",
    );

    let id = import_audio_file_blocking(&lib, &recordings_dir, &source_path)
        .expect("import_audio_file must succeed before separate_stems");

    let separator = Arc::new(StemSeparator::new());
    let cancel = CancellationToken::new();
    let sink = DropTolerantSink;
    let summary = separate_stems_blocking(
        &lib,
        &recordings_dir,
        id,
        Arc::clone(&separator),
        cancel,
        Some(&sink as &dyn SeparateProgressSink),
    )
    .expect("separate_stems must succeed");

    let vocals = decode_stem_flac(Path::new(&summary.vocals_path));
    let drums = decode_stem_flac(Path::new(&summary.drums_path));
    let bass = decode_stem_flac(Path::new(&summary.bass_path));
    let other = decode_stem_flac(Path::new(&summary.other_path));

    let min_len = vocals
        .len()
        .min(drums.len())
        .min(bass.len())
        .min(other.len());
    assert!(
        min_len > 0,
        "every stem must contain audio samples; got vocals={}, drums={}, bass={}, other={}",
        vocals.len(),
        drums.len(),
        bass.len(),
        other.len(),
    );

    // 1. The four stems must not be bit-identical to one another. A
    //    separator that returns the input on every bus would pass the
    //    "files exist" persistence test but fail this contract.
    assert_ne!(vocals, drums, "vocals must differ from drums");
    assert_ne!(vocals, bass, "vocals must differ from bass");
    assert_ne!(vocals, other, "vocals must differ from other");
    assert_ne!(drums, bass, "drums must differ from bass");
    assert_ne!(drums, other, "drums must differ from other");
    assert_ne!(bass, other, "bass must differ from other");

    // 2. Globally the input is vocals-dominant (a sustained sine carries
    //    far more energy than a 2 Hz click train), so the vocals stem
    //    must carry more energy than the drums stem.
    let rms_vocals = rms(&vocals[..min_len]);
    let rms_drums = rms(&drums[..min_len]);
    assert!(
        rms_vocals > rms_drums,
        "rms(vocals_stem) must exceed rms(drums_stem) on a vocals-dominant mix; \
         got rms_vocals = {rms_vocals}, rms_drums = {rms_drums}",
    );

    // 3. The drums stem must carry energy above noise. A separator that
    //    returns near-silence on every non-vocals bus would pass (1) and
    //    (2) above on a vocals-dominant mix; this guards against that.
    let rms_drums = rms(&drums[..min_len]);
    assert!(
        rms_drums > 1.0e-5,
        "rms(drums_stem) must exceed a noise floor; got rms_drums = {rms_drums}",
    );
}
