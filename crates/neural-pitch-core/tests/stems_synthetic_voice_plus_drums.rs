#![allow(missing_docs)]
#![cfg(feature = "neural")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::uninlined_format_args
)]

//! Phase 5 RED — synthetic voice + synthetic drums mix.
//!
//! Build a clean voice signal at MIDI 69 (A4) using
//! [`crate::test_utils::voice::synth_voice`] and sum it with a
//! synthetic 1 Hz kick-click track (impulse train at 1 Hz, decaying
//! exponential per click). The vocals stem must capture ≥ 50 % of
//! the voice-only RMS energy; the drums stem at the voice formant
//! band must stay below 10 % of the voice-only energy (i.e. the
//! drum stem must not bleed the voice's spectral content).

use neural_pitch_core::stems::StemSeparator;
use neural_pitch_core::test_utils::voice::synth_voice;
use tokio_util::sync::CancellationToken;

const SR_HZ: u32 = 44_100;
const DURATION_MS: u64 = 4_000;
const F0_HZ: f32 = 440.0; // A4 / MIDI 69

fn rms(buf: &[f32]) -> f32 {
    if buf.is_empty() {
        return 0.0;
    }
    let s: f32 = buf.iter().map(|v| v * v).sum();
    (s / buf.len() as f32).sqrt()
}

/// Build a synthetic kick-click track: 1 Hz impulse train shaped by
/// an exponential decay. Returns mono.
fn synth_kicks(sample_rate: u32, n_samples: usize) -> Vec<f32> {
    let sr = sample_rate as f32;
    let mut out = vec![0.0_f32; n_samples];
    let kick_period_samples = sr as usize; // 1 Hz
    let decay = 0.999_f32;
    let mut i = 0;
    while i < n_samples {
        let mut amp: f32 = 0.95;
        let mut j = i;
        while j < n_samples && amp > 1e-4 {
            out[j] += amp;
            amp *= decay;
            j += 1;
        }
        i += kick_period_samples;
    }
    out
}

#[ignore = "ort cpu-fallback path is too slow on the CI matrix; HTDEMUCS_MODEL_URL/SHA256 are also placeholders until the upstream commit is pinned, so this test only exercises a sideloaded model on the local gate"]
#[test]
fn stems_voice_plus_drums_isolates_voice_into_vocals() {
    let n_samples = (SR_HZ as u64 * DURATION_MS / 1_000) as usize;

    let voice_only = synth_voice(F0_HZ, SR_HZ, n_samples, None, true);
    let kicks = synth_kicks(SR_HZ, n_samples);

    // Sum voice + drums into a stereo (duplicated) mix.
    let mut stereo = Vec::with_capacity(n_samples * 2);
    for i in 0..n_samples {
        let s = 0.5 * voice_only[i] + 0.5 * kicks[i];
        stereo.push(s);
        stereo.push(s);
    }

    let model_path = StemSeparator::ensure_model(|_| {})
        .expect("HTDemucs ONNX must be cached or downloadable on the local gate");
    let mut sep = StemSeparator::open(&model_path).expect("open HTDemucs session");

    let result = sep
        .separate(&stereo, SR_HZ, 2, |_| {}, &CancellationToken::new())
        .expect("separate must not error on a voice + drums stereo mix");

    let voice_only_rms = rms(&voice_only);

    // Vocals stem must hold at least half of the voice's RMS energy.
    let vocals_rms = rms(&result.vocals);
    assert!(
        vocals_rms > 0.5 * voice_only_rms,
        "vocals stem must capture the voice content: vocals_rms={vocals_rms} \
         voice_only_rms={voice_only_rms}",
    );

    // Drums stem at the (broadband) voice level must not bleed more
    // than 10 % of the voice-only energy: a clean separation puts the
    // tonal content in `vocals`, not `drums`.
    let drums_rms = rms(&result.drums);
    assert!(
        drums_rms < 0.1 * voice_only_rms,
        "drums stem must not bleed voice content: drums_rms={drums_rms} \
         voice_only_rms={voice_only_rms}",
    );
}
