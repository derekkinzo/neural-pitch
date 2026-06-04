//! Phase 1.4 Tier-2 voice-fixture builder.
//!
//! Generates 13 deterministic synthetic vocal fixtures across the SATB
//! tessitura range (E2..A5), encodes them as 48 kHz / 24-bit / mono FLAC,
//! and writes them — along with a canonical `MANIFEST.toml` — into
//! `crates/neural-pitch-core/tests/fixtures/voice/`.
//!
//! Run once locally and commit the output:
//!
//! ```text
//! cargo run --example build_voice_fixtures --release
//! ```
//!
//! This is a build-time CLI tool, not production code, so the workspace
//! `unwrap_used`/`expect_used`/`panic` denials are explicitly relaxed at
//! the crate-level attribute below. The synthesis itself lives in
//! `neural_pitch_core::test_utils::voice::synth_voice` and is bound by the
//! strict workspace lint policy.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(clippy::print_stdout)]

use std::fs;
use std::path::{Path, PathBuf};

use flacenc::component::BitRepr;
use flacenc::error::Verify;
use neural_pitch_core::test_utils::voice::synth_voice;

const SAMPLE_RATE: u32 = 48_000;
const BITS_PER_SAMPLE: usize = 24;
const DURATION_SECONDS: f32 = 1.5;
const PEAK_SCALE_FACTOR: f32 = (1_i32 << 23) as f32; // 24-bit signed peak (8388608)

/// Reference frequency for MIDI 69 (A4). Hard-pinned to 440 Hz for all
/// fixture metadata so the manifest matches the harness expectation
/// regardless of any user A4 setting.
const A4_HZ: f32 = 440.0;

/// One fixture spec — what the builder needs to know to synthesise and
/// describe each WAV→FLAC slot.
struct FixtureSpec {
    midi: u8,
    note_name: &'static str,
    vibrato: Option<(f32, f32)>, // (rate_hz, depth_cents)
    /// Fragment placed in the filename and `vibrato` manifest field.
    vibrato_tag: &'static str,
}

const FIXTURES: &[FixtureSpec] = &[
    // Bass
    FixtureSpec {
        midi: 40,
        note_name: "E2",
        vibrato: None,
        vibrato_tag: "clean",
    },
    FixtureSpec {
        midi: 45,
        note_name: "A2",
        vibrato: None,
        vibrato_tag: "clean",
    },
    FixtureSpec {
        midi: 48,
        note_name: "C3",
        vibrato: None,
        vibrato_tag: "clean",
    },
    // Tenor
    FixtureSpec {
        midi: 55,
        note_name: "G3",
        vibrato: None,
        vibrato_tag: "clean",
    },
    FixtureSpec {
        midi: 60,
        note_name: "C4",
        vibrato: None,
        vibrato_tag: "clean",
    },
    FixtureSpec {
        midi: 64,
        note_name: "E4",
        vibrato: Some((5.0, 50.0)),
        vibrato_tag: "vibrato5hz_50c",
    },
    // Alto
    FixtureSpec {
        midi: 57,
        note_name: "A3",
        vibrato: None,
        vibrato_tag: "clean",
    },
    FixtureSpec {
        midi: 62,
        note_name: "D4",
        vibrato: None,
        vibrato_tag: "clean",
    },
    FixtureSpec {
        midi: 65,
        note_name: "F4",
        vibrato: None,
        vibrato_tag: "clean",
    },
    // Soprano
    FixtureSpec {
        midi: 69,
        note_name: "A4",
        vibrato: None,
        vibrato_tag: "clean",
    },
    FixtureSpec {
        midi: 72,
        note_name: "C5",
        vibrato: None,
        vibrato_tag: "clean",
    },
    FixtureSpec {
        midi: 77,
        note_name: "F5",
        vibrato: Some((5.0, 50.0)),
        vibrato_tag: "vibrato5hz_50c",
    },
    FixtureSpec {
        midi: 81,
        note_name: "A5",
        vibrato: Some((5.0, 50.0)),
        vibrato_tag: "vibrato5hz_50c",
    },
];

fn midi_to_hz(midi: u8) -> f32 {
    A4_HZ * ((midi as f32 - 69.0) / 12.0).exp2()
}

fn fixture_filename(spec: &FixtureSpec) -> String {
    format!(
        "{:03}_{}_synthvoice_{}.flac",
        spec.midi, spec.note_name, spec.vibrato_tag
    )
}

/// Convert the `synth_voice` `[-0.95, 0.95]` float buffer into 24-bit
/// signed PCM packed in `i32`. Peak headroom at 0.95 keeps us well clear
/// of the ±2^23 clipping point.
fn float_to_pcm24(samples: &[f32]) -> Vec<i32> {
    let mut out = Vec::with_capacity(samples.len());
    let max_val = (1_i32 << 23) - 1; //  8_388_607
    let min_val = -(1_i32 << 23); // -8_388_608
    for &s in samples {
        let scaled = (s * PEAK_SCALE_FACTOR).round() as i64;
        let clamped = scaled.clamp(min_val as i64, max_val as i64) as i32;
        out.push(clamped);
    }
    out
}

fn encode_flac(pcm24: &[i32], sample_rate: u32) -> Vec<u8> {
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .expect("default flacenc config must verify");
    let source = flacenc::source::MemSource::from_samples(
        pcm24,
        1, // channels
        BITS_PER_SAMPLE,
        sample_rate as usize,
    );
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .expect("flac encode must succeed for synthesised input");
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).expect("flac stream serialisation");
    sink.as_slice().to_vec()
}

fn manifest_entry(spec: &FixtureSpec) -> String {
    let filename = fixture_filename(spec);
    let expected_hz = midi_to_hz(spec.midi);
    let vibrato_block = match spec.vibrato {
        Some((rate, depth)) => {
            format!("vibrato        = {{ rate_hz = {rate:.1}, depth_cents = {depth:.1} }}")
        }
        None => "vibrato        = \"none\"".to_string(),
    };
    // The `formants` field tracks the corresponding `synth_voice` argument so
    // a future no-formants variant cannot silently mis-describe itself in
    // the manifest. Today every fixture is built with `formants = true`,
    // but the field is emitted unconditionally so the schema is honest.
    format!(
        "[[fixture]]\n\
         filename       = \"{filename}\"\n\
         expected_midi  = {midi}\n\
         expected_hz    = {expected_hz:.4}\n\
         instrument     = \"synthvoice\"\n\
         {vibrato_block}\n\
         snr            = \"clean\"\n\
         formants       = true\n\
         duration_s     = {duration:.1}\n",
        filename = filename,
        midi = spec.midi,
        expected_hz = expected_hz,
        vibrato_block = vibrato_block,
        duration = DURATION_SECONDS,
    )
}

fn build_manifest() -> String {
    let mut out = String::new();
    out.push_str("# Phase 1.4 Tier-2 voice fixture manifest.\n");
    out.push_str("# Generated by `cargo run --example build_voice_fixtures --release`.\n");
    out.push_str("# Do not edit by hand — re-run the example to regenerate.\n");
    out.push_str("#\n");
    out.push_str("# Schema (v1):\n");
    out.push_str("#   schema_version : u32 — bumped when fields below change.\n");
    out.push_str("#                          Currently informational only; the\n");
    out.push_str("#                          acceptance harness does not gate on\n");
    out.push_str("#                          it (see Phase-2 follow-up in\n");
    out.push_str("#                          PHASE-1-CLOSEOUT.md).\n");
    out.push_str("#\n");
    out.push_str("# Per [[fixture]] entry:\n");
    out.push_str("#   filename       : string  — path relative to this manifest.\n");
    out.push_str("#                              48 kHz / 24-bit / mono FLAC.\n");
    out.push_str("#   expected_midi  : i32     — MIDI note number of the\n");
    out.push_str("#                              fundamental, ground truth.\n");
    out.push_str("#   expected_hz    : f32 Hz  — `440 * 2^((midi-69)/12)`,\n");
    out.push_str("#                              re-emitted for human reading.\n");
    out.push_str("#   instrument     : string  — currently always \"synthvoice\".\n");
    out.push_str("#   vibrato        : \"none\" | { rate_hz: f32, depth_cents: f32 }\n");
    out.push_str("#   snr            : \"clean\" — placeholder; SNR variants are\n");
    out.push_str("#                              deferred to Phase 2.\n");
    out.push_str("#   formants       : bool    — true ⇒ `synth_voice` was\n");
    out.push_str("#                              called with formant cascade on.\n");
    out.push_str("#   duration_s     : f32 s   — wall-clock fixture length.\n");
    out.push_str("\nschema_version = 1\n\n");
    for spec in FIXTURES {
        out.push_str(&manifest_entry(spec));
        out.push('\n');
    }
    out
}

fn fixtures_dir() -> PathBuf {
    // Resolve relative to CARGO_MANIFEST_DIR so the example works regardless
    // of the caller's CWD (e.g. when invoked from the workspace root vs.
    // from inside the crate).
    let manifest = env!("CARGO_MANIFEST_DIR");
    Path::new(manifest)
        .join("tests")
        .join("fixtures")
        .join("voice")
}

fn main() {
    let dir = fixtures_dir();
    fs::create_dir_all(&dir).expect("create fixtures dir");
    println!("[fixtures] writing into {}", dir.display());

    let n_samples = (SAMPLE_RATE as f32 * DURATION_SECONDS).round() as usize;
    let mut total_bytes: u64 = 0;

    for spec in FIXTURES {
        let f0 = midi_to_hz(spec.midi);
        let buf = synth_voice(f0, SAMPLE_RATE, n_samples, spec.vibrato, true);
        let pcm = float_to_pcm24(&buf);
        let bytes = encode_flac(&pcm, SAMPLE_RATE);
        let filename = fixture_filename(spec);
        let path = dir.join(&filename);
        fs::write(&path, &bytes).expect("write flac");
        total_bytes += bytes.len() as u64;
        println!(
            "[fixtures] wrote {filename} (midi={}, f0={:.2} Hz, vibrato={}, {} bytes)",
            spec.midi,
            f0,
            spec.vibrato_tag,
            bytes.len()
        );
    }

    let manifest = build_manifest();
    let manifest_path = dir.join("MANIFEST.toml");
    fs::write(&manifest_path, manifest).expect("write manifest");
    println!(
        "[fixtures] wrote MANIFEST.toml ({} fixtures)",
        FIXTURES.len()
    );
    println!(
        "[fixtures] done: {} files, {:.1} KB total flac",
        FIXTURES.len(),
        total_bytes as f32 / 1024.0
    );
}
