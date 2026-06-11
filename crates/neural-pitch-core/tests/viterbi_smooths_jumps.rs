#![allow(missing_docs)]
#![cfg(feature = "neural")]

//! Viterbi rejects high-frequency state jumps under a Gaussian
//! transition prior.
//!
//! Build emissions where alternate frames pull toward two distant
//! states (`A = 100`, `B = 200`, `delta = 100` bins). The per-frame argmax
//! would zig-zag A, B, A, B; under sigma = 2 bins the transition penalty
//! between A and B is on the order of `-(100/2)^2/2 = -1250` log-units,
//! dwarfing the modest emission preference for the alternating peak. The
//! optimal path therefore parks on whichever state has the larger sum of
//! emission log-probs across the sequence.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use neural_pitch_core::analysis::viterbi::{TransitionModel, decode};

/// Build a row biased toward `primary` (log-prob 0.0) and a smaller
/// secondary bump at `secondary` (log-prob `-0.1`). Everything else sits
/// at a deep log-floor so only those two states matter to the recurrence.
fn bimodal_log(n_states: usize, primary: usize, secondary: usize) -> Vec<f32> {
    const LOG_FLOOR: f32 = -1e3;
    let mut row = vec![LOG_FLOOR; n_states];
    row[primary] = 0.0;
    row[secondary] = -0.1;
    row
}

#[test]
fn viterbi_under_default_sigma_does_not_zigzag_between_distant_states() {
    let n_states = 384;
    let a = 100;
    let b = 200;

    // 16 frames alternating which mode is "primary". Total emission lead
    // for A = lead for B (each is primary 8 times). The deciding factor
    // is the transition cost — the smoother path wins.
    let mut emissions: Vec<Vec<f32>> = Vec::with_capacity(16);
    for t in 0..16 {
        if t % 2 == 0 {
            emissions.push(bimodal_log(n_states, a, b));
        } else {
            emissions.push(bimodal_log(n_states, b, a));
        }
    }

    let path = decode(&emissions, &TransitionModel::default());

    assert_eq!(path.len(), emissions.len());

    // Count A-flips: any frame where state[t] != state[t-1].
    let flips: usize = path.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(
        flips <= 1,
        "Viterbi must not zig-zag under sigma=2 — got {flips} state changes in {n} frames; \
         path = {path:?}",
        n = path.len()
    );

    // And the chosen state on every frame must be one of the two
    // candidates — never some unrelated bin smeared in by the prior.
    for (t, &s) in path.iter().enumerate() {
        assert!(
            s == a || s == b,
            "frame {t}: state {s} is neither A={a} nor B={b}",
        );
    }
}
