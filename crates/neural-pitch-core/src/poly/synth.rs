//! Phase 4 — pure-Rust additive prompt synth.
//!
//! [`PromptSynth`] renders a short MIDI-pitch prompt as a complete
//! RIFF/WAVE byte stream (PCM16 mono, 48 kHz). The Tauri shell pipes the
//! returned bytes into a `Blob` URL the front-end's `<audio>` element
//! plays back so the drill UI never has to reach into Web Audio for
//! synthesis. Pure-Rust, no `ort` or external models — ships under
//! `--no-default-features`.
//!
//! GREEN implementation: a small additive sine stack
//! (fundamental + 2nd / 3rd partial at modest amplitudes) shaped by a
//! short cosine fade-in/out envelope to avoid the click that a hard
//! gate would produce. The harmonic content is deliberately quiet
//! enough that a YIN/MPM estimator locks the median onto the
//! fundamental within 1 cent, which is the contract
//! `synthesize_prompt_a4_440hz.rs` asserts.

use thiserror::Error;

use crate::music::midi_to_hz;

/// Sample rate the synth emits at, in Hertz. Matches the live-capture
/// sample rate so the prompt audio and any user-side capture share the
/// same clock domain.
pub const PROMPT_SAMPLE_RATE_HZ: u32 = 48_000;

/// Hard cap on prompt duration. Mirrors the Tauri command boundary
/// clamp: prompts longer than 10 s are not a use case the drill UI
/// supports today and rejecting them server-side keeps the synth's
/// memory budget bounded by a fixed constant.
pub const MAX_PROMPT_DURATION_MS: u32 = 10_000;

/// Reference A4 the synth tunes against. The drill UI sends the
/// caller's preferred A4 through [`crate::training::NoteSpec::a4_hz`];
/// the bare [`PromptSynth::render_wav`] shortcut uses 440.0 because
/// the drill protocol stores the a4 inside the prompt note and the
/// command boundary supplies it explicitly.
const PROMPT_A4_HZ: f32 = 440.0;

/// Additive partial amplitudes. Total peak amplitude budget held below
/// 0.85 to leave headroom for the cosine envelope without clipping i16
/// quantisation. Hoisted to module scope so `clippy::items_after_statements`
/// does not flag a `const` declared after the function's first `let`.
const FUND_AMP: f32 = 0.65;
const PART2_AMP: f32 = 0.12;
const PART3_AMP: f32 = 0.06;

/// Errors raised by [`PromptSynth::render_wav`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SynthError {
    /// Caller supplied a MIDI number outside `0..=127`.
    #[error("midi note out of range: {0}")]
    MidiOutOfRange(i32),
    /// Caller supplied a duration above [`MAX_PROMPT_DURATION_MS`].
    #[error("duration {got_ms} ms exceeds {MAX_PROMPT_DURATION_MS} ms cap")]
    DurationTooLong {
        /// Requested duration in milliseconds.
        got_ms: u32,
    },
}

/// Pure-Rust additive prompt synth.
#[derive(Debug)]
pub struct PromptSynth {
    sample_rate_hz: u32,
}

impl PromptSynth {
    /// Construct a new synth running at [`PROMPT_SAMPLE_RATE_HZ`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            sample_rate_hz: PROMPT_SAMPLE_RATE_HZ,
        }
    }

    /// Sample rate the synth emits at, in Hertz.
    #[must_use]
    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// Render a single MIDI prompt as a complete RIFF/WAVE byte stream.
    ///
    /// Returns a PCM16 mono WAV containing exactly
    /// `duration_ms * sample_rate_hz / 1_000` samples plus a 44-byte
    /// canonical RIFF header. Tuning is against
    /// [`PROMPT_A4_HZ`] (440.0 Hz); callers wanting an alternative
    /// reference should use [`Self::render_wav_at_a4`].
    pub fn render_wav(&mut self, midi: i32, duration_ms: u32) -> Result<Vec<u8>, SynthError> {
        self.render_wav_at_a4(midi, duration_ms, PROMPT_A4_HZ)
    }

    /// Render a prompt at an arbitrary A4 reference.
    ///
    /// Used by the drill IPC layer when [`crate::training::NoteSpec::a4_hz`]
    /// differs from 440 Hz (e.g. baroque pitch).
    pub fn render_wav_at_a4(
        &mut self,
        midi: i32,
        duration_ms: u32,
        a4_hz: f32,
    ) -> Result<Vec<u8>, SynthError> {
        if !(0..=127).contains(&midi) {
            return Err(SynthError::MidiOutOfRange(midi));
        }
        if duration_ms > MAX_PROMPT_DURATION_MS {
            return Err(SynthError::DurationTooLong {
                got_ms: duration_ms,
            });
        }

        let sample_rate = self.sample_rate_hz;
        let n_samples = (u64::from(sample_rate) * u64::from(duration_ms) / 1_000) as usize;
        let f0 = midi_to_hz(midi, if a4_hz > 0.0 { a4_hz } else { PROMPT_A4_HZ });

        // Additive stack: fundamental at full amplitude plus quieter
        // second and third partials. Tuning constants live at module
        // scope so the function body stays statement-only.

        // Cosine fade-in/out over the first/last 10 ms (or 10% of total
        // duration, whichever is shorter) so the prompt does not click.
        let fade_n = ((sample_rate as usize) / 100).min(n_samples / 5);

        let mut samples = Vec::<f32>::with_capacity(n_samples);
        let two_pi = core::f32::consts::TAU;
        let inv_sr = 1.0_f32 / sample_rate as f32;
        for i in 0..n_samples {
            let t = i as f32 * inv_sr;
            let phase1 = two_pi * f0 * t;
            let phase2 = two_pi * f0 * 2.0 * t;
            let phase3 = two_pi * f0 * 3.0 * t;
            let mut s =
                FUND_AMP * phase1.sin() + PART2_AMP * phase2.sin() + PART3_AMP * phase3.sin();

            // Half-cosine envelope on both ends.
            if fade_n > 0 {
                if i < fade_n {
                    let p = i as f32 / fade_n as f32;
                    let env = 0.5 - 0.5 * (core::f32::consts::PI * p).cos();
                    s *= env;
                } else if i + fade_n >= n_samples {
                    let tail = n_samples - i - 1;
                    let p = tail as f32 / fade_n as f32;
                    let env = 0.5 - 0.5 * (core::f32::consts::PI * p).cos();
                    s *= env;
                }
            }
            samples.push(s);
        }

        Ok(encode_wav_pcm16_mono(&samples, sample_rate))
    }
}

impl Default for PromptSynth {
    fn default() -> Self {
        Self::new()
    }
}

/// Encode a slice of `f32` samples in `[-1.0, 1.0]` as a canonical
/// RIFF/WAVE PCM16 mono byte stream.
fn encode_wav_pcm16_mono(samples: &[f32], sample_rate_hz: u32) -> Vec<u8> {
    let bits_per_sample: u16 = 16;
    let num_channels: u16 = 1;
    let byte_rate: u32 = sample_rate_hz * u32::from(num_channels) * u32::from(bits_per_sample) / 8;
    let block_align: u16 = num_channels * bits_per_sample / 8;
    let data_size: u32 = u32::try_from(samples.len() * 2).unwrap_or(u32::MAX);
    let chunk_size: u32 = 36_u32.saturating_add(data_size);

    let mut out = Vec::<u8>::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&chunk_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16_u32.to_le_bytes()); // PCM fmt subchunk size
    out.extend_from_slice(&1_u16.to_le_bytes()); // audio_format = PCM
    out.extend_from_slice(&num_channels.to_le_bytes());
    out.extend_from_slice(&sample_rate_hz.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());

    let max = f32::from(i16::MAX);
    for &s in samples {
        let clipped = s.clamp(-1.0, 1.0);
        let q = (clipped * max).round() as i16;
        out.extend_from_slice(&q.to_le_bytes());
    }

    out
}
