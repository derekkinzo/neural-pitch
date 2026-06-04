//! Quick round-trip verification for the Phase 1.4 Tier-2 voice fixtures.
//!
//! Decodes one fixture (A4 / 440 Hz) via `claxon`, runs a textbook
//! parabolic-interpolation FFT pitch estimator, and prints the deviation
//! in cents from the synth_voice input. Used during Phase 1.4 bring-up
//! to confirm the FLAC encode → decode pipeline does not corrupt pitch.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(clippy::print_stdout, clippy::cast_possible_wrap)]

use std::path::Path;

use claxon::FlacReader;

fn main() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let path = Path::new(manifest)
        .join("tests")
        .join("fixtures")
        .join("voice")
        .join("069_A4_synthvoice_clean.flac");

    let mut reader = FlacReader::open(&path).expect("open flac");
    let info = reader.streaminfo();
    let sr = info.sample_rate;
    let bits = info.bits_per_sample;
    let max_val = (1_i32 << (bits - 1)) as f32;

    let mut samples: Vec<f32> = Vec::new();
    for s in reader.samples() {
        let s = s.expect("decode sample");
        samples.push(s as f32 / max_val);
    }
    println!(
        "decoded {} samples @ {} Hz, {} bits",
        samples.len(),
        sr,
        bits
    );

    // Take the centre 2048-sample window for analysis.
    let win = 2048usize.min(samples.len());
    let start = (samples.len() - win) / 2;
    let frame: Vec<f32> = samples[start..start + win].to_vec();

    // Window with Hann to reduce side-lobes.
    let n = frame.len();
    let pi = std::f32::consts::PI;
    let windowed: Vec<f32> = frame
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let w = 0.5 - 0.5 * ((2.0 * pi * i as f32) / (n as f32 - 1.0)).cos();
            s * w
        })
        .collect();

    // Autocorrelation-based fundamental finder. Robust to formant emphasis
    // on upper partials (the harmonic stack + F2/F3 boost can put the
    // strongest spectral peak at the 3rd harmonic, but autocorrelation
    // tracks the fundamental period directly).
    let target_hz = 440.0_f32;
    let sr_f = sr as f32;
    // Search lag window around the expected period ±50 cents — enough
    // headroom that we'd catch a real round-trip drift.
    let period_target = sr_f / target_hz;
    let lag_min = ((period_target * 0.5).floor() as usize).max(2);
    let lag_max = ((period_target * 2.0).ceil() as usize).min(windowed.len() / 2);

    let mean: f32 = windowed.iter().sum::<f32>() / windowed.len() as f32;
    let centred: Vec<f32> = windowed.iter().map(|&s| s - mean).collect();

    let mut best_lag = lag_min;
    let mut best_corr = f32::NEG_INFINITY;
    for lag in lag_min..=lag_max {
        let mut acc = 0.0_f32;
        for i in 0..(centred.len() - lag) {
            acc += centred[i] * centred[i + lag];
        }
        if acc > best_corr {
            best_corr = acc;
            best_lag = lag;
        }
    }

    // Parabolic interpolation across the integer-lag peak for sub-sample
    // accuracy. Skip if the peak sits on an edge.
    let lag_refined = if best_lag > lag_min && best_lag < lag_max {
        let acc_at = |l: usize| {
            let mut a = 0.0_f32;
            for i in 0..(centred.len() - l) {
                a += centred[i] * centred[i + l];
            }
            a
        };
        let y0 = acc_at(best_lag - 1);
        let y1 = best_corr;
        let y2 = acc_at(best_lag + 1);
        let denom = y0 - 2.0 * y1 + y2;
        if denom.abs() > 1e-9 {
            best_lag as f32 + 0.5 * (y0 - y2) / denom
        } else {
            best_lag as f32
        }
    } else {
        best_lag as f32
    };

    let measured_hz = sr_f / lag_refined;
    let cents = 1200.0 * (measured_hz / target_hz).log2();
    println!(
        "autocorr fundamental: {measured_hz:.3} Hz, |Δcents| from {target_hz} Hz = {:.3}",
        cents.abs()
    );
    if cents.abs() < 5.0 {
        println!("ROUND-TRIP OK (within 5 cents)");
    } else {
        println!("ROUND-TRIP DRIFT (exceeds 5 cents)");
    }
}
