#![allow(missing_docs)]
#![cfg(feature = "neural")]

//! Phase 2.2 RED — Viterbi over single-peak-per-frame emissions.
//!
//! When every frame's emission distribution is sharply unimodal, Viterbi
//! is degenerate: the chosen state is the per-frame argmax regardless of
//! the transition prior. This test pins that property as a baseline so
//! later refactors of the transition kernel can't accidentally weight
//! the prior strongly enough to override clear evidence.
//!
//! TDD-RED: the assertions are sketched against the spec; the body of
//! [`neural_pitch_core::analysis::viterbi::decode`] is `todo!()`, so the
//! test currently panics on the first call. Phase 2.2 GREEN turns this
//! green by wiring the log-domain forward pass and back-pointer walk.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::analysis::viterbi::{TransitionModel, decode};

/// Build an emission row of length `n_states` with all bins at
/// `LOG_FLOOR` except the chosen `peak` bin set to `0.0` (log(1) — the
/// undisputed argmax). All math is log-domain per the decoder contract.
fn one_hot_log(n_states: usize, peak: usize) -> Vec<f32> {
    const LOG_FLOOR: f32 = -1e3;
    let mut row = vec![LOG_FLOOR; n_states];
    row[peak] = 0.0;
    row
}

#[test]
fn viterbi_constant_emission_returns_argmax_path() {
    let n_states = 384; // PESTO cents bins.
    let peaks = [120_usize, 121, 122, 123, 122, 121, 120, 119, 118, 117];
    let emissions: Vec<Vec<f32>> = peaks.iter().map(|&p| one_hot_log(n_states, p)).collect();

    let path = decode(&emissions, &TransitionModel::default());

    assert_eq!(path.len(), peaks.len(), "one state per frame");
    for (t, (got, want)) in path.iter().zip(peaks.iter()).enumerate() {
        assert_eq!(
            got, want,
            "frame {t}: expected argmax bin {want}, got {got}",
        );
    }
}

#[test]
fn viterbi_empty_emissions_returns_empty_path() {
    let emissions: Vec<Vec<f32>> = vec![];
    let path = decode(&emissions, &TransitionModel::default());
    assert!(path.is_empty(), "empty input must yield empty path");
}

/// Single-frame input — exercises the path where the back-pointer walk
/// loop `for t in (1..n_frames).rev()` is a no-op and the only frame's
/// state must come from `argmax(emissions[0])`. Without this test, the
/// `n_frames == 1` corner is not pinned anywhere.
#[test]
fn viterbi_single_frame_returns_argmax_of_only_row() {
    let n_states = 384;
    let peak = 73_usize;
    let row = one_hot_log(n_states, peak);
    let path = decode(&[row], &TransitionModel::default());
    assert_eq!(path.len(), 1, "n_frames=1 must yield one state");
    assert_eq!(
        path[0], peak,
        "single-frame decode must return argmax of the only emission row"
    );
}
