//! Apply accrued demurrage by routing it through `EnergyRedirect::Demurrage`.
//!
//! This is the *only* place this crate touches state outside its own
//! computation. By going through the kernel's redirect type, the
//! conservation invariant
//! (`evaporchain_energy_kernel::ConservationCheck::redirect`) holds by
//! construction: the total energy across {Accounts, Stake, RefreshPool,
//! SlashedPool} is preserved exactly across the call.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_energy_kernel::{
    Compartment, EnergyAccumulator, EnergyRedirect, RedirectKind, RefreshPool,
};
use evaporchain_types::Energy;

use crate::accrual::demurrage_owed;
use crate::params::DemurrageParams;

/// What `apply_demurrage` produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyOutcome {
    /// Energy redirected from `Accounts` to `RefreshPool`.
    pub demurrage: Energy,
    /// Updated `last_touched_epoch` to write back into the account
    /// state — always the `current_epoch` argument, even when the owed
    /// amount was zero (so the account is "marked seen" to avoid
    /// repeated zero-credit recomputation).
    pub new_last_touched_epoch: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ApplyError {
    #[error("accumulator's Accounts compartment ({available}) is below the owed demurrage ({owed}) — chain state is inconsistent")]
    AccountsBelowOwed { available: Energy, owed: Energy },
    #[error("redirect failed: {0}")]
    RedirectFailed(String),
}

/// Compute and apply demurrage for one account.
///
/// `account_balance` is the per-account balance from which demurrage
/// is owed. `acc[Accounts]` is the chain-wide accumulator; the redirect
/// debits it by the owed amount and credits `acc[RefreshPool]`. The
/// per-namespace credit `pool` is also incremented under `namespace`.
///
/// Returns the [`ApplyOutcome`]. On error the kernel state is unchanged.
pub fn apply_demurrage(
    account_balance: Energy,
    last_touched_epoch: u64,
    current_epoch: u64,
    params: &DemurrageParams,
    acc: &mut EnergyAccumulator,
    pool: &mut RefreshPool,
    namespace: &[u8],
) -> Result<ApplyOutcome, ApplyError> {
    let owed = demurrage_owed(account_balance, last_touched_epoch, current_epoch, params);
    if owed == 0 {
        return Ok(ApplyOutcome {
            demurrage: 0,
            new_last_touched_epoch: current_epoch,
        });
    }
    if acc[Compartment::Accounts] < owed {
        return Err(ApplyError::AccountsBelowOwed {
            available: acc[Compartment::Accounts],
            owed,
        });
    }
    EnergyRedirect::new(RedirectKind::Demurrage, owed)
        .apply(acc)
        .map_err(|e| ApplyError::RedirectFailed(format!("{e}")))?;
    pool.accrue(namespace.to_vec(), owed, current_epoch);
    Ok(ApplyOutcome {
        demurrage: owed,
        new_last_touched_epoch: current_epoch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (EnergyAccumulator, RefreshPool) {
        // 10_000 in Accounts, nothing else.
        (EnergyAccumulator::new(10_000, 0, 0, 0), RefreshPool::new())
    }

    #[test]
    fn zero_owed_no_state_change_but_marks_seen() {
        let (mut acc, mut pool) = fresh();
        let pre = acc;
        let outcome = apply_demurrage(
            500, // below threshold
            10,
            20,
            &DemurrageParams::default(),
            &mut acc,
            &mut pool,
            b"alice",
        )
        .unwrap();
        assert_eq!(outcome.demurrage, 0);
        assert_eq!(outcome.new_last_touched_epoch, 20);
        assert_eq!(acc, pre);
        assert!(pool.is_empty());
    }

    #[test]
    fn nonzero_owed_redirects_into_refresh_pool() {
        let (mut acc, mut pool) = fresh();
        let pre_total = acc.total();
        // Aggressive params so we get a non-zero owed amount.
        let p = DemurrageParams::new(10_000, 1024);
        let outcome = apply_demurrage(8_000, 0, 1, &p, &mut acc, &mut pool, b"alice").unwrap();
        assert!(outcome.demurrage > 0);
        // Conservation: total preserved.
        assert_eq!(acc.total(), pre_total);
        // Compartments shifted as expected.
        assert_eq!(acc[Compartment::Accounts], 10_000 - outcome.demurrage);
        assert_eq!(acc[Compartment::RefreshPool], outcome.demurrage);
        // Pool ledger reflects the per-namespace accrual.
        assert_eq!(pool.total_accrued(), outcome.demurrage);
    }

    #[test]
    fn redirect_preserves_conservation_invariant() {
        use evaporchain_energy_kernel::ConservationCheck;
        let (mut acc, mut pool) = fresh();
        let before = acc;
        let p = DemurrageParams::new(1_000, 1024);
        let _ = apply_demurrage(5_000, 0, 100, &p, &mut acc, &mut pool, b"ns").unwrap();
        // Conservation check (no epochs elapsed in the kernel sense — a
        // redirect step on its own preserves total exactly).
        ConservationCheck::redirect(&before, &acc).expect("redirect must conserve total");
    }

    #[test]
    fn accounts_below_owed_returns_err_and_no_state_change() {
        // Construct a scenario where the accumulator's Accounts is
        // *less* than the per-account balance reports. This is a chain-
        // state inconsistency the kernel is meant to detect.
        let mut acc = EnergyAccumulator::new(1_000, 0, 0, 0);
        let pre = acc;
        let mut pool = RefreshPool::new();
        let p = DemurrageParams::new(10_000, 1024);
        // Per-account balance reports 50_000 but accumulator only holds 1_000.
        // The owed amount is computed against 50_000 → likely > 1_000.
        let err = apply_demurrage(50_000, 0, 100, &p, &mut acc, &mut pool, b"ns").unwrap_err();
        assert!(matches!(err, ApplyError::AccountsBelowOwed { .. }));
        assert_eq!(acc, pre);
        assert!(pool.is_empty());
    }
}
