//! Energy-kernel audit hooks for the execution layer.
//!
//! Bridges `evaporchain-state`'s `StateDB` view of accounts + stake
//! into `evaporchain-energy-kernel`'s 4-compartment conservation
//! domain. Lets any execution-layer caller take a snapshot of the
//! chain's current energy distribution and verify that a proposed
//! state transition is *conservation-preserving* (or at least
//! drops only by what the global λ allows).
//!
//! ## What `compartment_snapshot` populates
//!
//! - `Accounts`     ← Σ over `db.all_account_addresses()` of
//!                    `db.get_account(addr).balance`.
//! - `Stake`        ← Σ over `db.all_stakes()` of `r.staked_amount −
//!                    r.slashed_amount` (live stake only).
//! - `RefreshPool`  ← 0 (the kernel's `RefreshPool` ledger lives
//!                    outside the existing `StateDB`; consumers that
//!                    have already wired one in pass it through).
//! - `SlashedPool`  ← Σ over `db.all_stakes()` of `r.slashed_amount`
//!                    (held in the kernel's slashed-pool until a
//!                    `SlashSettle` redirect drains it into RefreshPool).
//!
//! ## Why this is the right first integration
//!
//! Single function, no new state, read-only over `StateDB`. Lets the
//! existing chain code adopt the kernel's invariant audit *without*
//! changing any semantics — every other primitive (demurrage, fee
//! controller, sanov-slashing, refresh market, …) wires through the
//! same `Compartment` surface once this snapshot exists.

use evaporchain_energy_kernel::{
    ChainLambda, Compartment, ConservationCheck, ConservationViolation, EnergyAccumulator,
};
use evaporchain_state::db::StateDB;

/// Build an `EnergyAccumulator` from the current `StateDB` view.
/// `RefreshPool` is left at 0; callers that maintain a kernel
/// `RefreshPool` ledger outside the StateDB pass that total in via
/// [`compartment_snapshot_with_pool`].
pub fn compartment_snapshot(db: &dyn StateDB) -> EnergyAccumulator {
    compartment_snapshot_with_pool(db, 0)
}

/// Same as [`compartment_snapshot`] but with an explicit refresh-
/// pool total (the kernel's per-namespace `RefreshPool::total_accrued`
/// is the natural source).
pub fn compartment_snapshot_with_pool(db: &dyn StateDB, refresh_pool_total: u64) -> EnergyAccumulator {
    let accounts: u64 = db
        .all_account_addresses()
        .into_iter()
        .filter_map(|addr| db.get_account(&addr).map(|a| a.balance))
        .fold(0u64, |a, b| a.saturating_add(b));

    let (stake, slashed): (u64, u64) =
        db.all_stakes().iter().fold((0u64, 0u64), |(s, sl), r| {
            let live = r.staked_amount.saturating_sub(r.slashed_amount);
            (s.saturating_add(live), sl.saturating_add(r.slashed_amount))
        });

    EnergyAccumulator::new(accounts, stake, refresh_pool_total, slashed)
}

/// Audit a proposed block transition: `before → after` over
/// `epochs_elapsed` epochs at the chain-global λ. Wraps
/// `ConservationCheck::block_step` for the execution-layer caller.
pub fn audit_block_step(
    before: &EnergyAccumulator,
    after: &EnergyAccumulator,
    epochs_elapsed: u64,
    chain_lambda: ChainLambda,
) -> Result<(), ConservationViolation> {
    ConservationCheck::block_step(before, after, epochs_elapsed, chain_lambda)
}

/// Convenience: total energy across the four compartments. Mirrors
/// `EnergyAccumulator::total()` but takes a `&dyn StateDB` directly.
pub fn db_total_energy(db: &dyn StateDB, refresh_pool_total: u64) -> u64 {
    compartment_snapshot_with_pool(db, refresh_pool_total).total()
}

/// Per-compartment value at the snapshot. Mostly useful for debug
/// dashboards and the chain-status API endpoint.
pub fn db_compartment(db: &dyn StateDB, c: Compartment, refresh_pool_total: u64) -> u64 {
    compartment_snapshot_with_pool(db, refresh_pool_total)[c]
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;
    use evaporchain_state::db::InMemoryStateDB;
    use evaporchain_types::{Account, AccountAddress, StakeRecord};

    fn addr(b: u8) -> AccountAddress {
        [b; 32]
    }

    fn stake(id: u64, amount: u64, slashed: u64) -> StakeRecord {
        StakeRecord {
            validator_id: id,
            validator_address: addr(id as u8),
            staked_amount: amount,
            staked_at_epoch: 0,
            unbonding_epoch: None,
            slashed_amount: slashed,
        }
    }

    fn db_with_accounts_and_stake() -> InMemoryStateDB {
        let mut db = InMemoryStateDB::new();
        // Two accounts.
        let a = db.get_or_create_account(&addr(1));
        a.balance = 1_000;
        let b = db.get_or_create_account(&addr(2));
        b.balance = 500;
        // Two stake records — one with a slash already deducted.
        db.put_stake(stake(10, 800, 0));
        db.put_stake(stake(11, 1_200, 200));
        db
    }

    #[test]
    fn snapshot_sums_accounts_and_stake_and_slashed() {
        let db = db_with_accounts_and_stake();
        let snap = compartment_snapshot(&db);
        assert_eq!(snap[Compartment::Accounts], 1_500);     // 1000 + 500
        // Live stake = 800 + (1200 - 200) = 1800.
        assert_eq!(snap[Compartment::Stake], 1_800);
        assert_eq!(snap[Compartment::RefreshPool], 0);
        // Slashed-but-not-settled = 200.
        assert_eq!(snap[Compartment::SlashedPool], 200);
        // Total = 1500 + 1800 + 0 + 200 = 3500.
        assert_eq!(snap.total(), 3_500);
    }

    #[test]
    fn snapshot_with_pool_threads_external_total() {
        let db = db_with_accounts_and_stake();
        let snap = compartment_snapshot_with_pool(&db, 9_999);
        assert_eq!(snap[Compartment::RefreshPool], 9_999);
        assert_eq!(snap.total(), 3_500 + 9_999);
    }

    #[test]
    fn audit_no_op_passes() {
        let db = db_with_accounts_and_stake();
        let snap = compartment_snapshot(&db);
        let lambda = ChainLambda::new(Lambda::from_epochs(100));
        // No state change, no time elapsed → trivially passes.
        audit_block_step(&snap, &snap, 0, lambda).unwrap();
    }

    #[test]
    fn audit_total_increase_rejected() {
        let db = db_with_accounts_and_stake();
        let before = compartment_snapshot(&db);
        // Hand-construct an "after" with strictly more energy.
        let mut after = before;
        after.credit(Compartment::Accounts, 1);
        let lambda = ChainLambda::new(Lambda::from_epochs(100));
        let err = audit_block_step(&before, &after, 0, lambda).unwrap_err();
        assert!(matches!(err, ConservationViolation::DecayIncreasedTotal { .. }));
    }

    #[test]
    fn audit_decay_below_lambda_floor_rejected() {
        let before = EnergyAccumulator::new(1_000, 0, 0, 0);
        // Drop below what 100 epochs of λ-decay allow.
        let after = EnergyAccumulator::new(400, 0, 0, 0);
        let lambda = ChainLambda::new(Lambda::from_epochs(100));
        let err = audit_block_step(&before, &after, 100, lambda).unwrap_err();
        assert!(matches!(err, ConservationViolation::DecayExceededLambda { .. }));
    }

    #[test]
    fn db_total_and_compartment_match_snapshot() {
        let db = db_with_accounts_and_stake();
        assert_eq!(db_total_energy(&db, 0), 3_500);
        assert_eq!(db_compartment(&db, Compartment::Accounts, 0), 1_500);
        assert_eq!(db_compartment(&db, Compartment::Stake, 0), 1_800);
    }
}
