#![allow(missing_docs)]

//! `PYIN_ANALYZER_VERSION` static-contract sentinel.
//!
//! [`PYIN_ANALYZER_VERSION`] is `"0.2"`. The richer
//! `cache_version_0_1_to_0_2` integration test asserts the same equality
//! but is gated behind `#[cfg(feature = "pyin")]` because it exercises
//! the FLAC decode path. Under `cargo test --no-default-features` the
//! gated file compiles to an empty crate and the assertion vanishes — so
//! a contributor who reverts the bump and only runs
//! `--no-default-features` would see CI green.
//!
//! This sister test is unconditional: it carries the same equality check
//! but reads only [`PYIN_ANALYZER_VERSION`] and [`PYIN_ANALYZER_NAME`],
//! both of which are unconditionally exposed (see
//! `src/analysis/contour.rs`). The project hard rule promises both
//! feature matrices stay green; this file pins the static contract on the
//! no-default-features side too.

use neural_pitch_core::analysis::contour::{PYIN_ANALYZER_NAME, PYIN_ANALYZER_VERSION};

#[test]
fn pyin_analyzer_version_is_post_bump() {
    assert_eq!(
        PYIN_ANALYZER_VERSION, "0.2",
        "PYIN_ANALYZER_VERSION must be \"0.2\"; reverting to \"0.1\" \
         silently invalidates the cache back-compat contract.",
    );
    assert_eq!(PYIN_ANALYZER_NAME, "pyin");
}
