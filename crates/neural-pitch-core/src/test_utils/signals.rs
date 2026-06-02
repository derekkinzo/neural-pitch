//! Deterministic signal generators for unit, integration, and property tests.
//!
//! All generators return owned `Vec<f32>` and normalise to a peak amplitude
//! of `0.95` (leaving headroom for downstream gain). Random noise uses a
//! deterministic LCG so tests do not pull in the `rand` crate at the
//! day-1 dependency budget.

use core::f32::consts::TAU;

/// Maximum absolute amplitude after peak-normalisation.
const PEAK: f32 = 0.95;

/// Generate a pure sine wave at `freq_hz`, `n_samples` long, normalised to
/// peak [`PEAK`].
pub fn sine_wave(freq_hz: f32, sample_rate_hz: u32, n_samples: usize) -> Vec<f32> {
    let sr = sample_rate_hz as f32;
    let mut out = Vec::with_capacity(n_samples);
    for n in 0..n_samples {
        let t = n as f32 / sr;
        out.push((TAU * freq_hz * t).sin());
    }
    normalise_peak(&mut out, PEAK);
    out
}

/// Generate a vibrato signal: a sinusoid whose instantaneous frequency
/// oscillates around `center_hz` at `vibrato_hz`, with peak deviation
/// `vibrato_extent_cents` cents.
///
/// The vibrato modulator is phase-shifted so the *centre* of the requested
/// window sits exactly on a zero crossing of the modulator. As a result the
/// average instantaneous frequency over the window is exactly `center_hz`,
/// even when the window covers less than a full vibrato period. Without this
/// shift, a window starting at the rising edge of a 5 Hz vibrato modulator
/// would carry a +20-30 cent bias for any 42 ms analysis frame, which would
/// make `yin_vibrato_within_10_cents` unsatisfiable.
pub fn vibrato_signal(
    center_hz: f32,
    vibrato_hz: f32,
    vibrato_extent_cents: f32,
    sample_rate_hz: u32,
    n_samples: usize,
) -> Vec<f32> {
    let sr = sample_rate_hz as f32;
    // Use phase-accumulator integration so the instantaneous frequency
    // matches the analytic intent. Phase increment per sample = 2*pi*f(t)/sr.
    let mut out = Vec::with_capacity(n_samples);
    let mut phase: f32 = 0.0;
    let extent_ratio = (vibrato_extent_cents / 1200.0).exp2(); // semitone ratio at peak
    let log2_ratio = (extent_ratio).log2(); // == vibrato_extent_cents / 1200
    // Centre the window on a zero crossing of the modulator: `sin(2pi*v*t + phi)`
    // is zero at t = centre when phi = -2*pi*v*t_centre.
    let t_centre = (n_samples as f32 / sr) * 0.5;
    let mod_phase_offset = -TAU * vibrato_hz * t_centre;
    for n in 0..n_samples {
        let t = n as f32 / sr;
        // Frequency-modulation: f(t) = center * 2^(extent_octaves * sin(2*pi*v*t + phi))
        let mod_octaves = log2_ratio * (TAU * vibrato_hz * t + mod_phase_offset).sin();
        let f_inst = center_hz * mod_octaves.exp2();
        phase += TAU * f_inst / sr;
        out.push(phase.sin());
    }
    normalise_peak(&mut out, PEAK);
    out
}

/// Mix two equal-amplitude sinusoids at `f1` and `f2`. The first tone is
/// `0.5` louder than the second so `two_tone_picks_louder`-style tests have
/// a deterministic answer.
pub fn two_tone(f1: f32, f2: f32, sample_rate_hz: u32, n_samples: usize) -> Vec<f32> {
    let sr = sample_rate_hz as f32;
    let mut out = Vec::with_capacity(n_samples);
    for n in 0..n_samples {
        let t = n as f32 / sr;
        let a = 1.0 * (TAU * f1 * t).sin();
        let b = 0.5 * (TAU * f2 * t).sin();
        out.push(a + b);
    }
    normalise_peak(&mut out, PEAK);
    out
}

/// Deterministic white-noise generator using a 32-bit LCG.
///
/// `seed` is the starting state; identical seeds produce identical output.
pub fn white_noise(_sample_rate_hz: u32, n_samples: usize, seed: u32) -> Vec<f32> {
    // Numerical Recipes LCG constants.
    let mut state: u32 = seed.wrapping_add(1);
    let mut out = Vec::with_capacity(n_samples);
    for _ in 0..n_samples {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        // Map to [-1, 1).
        let f = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
        out.push(f);
    }
    normalise_peak(&mut out, PEAK);
    out
}

/// `n_samples` of pure silence.
pub fn silence(n_samples: usize) -> Vec<f32> {
    vec![0.0; n_samples]
}

/// Mix two signals so the first has a target signal-to-noise ratio over the
/// second. `a` and `b` should be the same length; the shorter one is
/// truncated. The result is peak-normalised to [`PEAK`].
pub fn mix(a: &[f32], b: &[f32], snr_db: f32) -> Vec<f32> {
    let n = a.len().min(b.len());
    let pa: f32 = power(&a[..n]);
    let pb: f32 = power(&b[..n]);
    if pa == 0.0 || pb == 0.0 {
        let mut out = a[..n].to_vec();
        if out.is_empty() {
            return out;
        }
        normalise_peak(&mut out, PEAK);
        return out;
    }
    // Scale b so that pa / (pb * scale^2) = 10^(snr_db/10)
    let target = 10.0_f32.powf(snr_db / 10.0);
    let scale = (pa / (pb * target)).sqrt();
    let mut out: Vec<f32> = (0..n).map(|i| a[i] + scale * b[i]).collect();
    normalise_peak(&mut out, PEAK);
    out
}

fn power(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    let s: f32 = x.iter().map(|v| v * v).sum();
    s / x.len() as f32
}

fn normalise_peak(x: &mut [f32], target_peak: f32) {
    let mut peak = 0.0_f32;
    for v in x.iter() {
        let a = v.abs();
        if a > peak {
            peak = a;
        }
    }
    if peak == 0.0 {
        return;
    }
    let g = target_peak / peak;
    for v in x.iter_mut() {
        *v *= g;
    }
}
