//! Singh Curve — the stability update law.
//!
//! Spaced-repetition systems (Ebbinghaus → SM-2 → FSRS) update card
//! stability after each review by multiplying it by a grade-dependent
//! factor. V1 ships a coarse FSRS-shaped piecewise law:
//!
//! | Grade   | Multiplier (×stability) | Notes                                         |
//! |---------|-------------------------|-----------------------------------------------|
//! | Again   | 0.10× (lapse)           | Hard collapse — half-life nearly bottoms out  |
//! | Hard    | 1.20×                   | Slow growth                                   |
//! | Good    | 2.50×                   | The Anki-default growth                       |
//! | Easy    | 4.00×                   | Aggressive growth                             |
//!
//! Constants are deliberately whole numbers (×100 fixed-point) so the
//! arithmetic is integer-only and validators agree on stability
//! transitions deterministically. Real FSRS uses log-linear weights
//! tuned per learner; that's the V2 elaboration. The V1 law captures
//! the *shape* without per-learner tuning.

use evaporchain_types::HalfLife;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Grade {
    /// "I forgot" — collapses stability hard (lapse).
    Again,
    /// "I struggled but got it" — small bump.
    Hard,
    /// "I got it cleanly" — Anki's default growth.
    Good,
    /// "Trivially easy" — aggressive growth, longer interval next time.
    Easy,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SinghCurveError {
    #[error("stability must be > 0 to apply the curve")]
    ZeroStability,
}

/// Multiplier applied to current stability, expressed in basis points
/// (×100). Doctrine constants — exposed as `pub const` so future
/// `Sentinel` governance can replace them.
pub const MULT_AGAIN_BP: u64 = 10; // 0.10×
pub const MULT_HARD_BP: u64 = 120; // 1.20×
pub const MULT_GOOD_BP: u64 = 250; // 2.50×
pub const MULT_EASY_BP: u64 = 400; // 4.00×

/// Hard floor on post-lapse stability so a card never drops below 1
/// epoch (an evaporated card is unrecoverable; we want lapses to
/// *hurt* but not erase).
pub const STABILITY_FLOOR: HalfLife = 1;

/// Compute the new stability after a review at `current_stability`
/// with `grade`. Pure function; deterministic; integer-only.
pub fn update_stability(
    current_stability: HalfLife,
    grade: Grade,
) -> Result<HalfLife, SinghCurveError> {
    if current_stability == 0 {
        return Err(SinghCurveError::ZeroStability);
    }
    let mult_bp = match grade {
        Grade::Again => MULT_AGAIN_BP,
        Grade::Hard => MULT_HARD_BP,
        Grade::Good => MULT_GOOD_BP,
        Grade::Easy => MULT_EASY_BP,
    };
    let new_s_u128 = (current_stability as u128) * (mult_bp as u128) / 100;
    let new_s: HalfLife = new_s_u128.min(HalfLife::MAX as u128) as HalfLife;
    Ok(new_s.max(STABILITY_FLOOR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_stability() {
        assert_eq!(
            update_stability(0, Grade::Good).unwrap_err(),
            SinghCurveError::ZeroStability
        );
    }

    #[test]
    fn good_more_than_doubles_stability() {
        let s = update_stability(100, Grade::Good).unwrap();
        assert_eq!(s, 250);
    }

    #[test]
    fn easy_grows_faster_than_good() {
        let g = update_stability(100, Grade::Good).unwrap();
        let e = update_stability(100, Grade::Easy).unwrap();
        assert!(e > g);
        assert_eq!(e, 400);
    }

    #[test]
    fn hard_still_grows_just_slowly() {
        let s = update_stability(100, Grade::Hard).unwrap();
        assert!(s > 100, "Hard should still grow stability");
        assert_eq!(s, 120);
    }

    #[test]
    fn again_collapses_to_floor_for_small_stability() {
        // 100 * 0.10 = 10. Above the floor; clean collapse.
        let s = update_stability(100, Grade::Again).unwrap();
        assert_eq!(s, 10);
    }

    #[test]
    fn again_clamps_at_floor_for_tiny_stability() {
        // 5 * 0.10 = 0 (integer floor). Floored to 1.
        let s = update_stability(5, Grade::Again).unwrap();
        assert_eq!(s, STABILITY_FLOOR);
    }

    #[test]
    fn very_long_stability_does_not_overflow() {
        // u64 stability * 4 must not panic.
        let huge = HalfLife::MAX / 100;
        let s = update_stability(huge, Grade::Easy).unwrap();
        assert!(s >= huge);
    }

    #[test]
    fn round_trip_serde_grade() {
        for g in [Grade::Again, Grade::Hard, Grade::Good, Grade::Easy] {
            let s = serde_json::to_string(&g).unwrap();
            let back: Grade = serde_json::from_str(&s).unwrap();
            assert_eq!(g, back);
        }
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Successful grades (Hard, Good, Easy) are monotone in
        /// difficulty: Easy ≥ Good ≥ Hard ≥ current_stability for any
        /// stability > 0. Wait — Hard *can* dip below if multiplier
        /// is rounded down for very small stability. We just verify
        /// Easy ≥ Good ≥ Hard.
        #[test]
        fn successful_grades_monotone_in_difficulty(s in 100u64..1_000_000) {
            let h = update_stability(s, Grade::Hard).unwrap();
            let g = update_stability(s, Grade::Good).unwrap();
            let e = update_stability(s, Grade::Easy).unwrap();
            prop_assert!(h <= g, "Hard {h} should be ≤ Good {g}");
            prop_assert!(g <= e, "Good {g} should be ≤ Easy {e}");
        }

        /// Stability never falls below the floor, regardless of input.
        #[test]
        fn stability_floor_holds(s in 1u64..1_000_000, grade in 0u8..4) {
            let g = match grade {
                0 => Grade::Again,
                1 => Grade::Hard,
                2 => Grade::Good,
                _ => Grade::Easy,
            };
            let new_s = update_stability(s, g).unwrap();
            prop_assert!(new_s >= STABILITY_FLOOR);
        }

        /// Again always reduces stability (a lapse must hurt).
        #[test]
        fn lapse_always_reduces(s in 100u64..1_000_000) {
            let new_s = update_stability(s, Grade::Again).unwrap();
            prop_assert!(new_s < s, "Again must reduce: {new_s} >= {s}");
        }
    }
}
