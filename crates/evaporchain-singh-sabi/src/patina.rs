//! Non-zero-floor decay curve.
//!
//! Standard decay (`energy_at_epoch`) approaches zero. Patina decay
//! approaches `floor` — the "ruined-beautiful" asymptote. The closed
//! form is:
//!
//! ```text
//!     patina(t) = floor + (initial - floor) * 2^(-t / half_life)
//! ```
//!
//! Equivalently: the *decayable portion* is `initial - floor`; that
//! portion follows the chain's standard half-life rule; the `floor` is
//! preserved forever.
//!
//! Invariants:
//! - `patina(0) == initial` (just-minted)
//! - `patina(t)` is monotone non-increasing in `t`
//! - `patina(t) ≥ floor` for all `t ≥ 0`
//! - `lim_{t → ∞} patina(t) == floor`

use evaporchain_types::{energy_at_epoch, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PatinaError {
    #[error("initial energy must be > 0")]
    ZeroInitial,
    #[error("half_life must be > 0")]
    ZeroHalfLife,
    #[error("floor must be ≤ initial (got floor={floor}, initial={initial})")]
    FloorAboveInitial { floor: Energy, initial: Energy },
}

/// Per-token patina parameters set at mint and frozen forever after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatinaParams {
    /// Initial energy at mint.
    pub initial: Energy,
    /// Asymptotic floor — the "ruined-beautiful" energy that the token
    /// never decays below. Doctrine default ≈ 15% of initial.
    pub floor: Energy,
    /// Half-life applied to the *decayable portion* (`initial -
    /// floor`).
    pub half_life: HalfLife,
}

impl PatinaParams {
    pub fn new(initial: Energy, floor: Energy, half_life: HalfLife) -> Result<Self, PatinaError> {
        if initial == 0 {
            return Err(PatinaError::ZeroInitial);
        }
        if half_life == 0 {
            return Err(PatinaError::ZeroHalfLife);
        }
        if floor > initial {
            return Err(PatinaError::FloorAboveInitial { floor, initial });
        }
        Ok(Self {
            initial,
            floor,
            half_life,
        })
    }

    /// Doctrine convenience: `floor = 15% of initial`, integer-floor.
    pub fn ruined_beautiful(initial: Energy, half_life: HalfLife) -> Result<Self, PatinaError> {
        let floor = (initial as u128 * 15 / 100) as Energy;
        Self::new(initial, floor, half_life)
    }
}

/// Patina score at `epoch_now`, given the params and `minted_at_epoch`.
///
/// Returns `params.initial` if `epoch_now ≤ minted_at_epoch` (clock
/// skew defensive).
pub fn patina_score(params: &PatinaParams, minted_at_epoch: Epoch, epoch_now: Epoch) -> Energy {
    let elapsed = epoch_now.saturating_sub(minted_at_epoch);
    let decayable = params.initial - params.floor;
    let decayed_remainder = energy_at_epoch(decayable, params.half_life, elapsed);
    params.floor + decayed_remainder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_initial() {
        assert_eq!(
            PatinaParams::new(0, 0, 100).unwrap_err(),
            PatinaError::ZeroInitial
        );
    }

    #[test]
    fn rejects_zero_half_life() {
        assert_eq!(
            PatinaParams::new(1000, 100, 0).unwrap_err(),
            PatinaError::ZeroHalfLife
        );
    }

    #[test]
    fn rejects_floor_above_initial() {
        let err = PatinaParams::new(100, 200, 50).unwrap_err();
        assert!(matches!(err, PatinaError::FloorAboveInitial { .. }));
    }

    #[test]
    fn ruined_beautiful_uses_15pct_floor() {
        let p = PatinaParams::ruined_beautiful(1000, 100).unwrap();
        assert_eq!(p.floor, 150);
    }

    #[test]
    fn at_mint_score_equals_initial() {
        let p = PatinaParams::new(1000, 150, 100).unwrap();
        assert_eq!(patina_score(&p, 50, 50), 1000);
    }

    #[test]
    fn before_mint_clock_skew_returns_initial() {
        let p = PatinaParams::new(1000, 150, 100).unwrap();
        assert_eq!(patina_score(&p, 50, 0), 1000);
    }

    #[test]
    fn never_decays_below_floor() {
        let p = PatinaParams::new(1000, 150, 10).unwrap();
        // After many half-lives (let's say 1000 epochs = 100 half-lives),
        // decayable portion ≈ 0. Score ≈ floor.
        let score = patina_score(&p, 0, 1000);
        assert_eq!(score, 150);
    }

    #[test]
    fn one_half_life_drops_decayable_by_half() {
        // initial=1000, floor=200, decayable=800.
        // After 1 half-life: decayable_remainder ≈ 400. Score ≈ 600.
        let p = PatinaParams::new(1000, 200, 100).unwrap();
        let score = patina_score(&p, 0, 100);
        // Allow rounding tolerance — energy_at_epoch is exact-doubling.
        assert!((500..=700).contains(&score), "got {score}");
    }

    #[test]
    fn floor_equals_initial_makes_token_never_decay() {
        // Degenerate but valid: initial=floor=500. Decayable=0.
        let p = PatinaParams::new(500, 500, 50).unwrap();
        for t in [0, 100, 1_000_000] {
            assert_eq!(patina_score(&p, 0, t), 500);
        }
    }

    #[test]
    fn round_trip_serde() {
        let p = PatinaParams::ruined_beautiful(10_000, 365).unwrap();
        let s = serde_json::to_string(&p).unwrap();
        let back: PatinaParams = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Patina score is monotone non-increasing in time.
        #[test]
        fn monotone_in_time(
            initial in 100u64..1_000_000,
            floor_frac in 0u64..100,
            half_life in 1u64..10_000,
            minted_at in 0u64..1_000,
            t_a in 0u64..2_000_000,
            extra in 0u64..1_000_000,
        ) {
            let floor = (initial * floor_frac) / 100;
            let p = PatinaParams::new(initial, floor, half_life).unwrap();
            let s_a = patina_score(&p, minted_at, t_a);
            let s_b = patina_score(&p, minted_at, t_a.saturating_add(extra));
            prop_assert!(s_b <= s_a, "score went up: {} → {}", s_a, s_b);
        }

        /// Patina score is always in [floor, initial].
        #[test]
        fn score_in_bounds(
            initial in 100u64..1_000_000,
            floor_frac in 0u64..100,
            half_life in 1u64..10_000,
            minted_at in 0u64..1_000,
            now in 0u64..2_000_000,
        ) {
            let floor = (initial * floor_frac) / 100;
            let p = PatinaParams::new(initial, floor, half_life).unwrap();
            let s = patina_score(&p, minted_at, now);
            prop_assert!(s >= floor, "below floor: {} < {}", s, floor);
            prop_assert!(s <= initial, "above initial: {} > {}", s, initial);
        }
    }
}
