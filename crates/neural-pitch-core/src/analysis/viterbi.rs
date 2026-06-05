#![allow(clippy::doc_markdown)]
//! Phase 2.2 — generic frame-by-frame Viterbi decoder.
//!
//! `decode(emissions, transition)` runs a dynamic-programming maximisation
//! over a discrete state space and returns one best-state index per frame.
//! All math is held in log-space so probability products remain numerically
//! stable across long contours (40+ s offline analyses, hundreds of frames).
//!
//! The recurrence implemented is
//!
//! ```text
//! delta_t(j) = max_i [ delta_{t-1}(i) + log A(i, j) ] + log B_t(j)
//! ```
//!
//! where `A(i, j)` is the transition log-probability from state `i` to state
//! `j` and `log B_t(j)` is the emission log-probability of state `j` at time
//! `t`. After the forward pass the best terminal state argmax-es `delta_T`
//! and a back-pointer table walks the path back to `t = 0`.
//!
//! Phase 2.2 wires this only for the neural backends — PESTO / CREPE emit
//! cents-bin frame-level probabilities and the offline pipeline calls
//! [`decode`] with a Gaussian transition model centred on the previous
//! state. The decoder is intentionally backend-agnostic: any future
//! classical estimator that wants HMM smoothing can lift the gate and
//! depend on this module.
//!
//! # Hot-path discipline
//!
//! [`decode`] allocates the `delta` and back-pointer matrices (`O(T * N)`
//! each). It is **not** intended for the live tuner path; offline pipelines
//! batch a window of frames and call this once per analysis.

/// Transition model for the Viterbi recurrence.
///
/// The default Gaussian model centres a normal-shaped log-penalty on the
/// previous state with `sigma = 2` bins (= 40 cents at the canonical
/// 20-cents-per-bin PESTO grid) and adds a small self-loop bonus that
/// slightly favours staying in the same state on quiet frames.
#[derive(Clone, Debug)]
pub struct TransitionModel {
    /// Standard deviation, in state-index units (typically cents bins).
    pub sigma_bins: f32,
    /// Additive log-prob bonus applied to the diagonal `i == j` entry.
    pub self_loop_log_bonus: f32,
}

impl Default for TransitionModel {
    fn default() -> Self {
        Self {
            sigma_bins: 2.0,
            self_loop_log_bonus: 0.5,
        }
    }
}

impl TransitionModel {
    /// Return the log-probability of transitioning from state `i` to state
    /// `j` under this model.
    ///
    /// The kernel is an un-normalised log-Gaussian centred on `i` with
    /// standard deviation `sigma_bins`, plus the additive
    /// `self_loop_log_bonus` on the diagonal. Normalisation is omitted
    /// because Viterbi cares only about relative log-probabilities along
    /// each row.
    #[inline]
    fn log_transition(&self, i: usize, j: usize) -> f32 {
        let d = j as f32 - i as f32;
        let sigma = self.sigma_bins.max(f32::EPSILON);
        let mut lp = -(d * d) / (2.0 * sigma * sigma);
        if i == j {
            lp += self.self_loop_log_bonus;
        }
        lp
    }
}

/// Run Viterbi over `emissions` and return one best-state index per frame.
///
/// `emissions[t][j]` is the **log**-probability of emitting state `j` at
/// time `t`. Implementations MAY hold raw probabilities upstream and
/// `.ln()` them before calling here; this function does not normalise.
///
/// # Panics
///
/// Returns an empty `Vec` when `emissions` is empty. Otherwise all rows
/// MUST have the same length (the state-space cardinality `N`).
#[must_use]
pub fn decode(emissions: &[Vec<f32>], transition: &TransitionModel) -> Vec<usize> {
    if emissions.is_empty() {
        return Vec::new();
    }
    let n_frames = emissions.len();
    let n_states = emissions[0].len();
    if n_states == 0 {
        return vec![0; n_frames];
    }

    // `delta[j]` = best log-prob of any path ending in state `j` at frame
    // `t`. We keep two rolling rows (`prev`, `curr`) instead of a full
    // T x N matrix to keep the working set small.
    let mut prev = vec![f32::NEG_INFINITY; n_states];
    let mut curr = vec![f32::NEG_INFINITY; n_states];
    // Back-pointer table: `bptr[t][j]` = the predecessor state of `j` at
    // frame `t`. `bptr[0]` is unused but kept so indexing is uniform.
    let mut bptr: Vec<Vec<usize>> = vec![vec![0_usize; n_states]; n_frames];

    // Initialisation: delta_0(j) = log B_0(j). Uniform prior on the
    // initial state distribution per the standard Viterbi setup.
    prev[..n_states].copy_from_slice(&emissions[0][..n_states]);

    // Forward pass. For each frame t = 1 .. T-1 and each destination
    // state j, find the predecessor i that maximises
    //    delta_{t-1}(i) + log A(i, j)
    // and add log B_t(j).
    for t in 1..n_frames {
        let row_t = &emissions[t];
        debug_assert_eq!(
            row_t.len(),
            n_states,
            "viterbi::decode: all emission rows must have the same width",
        );
        for (j, curr_j) in curr.iter_mut().enumerate() {
            let mut best_score = f32::NEG_INFINITY;
            let mut best_i: usize = 0;
            for (i, &prev_i) in prev.iter().enumerate() {
                let score = prev_i + transition.log_transition(i, j);
                if score > best_score {
                    best_score = score;
                    best_i = i;
                }
            }
            *curr_j = best_score + row_t[j];
            bptr[t][j] = best_i;
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    // Termination: argmax of `prev` (which now holds `delta_{T-1}`).
    let mut best_terminal: usize = 0;
    let mut best_score = f32::NEG_INFINITY;
    for (j, &score) in prev.iter().enumerate() {
        if score > best_score {
            best_score = score;
            best_terminal = j;
        }
    }

    // Back-pointer walk: rebuild the path from `t = T-1` down to `t = 0`.
    let mut path = vec![0_usize; n_frames];
    path[n_frames - 1] = best_terminal;
    for t in (1..n_frames).rev() {
        path[t - 1] = bptr[t][path[t]];
    }
    path
}
