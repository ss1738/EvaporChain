//! Singh-Boltzmann Stake integration — observability + opt-in
//! per-validator decay/refresh hooks against the chain's existing
//! StakeRecord ledger.
//!
//! Per `INVENTION_STACK.md` §4.1 #5: "Validator stake decays by
//! default; refresh by producing blocks. Kills the stake-and-lease-
//! key-to-MEV pattern."
//!
//! ## What ships here (substrate-level integration)
//!
//! - [`BoltzmannStakeRegistry`] — parallel map keyed on validator_id
//!   that tracks `last_touched_epoch` for each stake. The chain's
//!   existing `StakeRecord` doesn't carry this field; the registry
//!   sits beside the StateDB so we don't break the on-disk schema.
//! - [`decayed_voting_power`] — read-only computation: returns the
//!   `evaporchain_boltzmann_stake::decay_validator_stake` view of a
//!   validator's stake at `current_epoch`. Pure observability — does
//!   NOT mutate state. Caller can compare to the live `staked_amount`
//!   for monitoring or governance proposal modeling.
//! - [`apply_decay_to_all`] — opt-in mutator: walks the registry,
//!   applies `decay_validator_stake` to each, and writes the decayed
//!   amount back into the StateDB's `StakeRecord.staked_amount`.
//!   *Off by default* — the chain caller controls when this fires.
//! - [`refresh_on_block_production`] — opt-in mutator called by the
//!   chain when validator V proposes a block. Credits V's stake by
//!   `refresh_amount` and updates `last_touched_epoch`.

use std::collections::BTreeMap;

use evaporchain_boltzmann_stake::ValidatorStake;
use evaporchain_energy_kernel::ChainLambda;
use evaporchain_state::db::StateDB;
use evaporchain_types::Energy;

#[derive(Debug, Clone, Default)]
pub struct BoltzmannStakeRegistry {
    /// validator_id → last_touched_epoch. Initialised lazily; first
    /// observation seeds the entry from the chain's `staked_at_epoch`.
    pub last_touched: BTreeMap<u64, u64>,
}

impl BoltzmannStakeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up `validator_id`'s last-touched epoch, seeding from
    /// `default_epoch` if not yet present.
    pub fn get_or_seed(&mut self, validator_id: u64, default_epoch: u64) -> u64 {
        *self.last_touched.entry(validator_id).or_insert(default_epoch)
    }

    pub fn touch(&mut self, validator_id: u64, epoch: u64) {
        self.last_touched.insert(validator_id, epoch);
    }
}

/// Pure-observability view: what would `validator_id`'s stake be at
/// `current_epoch` under Singh-Boltzmann decay? Reads from the StateDB
/// + registry; does NOT mutate state.
pub fn decayed_voting_power(
    db: &dyn StateDB,
    registry: &BoltzmannStakeRegistry,
    validator_id: u64,
    chain_lambda: ChainLambda,
    current_epoch: u64,
) -> Option<Energy> {
    let stake = db.get_stake(validator_id)?;
    let last_touched = registry
        .last_touched
        .get(&validator_id)
        .copied()
        .unwrap_or(stake.staked_at_epoch);
    let live = stake.staked_amount.saturating_sub(stake.slashed_amount);
    let v = ValidatorStake::new(live, last_touched);
    Some(
        evaporchain_boltzmann_stake::decay_validator_stake(v, chain_lambda, current_epoch).active,
    )
}

/// Opt-in mutator: apply Boltzmann decay to every registered validator
/// in the registry, writing the decayed amount back into the StateDB.
/// Off by default — the chain caller controls when this fires
/// (e.g. governance amendment promotes from observability to enforcement).
pub fn apply_decay_to_all(
    db: &mut dyn StateDB,
    registry: &mut BoltzmannStakeRegistry,
    chain_lambda: ChainLambda,
    current_epoch: u64,
) -> usize {
    let mut count = 0;
    let stake_records: Vec<(u64, u64, u64, u64)> = db
        .all_stakes()
        .iter()
        .map(|s| (s.validator_id, s.staked_amount, s.slashed_amount, s.staked_at_epoch))
        .collect();
    for (validator_id, staked_amount, slashed_amount, staked_at_epoch) in stake_records {
        let last_touched = registry
            .last_touched
            .get(&validator_id)
            .copied()
            .unwrap_or(staked_at_epoch);
        if last_touched >= current_epoch {
            continue;
        }
        let live = staked_amount.saturating_sub(slashed_amount);
        let v = ValidatorStake::new(live, last_touched);
        let decayed = evaporchain_boltzmann_stake::decay_validator_stake(
            v,
            chain_lambda,
            current_epoch,
        );
        // Write decayed amount back into the StakeRecord.
        if let Some(mut s) = db.get_stake(validator_id).cloned() {
            // Decayed.active is the new live stake; preserve the
            // slashed_amount field so subsequent slashes remain
            // accountable (the doctrine doesn't say slashed energy
            // re-decays — it goes to the slashed pool).
            s.staked_amount = decayed.active.saturating_add(s.slashed_amount);
            db.put_stake(s);
            registry.touch(validator_id, current_epoch);
            count += 1;
        }
    }
    count
}

/// Opt-in mutator: credit `validator_id`'s stake by `refresh_amount`
/// (the doctrine's "refresh by producing blocks"). Updates
/// `last_touched_epoch` so subsequent decay accounts from now.
pub fn refresh_on_block_production(
    db: &mut dyn StateDB,
    registry: &mut BoltzmannStakeRegistry,
    validator_id: u64,
    refresh_amount: Energy,
    current_epoch: u64,
) -> bool {
    if let Some(mut s) = db.get_stake(validator_id).cloned() {
        s.staked_amount = s.staked_amount.saturating_add(refresh_amount);
        db.put_stake(s);
        registry.touch(validator_id, current_epoch);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;
    use evaporchain_state::db::InMemoryStateDB;
    use evaporchain_types::{AccountAddress, StakeRecord};

    fn addr(b: u8) -> AccountAddress {
        [b; 32]
    }

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    fn stake(id: u64, amount: u64, staked_at_epoch: u64) -> StakeRecord {
        StakeRecord {
            validator_id: id,
            validator_address: addr(id as u8),
            staked_amount: amount,
            staked_at_epoch,
            unbonding_epoch: None,
            slashed_amount: 0,
        }
    }

    #[test]
    fn observability_returns_decayed_view() {
        let mut db = InMemoryStateDB::new();
        db.put_stake(stake(1, 1000, 0));
        let registry = BoltzmannStakeRegistry::new();
        // After 100 epochs (one half-life): live=1000 → decayed=500.
        let decayed = decayed_voting_power(&db, &registry, 1, lambda(), 100).unwrap();
        assert_eq!(decayed, 500);
    }

    #[test]
    fn observability_does_not_mutate_state() {
        let mut db = InMemoryStateDB::new();
        db.put_stake(stake(1, 1000, 0));
        let registry = BoltzmannStakeRegistry::new();
        let _ = decayed_voting_power(&db, &registry, 1, lambda(), 100);
        // Stake unchanged in the DB.
        assert_eq!(db.get_stake(1).unwrap().staked_amount, 1000);
    }

    #[test]
    fn apply_decay_writes_back() {
        let mut db = InMemoryStateDB::new();
        db.put_stake(stake(1, 1000, 0));
        let mut registry = BoltzmannStakeRegistry::new();
        let n = apply_decay_to_all(&mut db, &mut registry, lambda(), 100);
        assert_eq!(n, 1);
        assert_eq!(db.get_stake(1).unwrap().staked_amount, 500);
        assert_eq!(*registry.last_touched.get(&1).unwrap(), 100);
    }

    #[test]
    fn refresh_credits_stake_and_touches_registry() {
        let mut db = InMemoryStateDB::new();
        db.put_stake(stake(1, 1000, 0));
        let mut registry = BoltzmannStakeRegistry::new();
        // Decay first → 500 at epoch 100.
        apply_decay_to_all(&mut db, &mut registry, lambda(), 100);
        // Refresh by 200 at epoch 100 → 700.
        let ok = refresh_on_block_production(&mut db, &mut registry, 1, 200, 100);
        assert!(ok);
        assert_eq!(db.get_stake(1).unwrap().staked_amount, 700);
        assert_eq!(*registry.last_touched.get(&1).unwrap(), 100);
    }

    #[test]
    fn refresh_unknown_validator_returns_false() {
        let mut db = InMemoryStateDB::new();
        let mut registry = BoltzmannStakeRegistry::new();
        let ok = refresh_on_block_production(&mut db, &mut registry, 99, 100, 0);
        assert!(!ok);
    }

    #[test]
    fn decay_preserves_slashed_amount() {
        let mut db = InMemoryStateDB::new();
        let mut s = stake(1, 1000, 0);
        s.slashed_amount = 200;
        db.put_stake(s);
        let mut registry = BoltzmannStakeRegistry::new();
        // Live stake = 1000 - 200 = 800; after one half-life → 400.
        // Re-add slashed → staked_amount = 400 + 200 = 600.
        apply_decay_to_all(&mut db, &mut registry, lambda(), 100);
        let after = db.get_stake(1).unwrap();
        assert_eq!(after.staked_amount, 600);
        assert_eq!(after.slashed_amount, 200);
    }
}
