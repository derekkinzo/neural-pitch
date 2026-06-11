#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! `DrillResult::from_attempts` aggregates per-attempt outcomes into
//! `accuracy()` and `mean_cents_error_abs`.
//!
//! Eight attempts with six correct and a mean absolute error of
//! 7.5 cents must yield `accuracy() == 0.75` and
//! `mean_cents_error_abs ~= 7.5`.

use approx::assert_relative_eq;
use neural_pitch_core::training::drill::DrillAttempt;
use neural_pitch_core::training::{ChordQuality, Drill, DrillResult, DrillSpec};

#[test]
fn six_of_eight_correct_yields_accuracy_seventy_five_percent() {
    let spec = DrillSpec {
        drill: Drill::ChordQualityId {
            qualities: vec![
                ChordQuality::Major,
                ChordQuality::Minor,
                ChordQuality::Diminished,
                ChordQuality::Dominant7,
            ],
        },
        attempts: 8,
        tolerance_cents: 25.0,
    };

    // Build 8 attempts: 6 correct (errors: 5,5,5,10,10,10) + 2 incorrect
    // (errors: 5,10). Sum = 55 → mean = 6.875. Adjust: 8 attempts with
    // mean 7.5 needs total error 60. Use:
    //   correct: 5,5,5,10,10,10 (sum 45)
    //   incorrect: 7.5, 7.5    (sum 15)
    // Total = 60, mean = 7.5.
    let attempts = vec![
        DrillAttempt {
            correct: true,
            cents_error_abs: 5.0,
        },
        DrillAttempt {
            correct: true,
            cents_error_abs: 5.0,
        },
        DrillAttempt {
            correct: true,
            cents_error_abs: 5.0,
        },
        DrillAttempt {
            correct: true,
            cents_error_abs: 10.0,
        },
        DrillAttempt {
            correct: true,
            cents_error_abs: 10.0,
        },
        DrillAttempt {
            correct: true,
            cents_error_abs: 10.0,
        },
        DrillAttempt {
            correct: false,
            cents_error_abs: 7.5,
        },
        DrillAttempt {
            correct: false,
            cents_error_abs: 7.5,
        },
    ];

    let result: DrillResult = DrillResult::from_attempts(spec, &attempts);
    assert_relative_eq!(result.accuracy(), 0.75_f32, epsilon = 1e-4);
    assert_relative_eq!(result.mean_cents_error_abs, 7.5_f32, epsilon = 1e-3);
}
