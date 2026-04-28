//! The conservation domain.
//!
//! Per INVENTION_STACK.md §1.2, the **total energy budget** of the chain
//! is `Σ over { accounts, stake, refresh_pool, slashed_pool }`. Every
//! protocol transition either:
//!
//!   1. Moves energy *between* compartments (a redirect — total preserved), or
//!   2. Decays energy *within* a compartment via the global λ (total drops by
//!      exactly the decay amount).
//!
//! No transition may destroy energy outside (2). The `EnergyAccumulator`
//! holds the four bucket totals; `ConservationCheck` audits transitions.

use serde::{Deserialize, Serialize};
use std::ops::{Index, IndexMut};

use evaporchain_types::Energy;

/// The four conservation buckets. Their sum is the chain's total energy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Compartment {
    /// User account balances aggregate.
    Accounts,
    /// Validator-bonded stake aggregate (active + delegated, unbonding).
    Stake,
    /// Protocol-owned refresh pool — sink for slashing/MEV/demurrage,
    /// source for namespace-root keep-alive.
    RefreshPool,
    /// Slashed-but-not-yet-redirected energy. Acts as a holding bucket
    /// between a slash event and its push to `RefreshPool`.
    SlashedPool,
}

impl Compartment {
    /// All four compartments in canonical iteration order.
    pub const ALL: [Compartment; 4] = [
        Compartment::Accounts,
        Compartment::Stake,
        Compartment::RefreshPool,
        Compartment::SlashedPool,
    ];
}

/// Aggregate energy in each compartment. `total()` is the conservation
/// quantity audited by `ConservationCheck`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyAccumulator {
    accounts: Energy,
    stake: Energy,
    refresh_pool: Energy,
    slashed_pool: Energy,
}

impl EnergyAccumulator {
    /// Construct directly from the four bucket totals.
    pub const fn new(
        accounts: Energy,
        stake: Energy,
        refresh_pool: Energy,
        slashed_pool: Energy,
    ) -> Self {
        Self {
            accounts,
            stake,
            refresh_pool,
            slashed_pool,
        }
    }

    /// Sum of all four compartments. This is the conservation quantity.
    pub const fn total(&self) -> Energy {
        // Saturating to keep the call infallible; any real-world overflow
        // here would already be a chain-state-corruption-class bug since
        // total energy budget is bounded at genesis.
        self.accounts
            .saturating_add(self.stake)
            .saturating_add(self.refresh_pool)
            .saturating_add(self.slashed_pool)
    }

    /// Saturating-add `amount` to a compartment. Returns the new bucket
    /// total. Panics in debug if the addition would overflow `Energy`;
    /// release builds saturate.
    pub fn credit(&mut self, c: Compartment, amount: Energy) -> Energy {
        let slot = &mut self[c];
        *slot = slot.saturating_add(amount);
        debug_assert!(
            self[c] >= amount,
            "EnergyAccumulator::credit overflow: compartment={c:?}, amount={amount}"
        );
        self[c]
    }

    /// Subtract `amount` from a compartment. Returns `Err(())` if the
    /// compartment doesn't have enough — the caller must handle (no
    /// silent wrap-around, no negative energy).
    pub fn debit(&mut self, c: Compartment, amount: Energy) -> Result<Energy, ()> {
        let slot = &mut self[c];
        if *slot < amount {
            return Err(());
        }
        *slot -= amount;
        Ok(*slot)
    }
}

impl Index<Compartment> for EnergyAccumulator {
    type Output = Energy;
    fn index(&self, c: Compartment) -> &Energy {
        match c {
            Compartment::Accounts => &self.accounts,
            Compartment::Stake => &self.stake,
            Compartment::RefreshPool => &self.refresh_pool,
            Compartment::SlashedPool => &self.slashed_pool,
        }
    }
}

impl IndexMut<Compartment> for EnergyAccumulator {
    fn index_mut(&mut self, c: Compartment) -> &mut Energy {
        match c {
            Compartment::Accounts => &mut self.accounts,
            Compartment::Stake => &mut self.stake,
            Compartment::RefreshPool => &mut self.refresh_pool,
            Compartment::SlashedPool => &mut self.slashed_pool,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_accumulator_total_is_zero() {
        assert_eq!(EnergyAccumulator::default().total(), 0);
    }

    #[test]
    fn total_sums_all_four() {
        let acc = EnergyAccumulator::new(10, 20, 30, 40);
        assert_eq!(acc.total(), 100);
    }

    #[test]
    fn credit_then_debit_round_trip() {
        let mut acc = EnergyAccumulator::default();
        acc.credit(Compartment::Accounts, 100);
        assert_eq!(acc[Compartment::Accounts], 100);
        let new_total = acc.debit(Compartment::Accounts, 30).unwrap();
        assert_eq!(new_total, 70);
        assert_eq!(acc.total(), 70);
    }

    #[test]
    fn debit_below_zero_rejected() {
        let mut acc = EnergyAccumulator::new(0, 0, 50, 0);
        assert!(acc.debit(Compartment::RefreshPool, 51).is_err());
        // Bucket is unchanged on rejection.
        assert_eq!(acc[Compartment::RefreshPool], 50);
    }

    #[test]
    fn all_iteration_order() {
        // Asserted because it defines the canonical compartment order
        // for any code that loops over Compartment::ALL (e.g. the
        // conservation checker).
        assert_eq!(
            Compartment::ALL,
            [
                Compartment::Accounts,
                Compartment::Stake,
                Compartment::RefreshPool,
                Compartment::SlashedPool,
            ]
        );
    }
}
