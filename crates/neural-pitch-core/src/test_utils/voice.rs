//! Synthetic voice signal generator for the voice-acceptance fixtures.
//!
//! `synth_voice` produces a deterministic mono `Vec<f32>` waveform that
//! approximates a held vocal note: a 4-partial harmonic stack with optional
//! vibrato (rate, depth) and optional formant-shaped colouration (cascaded
//! biquad bandpasses at F1/F2/F3 of a neutral schwa vowel).
//!
//! Determinism is load-bearing: output is closed-form deterministic — phase
//! is integrated per harmonic, vibrato uses a closed-form sin modulator,
//! and there is no random or thread-local state involved. Identical inputs
//! always produce byte-identical output, verified by the
//! `synth_voice_is_deterministic` test below; this is what lets
//! `examples/build_voice_fixtures.rs` produce reproducible FLAC fixtures.
//!
//! Any future breath-noise component must preserve the determinism
//! contract above by seeding any RNG from the input parameters.
//!
//! Lint policy: this module is compiled into the production crate (under
//! `pub mod test_utils`), so the workspace `unwrap_used`/`expect_used`/
//! `panic` denials apply. All numerical operations here are total — no
//! divisions by zero, no array indexing without a bounds check.

use core::f32::consts::TAU;

/// Maximum absolute amplitude after peak-normalisation.
///
/// Mirrors `signals::PEAK` so synthetic-voice fixtures share the same
/// headroom convention as the unit-test signal generators.
const PEAK: f32 = 0.95;

/// Harmonic weights for partials 1..=4 — a coarse glottal-source spectral
/// tilt approximation. Sum need not be unity; the output is peak-normalised
/// before return.
const HARMONIC_WEIGHTS: [f32; 4] = [1.0, 0.5, 0.25, 0.125];

/// Formant centre frequencies (Hz) for a neutral `~/ə/` (schwa) vowel.
/// Values from Peterson & Barney (1952) tables, central-vowel column.
const FORMANT_HZ: [f32; 3] = [500.0, 1500.0, 2500.0];

/// Formant bandwidth (Hz) — single shared bandwidth for all three formants.
/// Wider than a true vocal-tract Q to keep partial peaks audible across the
/// SATB range without filter-design subtlety.
const FORMANT_BANDWIDTH_HZ: f32 = 100.0;

/// Generate a synthetic-voice waveform.
///
/// # Arguments
///
/// - `f0_hz`: fundamental frequency (Hz). MUST be `> 0`. Values outside
///   `[20.0, sample_rate / 2 / 4]` clamp the highest harmonic so it never
///   exceeds Nyquist.
/// - `sample_rate`: output sample rate (Hz). Typical value is `48_000`,
///   matching the live-capture rate used throughout the crate.
/// - `n_samples`: number of output samples. The total duration is
///   `n_samples / sample_rate` seconds.
/// - `vibrato`: optional `(rate_hz, depth_cents)`. When `Some`, the
///   instantaneous fundamental sweeps `±depth_cents` around `f0_hz` at
///   `rate_hz` Hz. The modulator is phase-aligned so the analysis-window
///   centre lies on a modulator zero crossing — matches `vibrato_signal`
///   in `signals.rs`, so YIN and the auto-prior see the same envelope
///   shape across both unit-test and voice-acceptance fixtures.
/// - `formants`: when `true`, the harmonic stack is filtered through three
///   cascaded biquad band-passes (F1=500 Hz, F2=1500 Hz, F3=2500 Hz). The
///   filter chain is intentionally crude — its purpose is to give YIN
///   realistic vowel-like spectral colour to fight with, not to model a
///   real vocal tract.
///
/// # Determinism
///
/// Output is closed-form deterministic: phase is integrated per harmonic,
/// vibrato uses a closed-form sin modulator, no random source is involved.
/// Two calls with identical arguments produce byte-identical output.
///
/// # Lint policy
///
/// No `unwrap`, no `expect`, no `panic`. All branches are total.
pub fn synth_voice(
    f0_hz: f32,
    sample_rate: u32,
    n_samples: usize,
    vibrato: Option<(f32, f32)>,
    formants: bool,
) -> Vec<f32> {
    if n_samples == 0 || sample_rate == 0 || !f0_hz.is_finite() || f0_hz <= 0.0 {
        return vec![0.0; n_samples];
    }

    let sr = sample_rate as f32;
    let nyquist = sr * 0.5;

    // Pre-compute vibrato modulator parameters with zero-crossing centring.
    // We use the same form as `signals::vibrato_signal`: instantaneous
    // frequency follows f0 * 2^(extent * sin(2*pi*v*t + phi)), with phi
    // chosen so the modulator is zero at the window centre.
    let (vib_rate, vib_log2_ratio, vib_phase_offset) = match vibrato {
        Some((rate_hz, depth_cents)) if rate_hz > 0.0 && depth_cents.is_finite() => {
            let log2_ratio = depth_cents / 1200.0;
            let t_centre = (n_samples as f32 / sr) * 0.5;
            let phi = -TAU * rate_hz * t_centre;
            (rate_hz, log2_ratio, phi)
        }
        _ => (0.0, 0.0, 0.0),
    };

    // Generate the harmonic-stack waveform with per-sample frequency
    // modulation. We integrate phase per harmonic so frequency changes are
    // well-defined even when the modulator changes mid-window.
    let mut out = vec![0.0_f32; n_samples];
    let mut harmonic_phases: [f32; 4] = [0.0; 4];

    for (n, slot) in out.iter_mut().enumerate() {
        let t = n as f32 / sr;
        let mod_octaves = if vib_rate > 0.0 {
            vib_log2_ratio * (TAU * vib_rate * t + vib_phase_offset).sin()
        } else {
            0.0
        };
        let f_inst = f0_hz * mod_octaves.exp2();

        let mut sample = 0.0_f32;
        for (k, (&weight, phase_state)) in HARMONIC_WEIGHTS
            .iter()
            .zip(harmonic_phases.iter_mut())
            .enumerate()
        {
            let harmonic = (k + 1) as f32;
            let f_harm = f_inst * harmonic;
            // Suppress harmonics above Nyquist to prevent aliasing.
            if f_harm >= nyquist {
                continue;
            }
            let phase = *phase_state + TAU * f_harm / sr;
            *phase_state = phase;
            sample += weight * phase.sin();
        }
        *slot = sample;
    }

    if formants {
        apply_formants(&mut out, sample_rate);
    }

    normalise_peak(&mut out, PEAK);
    out
}

/// Apply three cascaded biquad band-pass filters (constant skirt, peak gain
/// = Q), one per formant in [`FORMANT_HZ`]. Single-pass, in-place. Each
/// filter follows the RBJ Audio EQ Cookbook BPF (constant 0 dB peak gain
/// variant); cascading approximates a vocal tract resonance pattern well
/// enough for the voice-acceptance harness.
fn apply_formants(buf: &mut [f32], sample_rate: u32) {
    if buf.is_empty() || sample_rate == 0 {
        return;
    }
    let sr = sample_rate as f32;
    for &f0 in &FORMANT_HZ {
        let q = f0 / FORMANT_BANDWIDTH_HZ;
        let coeffs = bandpass_biquad(f0, q, sr);
        run_biquad(buf, coeffs);
    }
}

/// Biquad coefficients in direct-form-1 order:
/// `(b0, b1, b2, a1, a2)`. `a0` is folded into the others (i.e. all
/// returned values are pre-divided by `a0`).
type BiquadCoeffs = (f32, f32, f32, f32, f32);

/// RBJ Audio EQ Cookbook band-pass filter (constant 0 dB peak gain).
///
/// Returns coefficients normalised by `a0`. Stable for any `f0 < sr/2` and
/// `q > 0`. Inputs outside that range collapse to a unity passthrough so
/// callers do not need to pre-validate — important since the workspace
/// lint policy denies `panic` and we cannot fall back to `unwrap`.
fn bandpass_biquad(f0: f32, q: f32, sr: f32) -> BiquadCoeffs {
    if !(f0.is_finite() && q.is_finite() && sr.is_finite())
        || f0 <= 0.0
        || q <= 0.0
        || sr <= 0.0
        || f0 >= sr * 0.5
    {
        return (1.0, 0.0, 0.0, 0.0, 0.0); // identity passthrough
    }
    let omega = TAU * f0 / sr;
    let sin_w = omega.sin();
    let cos_w = omega.cos();
    let alpha = sin_w / (2.0 * q);

    let b0 = alpha;
    let b1 = 0.0;
    let b2 = -alpha;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w;
    let a2 = 1.0 - alpha;

    if a0.abs() < f32::EPSILON {
        return (1.0, 0.0, 0.0, 0.0, 0.0);
    }

    (b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0)
}

/// Direct-form-1 biquad in-place runner. All-zero state at start.
fn run_biquad(buf: &mut [f32], coeffs: BiquadCoeffs) {
    let (b0, b1, b2, a1, a2) = coeffs;
    let mut x1 = 0.0_f32;
    let mut x2 = 0.0_f32;
    let mut y1 = 0.0_f32;
    let mut y2 = 0.0_f32;
    for v in buf.iter_mut() {
        let x0 = *v;
        let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        x2 = x1;
        x1 = x0;
        y2 = y1;
        y1 = y0;
        *v = y0;
    }
}

/// Peak-normalise `buf` to `target_peak`. No-op if `buf` is already silent.
fn normalise_peak(buf: &mut [f32], target_peak: f32) {
    let mut peak = 0.0_f32;
    for &v in buf.iter() {
        let a = v.abs();
        if a > peak {
            peak = a;
        }
    }
    if peak == 0.0 || !peak.is_finite() {
        return;
    }
    let g = target_peak / peak;
    for v in buf.iter_mut() {
        *v *= g;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synth_voice_is_deterministic() {
        let a = synth_voice(440.0, 48_000, 2048, None, true);
        let b = synth_voice(440.0, 48_000, 2048, None, true);
        assert_eq!(a, b);
    }

    #[test]
    fn synth_voice_handles_zero_samples() {
        let v = synth_voice(440.0, 48_000, 0, None, false);
        assert!(v.is_empty());
    }

    #[test]
    fn synth_voice_handles_invalid_f0() {
        let v = synth_voice(0.0, 48_000, 256, None, false);
        assert_eq!(v.len(), 256);
        assert!(v.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn synth_voice_peak_within_target() {
        let v = synth_voice(220.0, 48_000, 4096, None, true);
        let peak = v.iter().copied().fold(0.0_f32, |acc, s| acc.max(s.abs()));
        assert!(peak <= 0.96, "peak {peak} exceeded target");
        assert!(peak > 0.5, "peak {peak} suspiciously low");
    }

    #[test]
    fn synth_voice_with_vibrato_does_not_panic() {
        let v = synth_voice(330.0, 48_000, 4096, Some((5.0, 50.0)), true);
        assert_eq!(v.len(), 4096);
    }
}
