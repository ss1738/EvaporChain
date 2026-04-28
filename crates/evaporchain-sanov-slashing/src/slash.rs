//! `sanov_slash(stake, observed, honest)` — compute and apply the
//! Sanov slash through the energy-kernel's `RedirectKind::Slash`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_energy_kernel::{
    Compartment, EnergyAccumulator, EnergyRedirect, RedirectKind,
};
use evaporchain_types::Energy;

use crate::distribution::Distribution;
use crate::kl::{kl_millibits, KlError, KL_INFINITY};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SlashError {
    #[error("kl divergence error: {0}")]
    Kl(#[from] KlError),
    #[error("stake compartment ({available}) is below the slash amount ({slash})")]
    StakeBelowSlash { available: Energy, slash: Energy },
    #[error("redirect failed: {0}")]
    RedirectFailed(String),
}

/// Compute the Sanov slash magnitude in `Energy`.
///
/// `slash = stake × KL(observed ‖ honest) (millibits) / 1000` (so the
/// slash is in *bits of stake* — 1 bit → halve, 2 bits → quarter, etc.).
/// In practice the integer KL approximation gives a smooth ramp.
///
/// The result is always capped at `stake` (cannot slash more than the
/// validator holds).
pub fn sanov_slash(
    stake: Energy,
    observed: &Distribution,
    honest: &Distribution,
) -> Result<Energy, SlashError> {
    let kl_mb = kl_millibits(observed, honest)?;
    if kl_mb == 0 {
        return Ok(0);
    }
    if kl_mb == KL_INFINITY {
        return Ok(stake);
    }
    // slash = stake × kl_millibits / 1000, capped at stake. u128
    // intermediate to avoid overflow at extreme stake × kl.
    let scaled = (stake as u128).saturating_mul(kl_mb as u128) / 1_000u128;
    Ok(scaled.min(stake as u128) as u64)
}

/// Apply the slash to a kernel `EnergyAccumulator` via
/// `RedirectKind::Slash` (Stake → SlashedPool). Returns the actual
/// slashed amount on success.
///
/// Total energy across the four kernel compartments is preserved by
/// construction (the slash is a redirect, not a destruction — the
/// later `RedirectKind::SlashSettle` drains the slashed pool into the
/// refresh pool).
pub fn apply_slash(
    stake: Energy,
    observed: &Distribution,
    honest: &Distribution,
    acc: &mut EnergyAccumulator,
) -> Result<Energy, SlashError> {
    let slash = sanov_slash(stake, observed, honest)?;
    if slash == 0 {
        return Ok(0);
    }
    if acc[Compartment::Stake] < slash {
        return Err(SlashError::StakeBelowSlash {
            available: acc[Compartment::Stake],
            slash,
        });
    }
    EnergyRedirect::new(RedirectKind::Slash, slash)
        .apply(acc)
        .map_err(|e| SlashError::RedirectFailed(format!("{e}")))?;
    Ok(slash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distribution::FIXED_POINT_SCALE;
    use evaporchain_energy_kernel::ConservationCheck;

    fn fresh() -> EnergyAccumulator {
        EnergyAccumulator::new(0, 1_000_000, 0, 0)
    }

    #[test]
    fn observed_equal_to_honest_no_slash() {
        let honest = Distribution::new(vec![900_000, 100_000]).unwrap();
        let observed = Distribution::new(vec![900_000, 100_000]).unwrap();
        assert_eq!(sanov_slash(1_000_000, &observed, &honest).unwrap(), 0);
    }

    #[test]
    fn observed_strict_violation_slashes_stake() {
        // Honest: 99.9% produced, 0.1% missed.
        // Observed: 50/50 → strong divergence.
        let honest = Distribution::new(vec![999_000, 1_000]).unwrap();
        let observed = Distribution::new(vec![500_000, 500_000]).unwrap();
        let slash = sanov_slash(1_000_000, &observed, &honest).unwrap();
        assert!(slash > 0);
        assert!(slash <= 1_000_000);
    }

    #[test]
    fn impossible_event_full_slash() {
        // P_2 = 0 but Q_2 > 0 → KL = +∞ → slash = stake.
        let honest = Distribution::new(vec![FIXED_POINT_SCALE, 0]).unwrap();
        let observed = Distribution::new(vec![900_000, 100_000]).unwrap();
        assert_eq!(
            sanov_slash(500_000, &observed, &honest).unwrap(),
            500_000
        );
    }

    #[test]
    fn apply_slash_moves_stake_to_slashed_pool() {
        let mut acc = fresh();
        let pre_total = acc.total();
        let honest = Distribution::new(vec![999_000, 1_000]).unwrap();
        let observed = Distribution::new(vec![500_000, 500_000]).unwrap();
        let amt = apply_slash(1_000_000, &observed, &honest, &mut acc).unwrap();
        assert!(amt > 0);
        assert_eq!(acc[Compartment::Stake], 1_000_000 - amt);
        assert_eq!(acc[Compartment::SlashedPool], amt);
        assert_eq!(acc.total(), pre_total, "slash preserves total");
    }

    #[test]
    fn apply_slash_preserves_conservation_invariant() {
        let mut acc = fresh();
        let before = acc;
        let honest = Distribution::new(vec![999_000, 1_000]).unwrap();
        let observed = Distribution::new(vec![700_000, 300_000]).unwrap();
        let _ = apply_slash(800_000, &observed, &honest, &mut acc).unwrap();
        ConservationCheck::redirect(&before, &acc).expect("slash must conserve");
    }

    #[test]
    fn apply_slash_with_insufficient_stake_errs_and_state_unchanged() {
        let mut acc = EnergyAccumulator::new(0, 100, 0, 0);
        let pre = acc;
        // Force a near-full slash via impossible-event.
        let honest = Distribution::new(vec![FIXED_POINT_SCALE, 0]).unwrap();
        let observed = Distribution::new(vec![900_000, 100_000]).unwrap();
        // Stake-arg says 1_000_000 (e.g. validator's reported stake) but
        // accumulator only holds 100 — represents a chain-state inconsistency.
        let err = apply_slash(1_000_000, &observed, &honest, &mut acc).unwrap_err();
        assert!(matches!(err, SlashError::StakeBelowSlash { .. }));
        assert_eq!(acc, pre);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn arb_pmf2() -> impl Strategy<Value = Vec<u64>> {
        (1u64..FIXED_POINT_SCALE_INTERNAL).prop_map(|p| vec![p, FIXED_POINT_SCALE_INTERNAL - p])
    }
    const FIXED_POINT_SCALE_INTERNAL: u64 = crate::distribution::FIXED_POINT_SCALE;

    proptest! {
        /// Property: the slash never exceeds the stake.
        #[test]
        fn slash_never_exceeds_stake(
            stake in 0u64..1_000_000_000,
            obs in arb_pmf2(),
            hon in arb_pmf2(),
        ) {
            let observed = Distribution::new(obs).unwrap();
            let honest = Distribution::new(hon).unwrap();
            let slash = sanov_slash(stake, &observed, &honest).unwrap();
            prop_assert!(slash <= stake);
        }

        /// Property: identical distributions slash to zero.
        #[test]
        fn identical_distributions_zero_slash(
            stake in 0u64..1_000_000_000,
            pmf in arb_pmf2(),
        ) {
            let d = Distribution::new(pmf).unwrap();
            prop_assert_eq!(sanov_slash(stake, &d, &d).unwrap(), 0);
        }
    }
}
