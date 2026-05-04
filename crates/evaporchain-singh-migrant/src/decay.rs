//! Acceleration-of-decay-with-rest model.
//!
//! Below the resting threshold, the token decays at its declared
//! half-life. Past the threshold, the half-life *halves* (decay 2×
//! faster). Past `2 × threshold`, the half-life *halves again*
//! (decay 4× faster).
//!
//! Crucially, the acceleration is computed from `rested_at_epoch` —
//! the epoch the token last moved to a *novel* wallet. The token's
//! initial mint counts as the first move, with `rested_at_epoch =
//! minted_at_epoch`. A non-novel transfer (already-visited wallet)
//! does NOT reset `rested_at_epoch`.
//!
//! The energy curve is computed by *integrating* over the piecewise
//! tier — but for V1 we use a simpler closed-form approximation:
//! evaluate `energy_at_epoch(initial, effective_half_life(rest), rest)`
//! using whichever tier `rest` lands in. This is conservative
//! (slightly overstates decay at tier boundaries) and validator-cheap.

use evaporchain_types::{energy_at_epoch, Energy, Epoch, HalfLife};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecayError {
    #[error("base_half_life must be > 0")]
    ZeroHalfLife,
    #[error("resting_threshold_epochs must be > 0")]
    ZeroThreshold,
}

/// Effective half-life given how long the token has been resting.
///
/// Tier 1 (rest < threshold): unchanged base.
/// Tier 2 (threshold ≤ rest < 2×threshold): base / 2.
/// Tier 3 (rest ≥ 2×threshold): base / 4.
///
/// Returns `Err` if either input is zero — those would be undefined.
pub fn effective_half_life(
    base_half_life: HalfLife,
    rest_epochs: u64,
    resting_threshold_epochs: u64,
) -> Result<HalfLife, DecayError> {
    if base_half_life == 0 {
        return Err(DecayError::ZeroHalfLife);
    }
    if resting_threshold_epochs == 0 {
        return Err(DecayError::ZeroThreshold);
    }
    let two_thr = resting_threshold_epochs.saturating_mul(2);
    if rest_epochs < resting_threshold_epochs {
        Ok(base_half_life)
    } else if rest_epochs < two_thr {
        // Half the half-life. Floor at 1 so we never produce a 0
        // half-life (which would crash energy_at_epoch downstream).
        Ok((base_half_life / 2).max(1))
    } else {
        Ok((base_half_life / 4).max(1))
    }
}

/// Current energy of the token at `epoch_now`, given the rest tracker
/// `rested_at_epoch`.
///
/// Conservative approximation: pick the tier that `rest = epoch_now -
/// rested_at_epoch` lands in, and apply `energy_at_epoch` against the
/// effective half-life for that tier over the *whole* rest period.
pub fn current_energy(
    initial: Energy,
    base_half_life: HalfLife,
    rested_at_epoch: Epoch,
    resting_threshold_epochs: u64,
    epoch_now: Epoch,
) -> Result<Energy, DecayError> {
    let rest = epoch_now.saturating_sub(rested_at_epoch);
    let h_eff =
        effective_half_life(base_half_life, rest, resting_threshold_epochs)?;
    Ok(energy_at_epoch(initial, h_eff, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_half_life() {
        assert_eq!(
            effective_half_life(0, 50, 100).unwrap_err(),
            DecayError::ZeroHalfLife
        );
    }

    #[test]
    fn rejects_zero_threshold() {
        assert_eq!(
            effective_half_life(1000, 50, 0).unwrap_err(),
            DecayError::ZeroThreshold
        );
    }

    #[test]
    fn tier_1_below_threshold_is_base() {
        // base=1000, threshold=100. rest=50 < 100 ⇒ unchanged.
        assert_eq!(effective_half_life(1000, 50, 100).unwrap(), 1000);
        assert_eq!(effective_half_life(1000, 99, 100).unwrap(), 1000);
    }

    #[test]
    fn tier_2_at_or_past_threshold_is_half() {
        assert_eq!(effective_half_life(1000, 100, 100).unwrap(), 500);
        assert_eq!(effective_half_life(1000, 199, 100).unwrap(), 500);
    }

    #[test]
    fn tier_3_at_or_past_double_threshold_is_quarter() {
        assert_eq!(effective_half_life(1000, 200, 100).unwrap(), 250);
        assert_eq!(effective_half_life(1000, 9999, 100).unwrap(), 250);
    }

    #[test]
    fn tier_floor_at_one() {
        // base=1, threshold=100. tier 2 would be 0; floor at 1.
        assert_eq!(effective_half_life(1, 150, 100).unwrap(), 1);
        // base=3, threshold=100. tier 3 would be 3/4 = 0; floor at 1.
        assert_eq!(effective_half_life(3, 250, 100).unwrap(), 1);
    }

    #[test]
    fn current_energy_at_mint_is_initial() {
        let e = current_energy(1000, 100, 50, 30, 50).unwrap();
        assert_eq!(e, 1000);
    }

    #[test]
    fn current_energy_decays_faster_when_resting_too_long() {
        // initial=1000, base_half_life=100, threshold=30.
        // Compare two scenarios at total rest = 30 epochs:
        // (a) rest fits in tier 1 (e.g. threshold=100, never crosses)
        // (b) rest crosses into tier 2 (threshold=30, hits boundary)
        let a = current_energy(1000, 100, 0, 100, 30).unwrap();
        let b = current_energy(1000, 100, 0, 30, 30).unwrap();
        // (b) should be lower than (a) because (b) is using h_eff=50,
        // not 100, over the full 30 epochs.
        assert!(b < a, "tier-2 must decay faster: a={a}, b={b}");
    }

    #[test]
    fn current_energy_clock_skew_returns_initial() {
        // epoch_now < rested_at_epoch. Defensive: rest=0 ⇒ initial.
        let e = current_energy(1000, 100, 50, 30, 0).unwrap();
        assert_eq!(e, 1000);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Higher rest tier ⇒ smaller (or equal) effective half-life.
        #[test]
        fn effective_half_life_monotone_decreasing_with_rest(
            base in 4u64..1_000_000,
            threshold in 1u64..1000,
            rest_a in 0u64..2_000,
            extra in 0u64..2_000,
        ) {
            let h_a = effective_half_life(base, rest_a, threshold).unwrap();
            let h_b =
                effective_half_life(base, rest_a.saturating_add(extra), threshold).unwrap();
            prop_assert!(h_b <= h_a);
        }
    }
}
