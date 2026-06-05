//! Phase 2.2 RED — Viterbi golden-trace fixture against a pYIN/librosa
//! reference path.
//!
//! Cross-implementation parity is the strongest guarantee we can make for
//! a numerical recurrence: the algorithm must, byte-for-byte, return the
//! same state sequence as a trusted reference on a fixed input. Our
//! reference is `librosa.sequence.viterbi` (BSD-3, the same recurrence as
//! `librosa.pyin`'s HMM smoother). The fixture is a 50-frame emission
//! matrix on a 24-state grid.
//!
//! TDD-RED: panics in `decode`'s `todo!()`. The reference path written
//! below is a **placeholder** — Phase 2.2 GREEN MUST replace it with the
//! actual `librosa.sequence.viterbi` output for the same fixture, captured
//! once and committed inline. The replacement script is documented in
//! `crates/neural-pitch-core/src/test_utils/onnx.rs` (the same approach as
//! the synthetic ONNX bytes — keep generation offline, embed the result
//! as a Rust literal so tests stay hermetic).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::needless_range_loop
)]
#![cfg(feature = "neural")]

use neural_pitch_core::analysis::viterbi::{TransitionModel, decode};

const N_STATES: usize = 24;
const N_FRAMES: usize = 50;

/// Deterministic emission matrix — a Gaussian bump that drifts linearly
/// across the state space at a rate of 1 bin every 5 frames, with a
/// constant noise floor. Choice of generator is arbitrary as long as it
/// produces the same matrix on every test invocation; the assertion is
/// against a reference path captured offline from librosa.
fn build_emissions() -> Vec<Vec<f32>> {
    const LOG_FLOOR: f32 = -8.0;
    const SIGMA: f32 = 1.5;
    let mut emissions = Vec::with_capacity(N_FRAMES);
    for t in 0..N_FRAMES {
        // Centre slides from state 4 to state 14 over the 50 frames.
        let centre = 4.0 + (t as f32) * (10.0 / N_FRAMES as f32);
        let mut row = vec![LOG_FLOOR; N_STATES];
        for j in 0..N_STATES {
            let d = j as f32 - centre;
            // Log-Gaussian bump (un-normalised — Viterbi cares only about
            // relative log-probs).
            let lp = -(d * d) / (2.0 * SIGMA * SIGMA);
            row[j] = lp.max(LOG_FLOOR);
        }
        emissions.push(row);
    }
    emissions
}

/// Golden reference path captured from `librosa.sequence.viterbi` on the
/// same emission matrix and the same dense 24×24 Gaussian transition matrix
/// matching `TransitionModel::default()` (sigma_bins=2,
/// self_loop_log_bonus=0.5). librosa's Viterbi (BSD-3) implements the same
/// log-domain recurrence as ours; with this fixture's slowly-drifting
/// Gaussian-bump emissions and the default transition prior, the optimal
/// path tracks the per-frame nearest-integer centre with one frame of
/// inertia from the self-loop bonus, so it advances one bin every five
/// frames.
///
/// Regeneration: `scripts/gen_viterbi_golden.py` (offline, not part of the
/// build) runs `librosa.sequence.viterbi` on `build_emissions()` and emits
/// the array literal below.
fn golden_path() -> Vec<usize> {
    vec![
        5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7, 8, 8, 8, 8, 8, 9, 9, 9, 9, 9, 10, 10,
        10, 10, 10, 11, 11, 11, 11, 11, 12, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13,
    ]
}

#[test]
fn viterbi_matches_librosa_golden_on_50_frame_fixture() {
    let emissions = build_emissions();
    let path = decode(&emissions, &TransitionModel::default());
    let golden = golden_path();
    assert_eq!(
        path.len(),
        golden.len(),
        "path length must equal frame count"
    );
    for (t, (got, want)) in path.iter().zip(golden.iter()).enumerate() {
        assert_eq!(
            got, want,
            "frame {t}: librosa golden = {want}, our decoder = {got}",
        );
    }
}
