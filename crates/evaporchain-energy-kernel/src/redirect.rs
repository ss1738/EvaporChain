//! Energy redirects — moves between compartments that *preserve* the total.
//!
//! Per INVENTION_STACK.md §1.2:
//!
//! ```text
//!    slashed stake ───┐
//!    MEV revenue  ────┼─► Refresh Pool ─► auto-refresh of namespace roots
//!    demurrage    ────┘                    (chain history, beacon, light-cone proofs)
//! ```
//!
//! Each redirect kind names a specific energy flow. The `apply` method on
//! a redirect is the *only* sanctioned way to move energy between
//! compartments outside of a λ-driven decay step — every other path is a
//! conservation violation by construction.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::compartment::{Compartment, EnergyAccumulator};
use evaporchain_types::Energy;

/// The kind of energy redirect — a finite enumeration so the conservation
/// checker can enumerate every legal flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RedirectKind {
    /// Validator slash → Slashed pool. Sanov-Slashing per INVENTION_STACK.md
    /// §A1.3 picks the magnitude; this redirect just moves the bytes.
    Slash,
    /// Slashed pool drained into the Refresh pool (the
    /// slash → refresh-pool half of the conservation triplet).
    SlashSettle,
    /// MEV revenue burnt into the Refresh pool. Ties to Crooks-MEV Refund
    /// (INVENTION_STACK.md §A1.3) and the Singh-Lyapunov fee market.
    MevBurn,
    /// Per-account demurrage — passive accrual on an idle balance per
    /// the piecewise-log rate of §4.1 primitive #6, redirected into the
    /// Refresh pool.
    Demurrage,
    /// Refresh pool pays a namespace root keep-alive — Refresh pool →
    /// Accounts of the namespace operator (the "auto-refresh" leg).
    RefreshPayout,
}

impl RedirectKind {
    /// The canonical (from, to) compartment pair this kind always uses.
    /// Hard-coded so a misuse can be caught at the type/value level rather
    /// than at audit time.
    pub const fn flow(self) -> (Compartment, Compartment) {
        match self {
            RedirectKind::Slash => (Compartment::Stake, Compartment::SlashedPool),
            RedirectKind::SlashSettle => (Compartment::SlashedPool, Compartment::RefreshPool),
            RedirectKind::MevBurn => (Compartment::Accounts, Compartment::RefreshPool),
            RedirectKind::Demurrage => (Compartment::Accounts, Compartment::RefreshPool),
            RedirectKind::RefreshPayout => (Compartment::RefreshPool, Compartment::Accounts),
        }
    }
}

/// A single redirect transaction. Constructed at the call site, then
/// `apply`'d to mutate an `EnergyAccumulator`. The accumulator's total is
/// preserved exactly when the redirect succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyRedirect {
    pub kind: RedirectKind,
    pub amount: Energy,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RedirectError {
    #[error("source compartment {0:?} has insufficient energy ({1} < {2})")]
    InsufficientSource(Compartment, Energy, Energy),
}

impl EnergyRedirect {
    pub const fn new(kind: RedirectKind, amount: Energy) -> Self {
        Self { kind, amount }
    }

    /// Apply the redirect to `acc`. Returns the (from, to) pair on success
    /// for any caller that needs to log/audit the actual flow.
    ///
    /// Total energy across compartments is *exactly* preserved on success.
    /// On failure, `acc` is unchanged.
    pub fn apply(
        &self,
        acc: &mut EnergyAccumulator,
    ) -> Result<(Compartment, Compartment), RedirectError> {
        let (from, to) = self.kind.flow();
        let avail = acc[from];
        if avail < self.amount {
            return Err(RedirectError::InsufficientSource(from, avail, self.amount));
        }
        // Debit before credit so a saturation in the destination cannot
        // mask a missing debit — debit can fail; credit can saturate but
        // we already checked overflow class is unreachable in practice.
        acc.debit(from, self.amount)
            .map_err(|()| RedirectError::InsufficientSource(from, avail, self.amount))?;
        acc.credit(to, self.amount);
        Ok((from, to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compartment::Compartment;

    fn fresh() -> EnergyAccumulator {
        EnergyAccumulator::new(1_000, 1_000, 0, 0)
    }

    #[test]
    fn slash_moves_stake_to_slashed_pool() {
        let mut acc = fresh();
        let r = EnergyRedirect::new(RedirectKind::Slash, 200);
        let pre = acc.total();
        r.apply(&mut acc).unwrap();
        assert_eq!(acc[Compartment::Stake], 800);
        assert_eq!(acc[Compartment::SlashedPool], 200);
        assert_eq!(acc.total(), pre, "total preserved exactly");
    }

    #[test]
    fn slash_settle_drains_into_refresh_pool() {
        let mut acc = EnergyAccumulator::new(0, 0, 0, 500);
        let pre = acc.total();
        EnergyRedirect::new(RedirectKind::SlashSettle, 500)
            .apply(&mut acc)
            .unwrap();
        assert_eq!(acc[Compartment::SlashedPool], 0);
        assert_eq!(acc[Compartment::RefreshPool], 500);
        assert_eq!(acc.total(), pre);
    }

    #[test]
    fn mev_burn_moves_accounts_to_refresh_pool() {
        let mut acc = EnergyAccumulator::new(1_000, 0, 0, 0);
        let pre = acc.total();
        EnergyRedirect::new(RedirectKind::MevBurn, 75)
            .apply(&mut acc)
            .unwrap();
        assert_eq!(acc[Compartment::Accounts], 925);
        assert_eq!(acc[Compartment::RefreshPool], 75);
        assert_eq!(acc.total(), pre);
    }

    #[test]
    fn demurrage_redirects_to_refresh_pool() {
        let mut acc = EnergyAccumulator::new(10_000, 0, 0, 0);
        let pre = acc.total();
        EnergyRedirect::new(RedirectKind::Demurrage, 12)
            .apply(&mut acc)
            .unwrap();
        assert_eq!(acc[Compartment::Accounts], 9_988);
        assert_eq!(acc[Compartment::RefreshPool], 12);
        assert_eq!(acc.total(), pre);
    }

    #[test]
    fn refresh_payout_credits_account_holder() {
        let mut acc = EnergyAccumulator::new(0, 0, 1_000, 0);
        let pre = acc.total();
        EnergyRedirect::new(RedirectKind::RefreshPayout, 250)
            .apply(&mut acc)
            .unwrap();
        assert_eq!(acc[Compartment::RefreshPool], 750);
        assert_eq!(acc[Compartment::Accounts], 250);
        assert_eq!(acc.total(), pre);
    }

    #[test]
    fn insufficient_source_rejected_and_acc_unchanged() {
        let mut acc = EnergyAccumulator::new(0, 50, 0, 0);
        let before = acc;
        let err = EnergyRedirect::new(RedirectKind::Slash, 100)
            .apply(&mut acc)
            .unwrap_err();
        assert_eq!(
            err,
            RedirectError::InsufficientSource(Compartment::Stake, 50, 100)
        );
        assert_eq!(acc, before, "accumulator must be untouched on rejection");
    }
}
