//! Energy-weighted validator set with deterministic leader rotation.
//!
//! Standard round-robin gives each validator equal turns. EvaporChain weights
//! leader selection by the validator's contribution to the network's
//! thermodynamic health — validators who process more evaporations and
//! maintain healthier state get more frequent turns.
//!
//! Leader selection:
//! ```text
//! weight_i = stake_i * (1.0 + health_score_i * 0.2)   // health bonus caps at 20%
//! weighted_index = hash(epoch || "leader") mod total_weight
//! iterate validators: accumulate weights until >= weighted_index
//! ```

use serde::{Deserialize, Serialize};

/// Parts-per-million denominator for ratio fields. Lane R.7: minimal
/// declaration so cross-crate references compile. Value = 1_000_000
/// ppm = 1.0; the full integer-PID validator-set refactor that
/// consumes this constant is deferred to a separate session.
pub const VS_PPM_DENOMINATOR: u64 = 1_000_000;

// ── Phase 3a/3b (2026-05-08): ValidatorInfo, ValidatorSet, leader-
// selection + slashing constants moved to `evaporchain-consensus-types`.
// Re-exported here so all existing callers
// (`evaporchain_consensus::validator_set::{ValidatorInfo, ValidatorSet}`)
// keep working unchanged. The single method that depends on
// `evaporchain-state` (`refresh_delegated_stakes`) was extracted to a
// free function below — see definition near line ~620 of this file.
pub use evaporchain_consensus_types::{
    ValidatorInfo, ValidatorSet, HEALTH_BONUS_CAP, HEALTH_DECAY_RATE,
    HEALTH_PER_EVAPORATION, MAX_HEALTH_SCORE, MIN_STAKE,
    SLASH_DOWNTIME_PCT, SLASH_EQUIVOCATION_PCT,
};

// ─────────────────────── ValidatorSet ─────────────────────────────────────
//
// Phase 3b (2026-05-08): the ValidatorSet struct + impl + Default impl
// moved to `evaporchain-consensus-types` (see top of this file). Only
// the method that depends on `evaporchain-state` —
// `refresh_delegated_stakes` — stays here as a free function, since
// inherent impls cannot span crates.

/// Refresh each validator's `delegated_stake` from the live
/// DelegationRecord set in StateDB. Should be called at the start
/// of every block production cycle so quorum checks within that
/// block use up-to-date voting power. Within-block delegations
/// take effect on the next block.
///
/// Originally an inherent method on `ValidatorSet`; extracted to a
/// free function 2026-05-08 because the type now lives in
/// `evaporchain-consensus-types` (which intentionally excludes the
/// `evaporchain-state` dep). Existing callers should migrate from
/// `set.refresh_delegated_stakes(db)` to
/// `validator_set::refresh_delegated_stakes(set, db)`.
pub fn refresh_delegated_stakes(
    set: &mut ValidatorSet,
    db: &dyn evaporchain_state::db::StateDB,
) {
    let mut totals: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
    for d in db.all_delegations() {
        *totals.entry(d.validator_id).or_insert(0) = totals
            .get(&d.validator_id)
            .copied()
            .unwrap_or(0)
            .saturating_add(d.amount);
    }
    for v_id in set.validators().iter().map(|v| v.id).collect::<Vec<_>>() {
        if let Some(v) = set.get_mut(v_id) {
            v.delegated_stake = totals.get(&v.id).copied().unwrap_or(0);
        }
    }
}


/// Apply a proportional slash to every delegation against `validator_id`
/// (P0 #4 Phase 5). Use after [`ValidatorSet::slash_equivocation`] or
/// [`ValidatorSet::slash_downtime`] so delegators share the loss
/// proportionally with the misbehaving validator.
///
/// `slash_pct` is the fraction of *each delegation's amount* that gets
/// removed (0.0..=1.0). The slashed amount is permanently destroyed —
/// it is not credited back to the delegator's balance.
///
/// Both `amount` (active) and `unbonding_amount` are slashed so
/// misbehaviour during an in-flight undelegate still costs the delegator.
/// Records that drop to zero in both fields are removed entirely so the
/// delegation map doesn't accumulate dead entries.
///
/// Returns the total amount slashed across all delegators. Returns 0 if
/// `slash_pct` is outside `(0.0, 1.0]` or no delegations exist.
pub fn slash_delegations_for_validator(
    db: &mut dyn evaporchain_state::db::StateDB,
    validator_id: u64,
    slash_pct: f64,
) -> u64 {
    if !(0.0..=1.0).contains(&slash_pct) || slash_pct == 0.0 {
        return 0;
    }
    let records: Vec<evaporchain_types::DelegationRecord> = db
        .delegations_for_validator(validator_id)
        .into_iter()
        .cloned()
        .collect();
    let mut total_slashed: u64 = 0;
    for mut r in records {
        let active_slash = (r.amount as f64 * slash_pct).round() as u64;
        let unbonding_slash = (r.unbonding_amount as f64 * slash_pct).round() as u64;
        r.amount = r.amount.saturating_sub(active_slash);
        r.unbonding_amount = r.unbonding_amount.saturating_sub(unbonding_slash);
        total_slashed = total_slashed
            .saturating_add(active_slash)
            .saturating_add(unbonding_slash);
        if r.amount == 0 && r.unbonding_amount == 0 {
            db.remove_delegation(&r.delegator, validator_id);
        } else {
            db.put_delegation(r);
        }
    }
    total_slashed
}

// ─────────────────────── Epoch Transition Manager ───────────────────────

/// Minimum number of active validators (safety floor).
const MIN_VALIDATORS: usize = 3;

/// Maximum fraction of validators that can change per epoch (1/3).
const MAX_CHURN_FRACTION: f64 = 0.33;

/// Epochs a new validator must wait before entering the active set.
const BONDING_PERIOD_EPOCHS: u64 = 2;

/// Epochs an exiting validator must wait before their stake unlocks.
const UNBONDING_PERIOD_EPOCHS: u64 = 4;

/// Blocks per epoch (used to detect epoch boundaries).
const EPOCH_LENGTH: u64 = 100;

/// A requested change to the validator set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidatorSetChange {
    /// A new validator wants to join.
    Join(ValidatorInfo),
    /// A validator wants to leave.
    Leave { validator_id: u64 },
    /// A validator's stake changed (on-chain staking tx).
    StakeUpdate { validator_id: u64, new_stake: u64 },
}

/// Result of applying epoch transitions.
#[derive(Debug, Default)]
pub struct EpochTransitionResult {
    /// Changes that were applied this epoch.
    pub applied: Vec<String>,
    /// Changes deferred to a future epoch (bonding/unbonding period).
    pub deferred: Vec<String>,
    /// Changes rejected (safety constraints).
    pub rejected: Vec<String>,
}

/// Pending join with bonding countdown.
#[derive(Debug, Clone)]
struct PendingJoin {
    info: ValidatorInfo,
    ready_at_epoch: u64,
}

/// Pending leave with unbonding countdown.
#[derive(Debug, Clone)]
struct PendingLeave {
    validator_id: u64,
    unlock_at_epoch: u64,
}

/// Manages validator set transitions at epoch boundaries.
///
/// Safety invariants:
/// - Validator set never drops below `MIN_VALIDATORS`
/// - At most `MAX_CHURN_FRACTION` of validators change per epoch
/// - Joins require a bonding period; leaves require an unbonding period
pub struct EpochTransitionManager {
    /// Queued changes waiting to be applied.
    pending_joins: Vec<PendingJoin>,
    pending_leaves: Vec<PendingLeave>,
    pending_stake_updates: Vec<(u64, u64)>, // (validator_id, new_stake)
    /// Current epoch (updated on each transition).
    current_epoch: u64,
}

impl EpochTransitionManager {
    pub fn new() -> Self {
        Self {
            pending_joins: Vec::new(),
            pending_leaves: Vec::new(),
            pending_stake_updates: Vec::new(),
            current_epoch: 0,
        }
    }

    /// Queue a validator set change. It will be processed at the next epoch boundary.
    pub fn queue_change(&mut self, change: ValidatorSetChange, current_epoch: u64) {
        match change {
            ValidatorSetChange::Join(info) => {
                self.pending_joins.push(PendingJoin {
                    info,
                    ready_at_epoch: current_epoch + BONDING_PERIOD_EPOCHS,
                });
            }
            ValidatorSetChange::Leave { validator_id } => {
                self.pending_leaves.push(PendingLeave {
                    validator_id,
                    unlock_at_epoch: current_epoch + UNBONDING_PERIOD_EPOCHS,
                });
            }
            ValidatorSetChange::StakeUpdate {
                validator_id,
                new_stake,
            } => {
                self.pending_stake_updates.push((validator_id, new_stake));
            }
        }
    }

    /// Returns true if the given block height is an epoch boundary.
    pub fn is_epoch_boundary(height: u64) -> bool {
        height > 0 && height.is_multiple_of(EPOCH_LENGTH)
    }

    /// Apply pending transitions to the validator set at an epoch boundary.
    ///
    /// Returns a summary of what was applied, deferred, and rejected.
    pub fn apply_epoch_transition(
        &mut self,
        validator_set: &mut ValidatorSet,
        epoch: u64,
    ) -> EpochTransitionResult {
        self.current_epoch = epoch;
        let mut result = EpochTransitionResult::default();

        let max_churn =
            ((validator_set.active_count() as f64) * MAX_CHURN_FRACTION).ceil() as usize;
        let max_churn = max_churn.max(1); // at least 1 change allowed
        let mut changes_this_epoch = 0usize;

        // 1. Apply stake updates first (no churn cost).
        let updates: Vec<_> = self.pending_stake_updates.drain(..).collect();
        for (vid, new_stake) in updates {
            if new_stake < MIN_STAKE {
                result.rejected.push(format!(
                    "StakeUpdate for validator {} rejected: {} < MIN_STAKE {}",
                    vid, new_stake, MIN_STAKE
                ));
                continue;
            }
            if let Some(v) = validator_set.get_mut(vid) {
                let old = v.stake;
                v.stake = new_stake;
                result.applied.push(format!(
                    "Validator {} stake updated: {} → {}",
                    vid, old, new_stake
                ));
            } else {
                result.rejected.push(format!(
                    "StakeUpdate for validator {} rejected: not found",
                    vid
                ));
            }
        }

        // 2. Process ready joins (bonding period elapsed).
        let (ready, not_ready): (Vec<_>, Vec<_>) = self
            .pending_joins
            .drain(..)
            .partition(|p| p.ready_at_epoch <= epoch);

        self.pending_joins = not_ready;
        for pj in &self.pending_joins {
            result.deferred.push(format!(
                "Join for validator {} deferred until epoch {}",
                pj.info.id, pj.ready_at_epoch
            ));
        }

        for pj in ready {
            if changes_this_epoch >= max_churn {
                // Re-queue with immediate readiness for next epoch.
                self.pending_joins.push(PendingJoin {
                    info: pj.info.clone(),
                    ready_at_epoch: epoch + 1,
                });
                result.deferred.push(format!(
                    "Join for validator {} deferred: max churn reached",
                    pj.info.id
                ));
                continue;
            }
            if validator_set
                .validators()
                .iter()
                .any(|v| v.id == pj.info.id)
            {
                result.rejected.push(format!(
                    "Join for validator {} rejected: already exists",
                    pj.info.id
                ));
                continue;
            }
            let vid = pj.info.id;
            validator_set.add_validator(pj.info);
            changes_this_epoch += 1;
            result.applied.push(format!("Validator {} joined", vid));
        }

        // 3. Process leaves (unbonding period — validator removed immediately,
        //    but stake is locked until unlock_at_epoch).
        let (ready_leaves, not_ready_leaves): (Vec<_>, Vec<_>) = self
            .pending_leaves
            .drain(..)
            .partition(|p| p.unlock_at_epoch <= epoch);

        self.pending_leaves = not_ready_leaves;
        for pl in &self.pending_leaves {
            result.deferred.push(format!(
                "Leave for validator {} deferred: unbonding until epoch {}",
                pl.validator_id, pl.unlock_at_epoch
            ));
        }

        for pl in ready_leaves {
            if changes_this_epoch >= max_churn {
                self.pending_leaves.push(PendingLeave {
                    validator_id: pl.validator_id,
                    unlock_at_epoch: epoch + 1,
                });
                result.deferred.push(format!(
                    "Leave for validator {} deferred: max churn reached",
                    pl.validator_id
                ));
                continue;
            }
            // Safety: don't drop below minimum
            if validator_set.active_count() <= MIN_VALIDATORS {
                result.rejected.push(format!(
                    "Leave for validator {} rejected: would drop below MIN_VALIDATORS ({})",
                    pl.validator_id, MIN_VALIDATORS
                ));
                continue;
            }
            if validator_set.remove_validator(pl.validator_id) {
                changes_this_epoch += 1;
                result
                    .applied
                    .push(format!("Validator {} left", pl.validator_id));
            } else {
                result.rejected.push(format!(
                    "Leave for validator {} rejected: not found",
                    pl.validator_id
                ));
            }
        }

        result
    }

    /// Number of pending changes (joins + leaves + stake updates).
    pub fn pending_count(&self) -> usize {
        self.pending_joins.len() + self.pending_leaves.len() + self.pending_stake_updates.len()
    }
}

impl Default for EpochTransitionManager {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────── ValidatorSetSource seam ────────────────────────
//
// Lane G.5 — the final Layer 3 trait per DOCTRINE_PUNCH_LIST.md.
// `TendermintConsensus` holds a concrete `ValidatorSet` field with a
// large API surface (lookup, leader selection, stake bookkeeping, BLS
// key management, health scoring, queued changes). Defining a trait that
// wraps the entire surface would be overreach — alternative impls
// (Singh-Boltzmann stake variants, Self-Annealing validator sets) will
// replace the bookkeeping wholesale, not slot in piecewise.
//
// What the alternative impls genuinely need to plug in *to* is the
// **read-only consensus-decision surface**: leader selection,
// active-count snapshots, total-stake reads. The mutation pipeline
// (joins, leaves, stake updates) stays concrete because it's
// alt-impl-specific stateful bookkeeping. This trait names the
// minimum read-only contract that consensus depends on.
//
// Object-safe: every method returns owned values or borrows of types
// concrete to this crate (`ValidatorInfo`).

/// Read-only lookup contract for validator sets. Alternative
/// implementations (Singh-Boltzmann stake-based, Self-Annealing
/// thermodynamic validator sets) implement this trait against their own
/// internal storage; consensus calls only these methods on the read
/// path.
///
/// `Send + Sync` so consensus engines can hold the source behind locks.
pub trait ValidatorSetSource: Send + Sync {
    /// Total number of registered validators (active + inactive).
    fn len(&self) -> usize;

    /// True iff the source is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Count of validators currently considered "active" by the source's
    /// health/liveness rule.
    fn active_count(&self) -> usize;

    /// Sum of effective stake across all validators. The "effective"
    /// modifier is impl-specific: linear stake for the default impl,
    /// Boltzmann-weighted under Singh-Boltzmann.
    fn total_stake(&self) -> u64;

    /// Lookup a validator by ID. `None` if not registered.
    fn get(&self, id: u64) -> Option<&ValidatorInfo>;

    /// Leader for the given epoch, if any. `None` for an empty set or
    /// when the source's leader-selection rule rejects all candidates.
    fn leader_for_epoch(&self, epoch: u64) -> Option<&ValidatorInfo>;
}

/// Lane G.5 blanket impl. Pure delegation to `ValidatorSet`'s inherent
/// methods — no behaviour change.
impl ValidatorSetSource for ValidatorSet {
    fn len(&self) -> usize {
        ValidatorSet::len(self)
    }

    fn is_empty(&self) -> bool {
        ValidatorSet::is_empty(self)
    }

    fn active_count(&self) -> usize {
        ValidatorSet::active_count(self)
    }

    fn total_stake(&self) -> u64 {
        ValidatorSet::total_stake(self)
    }

    fn get(&self, id: u64) -> Option<&ValidatorInfo> {
        ValidatorSet::get(self, id)
    }

    fn leader_for_epoch(&self, epoch: u64) -> Option<&ValidatorInfo> {
        ValidatorSet::leader_for_epoch(self, epoch)
    }
}

// ─────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_validator(id: u64, stake: u64) -> ValidatorInfo {
        let mut addr = [0u8; 32];
        addr[0] = id as u8;
        ValidatorInfo::new(id, stake, addr)
    }

    fn make_validator_set(count: u64, stake: u64) -> ValidatorSet {
        let validators: Vec<_> = (1..=count).map(|id| make_validator(id, stake)).collect();
        ValidatorSet::with_validators(validators)
    }

    #[test]
    fn test_leader_selection_deterministic() {
        let vs = make_validator_set(4, 1000);

        let leader1 = vs.leader_for_epoch(10).unwrap().id;
        let leader2 = vs.leader_for_epoch(10).unwrap().id;
        assert_eq!(leader1, leader2, "Same epoch must yield same leader");
    }

    #[test]
    fn test_different_epochs_different_leaders() {
        let vs = make_validator_set(4, 1000);

        // Over 100 epochs, we should see at least 2 different leaders
        let mut leaders: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for epoch in 1..=100 {
            leaders.insert(vs.leader_for_epoch(epoch).unwrap().id);
        }
        assert!(
            leaders.len() >= 2,
            "Expected rotation across epochs, got {} unique leaders",
            leaders.len()
        );
    }

    #[test]
    fn test_all_validators_get_turns() {
        let vs = make_validator_set(4, 1000);

        let mut counts = std::collections::HashMap::new();
        for epoch in 1..=1000 {
            let leader = vs.leader_for_epoch(epoch).unwrap().id;
            *counts.entry(leader).or_insert(0u64) += 1;
        }

        // All 4 validators should get at least some turns
        assert_eq!(
            counts.len(),
            4,
            "All validators should participate: {:?}",
            counts
        );

        // With equal stake, each should get roughly 25% (allow 10-40% range)
        for (&id, &count) in &counts {
            assert!(
                count >= 100 && count <= 400,
                "Validator {} got {} turns out of 1000 (expected ~250)",
                id,
                count
            );
        }
    }

    #[test]
    fn test_higher_stake_more_turns() {
        let validators = vec![
            make_validator(1, 3000), // 3x stake
            make_validator(2, 1000), // 1x stake
        ];
        let vs = ValidatorSet::with_validators(validators);

        let mut counts = [0u64; 2];
        for epoch in 1..=10000 {
            let leader = vs.leader_for_epoch(epoch).unwrap().id;
            counts[(leader - 1) as usize] += 1;
        }

        // Validator 1 (3x stake) should get roughly 3x more turns than validator 2
        let ratio = counts[0] as f64 / counts[1] as f64;
        assert!(
            ratio > 2.0 && ratio < 4.5,
            "Expected ~3x ratio, got {:.2} (counts: {:?})",
            ratio,
            counts
        );
    }

    #[test]
    fn test_health_score_does_not_affect_leader_selection() {
        // Leader selection uses base stake, not health-weighted effective_weight,
        // to prevent inter-node divergence when health scores lag.
        let mut vs =
            ValidatorSet::with_validators(vec![make_validator(1, 1000), make_validator(2, 1000)]);

        let mut base_counts = [0u64; 2];
        for epoch in 1..=5000 {
            let leader = vs.leader_for_epoch(epoch).unwrap().id;
            base_counts[(leader - 1) as usize] += 1;
        }

        // Give validator 1 max health score — should NOT change leader turns
        vs.get_mut(1).unwrap().health_score = 1.0;

        let mut bonus_counts = [0u64; 2];
        for epoch in 1..=5000 {
            let leader = vs.leader_for_epoch(epoch).unwrap().id;
            bonus_counts[(leader - 1) as usize] += 1;
        }

        assert_eq!(
            base_counts, bonus_counts,
            "Health bonus must not affect leader selection (determinism requirement)"
        );
    }

    #[test]
    fn test_effective_weight() {
        let mut v = make_validator(1, 1000);
        assert_eq!(v.effective_weight(), 1000); // no health bonus

        v.health_score = 0.5;
        // 1000 * (1.0 + 0.5 * 0.2) = 1000 * 1.1 = 1100
        assert_eq!(v.effective_weight(), 1100);

        v.health_score = 1.0;
        // 1000 * (1.0 + 1.0 * 0.2) = 1000 * 1.2 = 1200
        assert_eq!(v.effective_weight(), 1200);

        // Health score above 1.0 is capped
        v.health_score = 5.0;
        assert_eq!(v.effective_weight(), 1200);
    }

    #[test]
    fn test_add_remove_validator() {
        let mut vs = ValidatorSet::new();
        assert!(vs.is_empty());

        vs.add_validator(make_validator(1, 1000));
        vs.add_validator(make_validator(2, 2000));
        assert_eq!(vs.len(), 2);

        // No duplicates
        vs.add_validator(make_validator(1, 9999));
        assert_eq!(vs.len(), 2);

        assert!(vs.remove_validator(1));
        assert_eq!(vs.len(), 1);
        assert!(!vs.remove_validator(1)); // already removed
        assert!(vs.get(1).is_none());
        assert!(vs.get(2).is_some());
    }

    #[test]
    fn test_validator_add_remove_changes_rotation() {
        let mut vs = make_validator_set(3, 1000);

        // Record leaders for epochs 1-100
        let leaders_before: Vec<u64> = (1..=100)
            .map(|e| vs.leader_for_epoch(e).unwrap().id)
            .collect();

        // Add a 4th validator
        vs.add_validator(make_validator(4, 1000));

        let leaders_after: Vec<u64> = (1..=100)
            .map(|e| vs.leader_for_epoch(e).unwrap().id)
            .collect();

        // At least some epochs should now select differently
        let changed = leaders_before
            .iter()
            .zip(leaders_after.iter())
            .filter(|(a, b)| a != b)
            .count();

        assert!(
            changed > 0,
            "Adding a validator should change at least some leader assignments"
        );
    }

    #[test]
    fn test_update_health_score() {
        let mut vs = make_validator_set(2, 1000);

        vs.update_health_score(1, 10);
        let v1 = vs.get(1).unwrap();
        assert_eq!(v1.blocks_produced, 1);
        assert_eq!(v1.evaporations_processed, 10);
        assert!((v1.health_score - 0.5).abs() < 0.001); // 10 * 0.05 = 0.5

        // Another block with 12 evaporations → 0.5 + 0.6 = 1.0 (capped)
        vs.update_health_score(1, 12);
        let v1 = vs.get(1).unwrap();
        assert_eq!(v1.blocks_produced, 2);
        assert_eq!(v1.evaporations_processed, 22);
        assert!((v1.health_score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_health_decay() {
        let mut vs = make_validator_set(2, 1000);
        vs.get_mut(1).unwrap().health_score = 0.5;

        vs.decay_health_scores();
        assert!((vs.get(1).unwrap().health_score - 0.49).abs() < 0.001);

        // Decay to zero
        for _ in 0..100 {
            vs.decay_health_scores();
        }
        assert_eq!(vs.get(1).unwrap().health_score, 0.0);
    }

    #[test]
    fn test_empty_validator_set_returns_none() {
        let vs = ValidatorSet::new();
        assert!(vs.leader_for_epoch(1).is_none());
        assert!(!vs.is_leader(1, 1));
    }

    #[test]
    fn test_is_leader() {
        let vs = make_validator_set(4, 1000);
        let leader_id = vs.leader_for_epoch(42).unwrap().id;

        assert!(vs.is_leader(leader_id, 42));

        // At least one other validator should NOT be the leader for this epoch
        let non_leaders: Vec<u64> = (1..=4).filter(|&id| id != leader_id).collect();
        for &id in &non_leaders {
            assert!(!vs.is_leader(id, 42));
        }
    }

    #[test]
    fn test_four_validator_simulation() {
        let vs = make_validator_set(4, 1000);

        let mut counts = std::collections::HashMap::new();
        for epoch in 1..=100 {
            let leader = vs.leader_for_epoch(epoch).unwrap().id;
            *counts.entry(leader).or_insert(0u64) += 1;
        }

        // Each validator should produce roughly 25% of blocks
        for id in 1..=4u64 {
            let count = counts.get(&id).copied().unwrap_or(0);
            assert!(
                count >= 10 && count <= 50,
                "Validator {} produced {} blocks out of 100 (expected ~25)",
                id,
                count
            );
        }
    }

    // ─── Slashing Tests ───────────────────────────────────────────────

    #[test]
    fn test_slash_equivocation() {
        let mut vs = make_validator_set(4, 1000);
        let slashed = vs.slash_equivocation(1);
        assert_eq!(slashed, 100); // 10% of 1000
        let v = vs.get(1).unwrap();
        assert_eq!(v.stake, 900);
        assert!(v.jailed);
        assert_eq!(v.health_score, 0.0);
        assert_eq!(v.total_slashed, 100);
    }

    #[test]
    fn test_slash_equivocation_jails_if_below_min() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 50, [1u8; 32])); // Below MIN_STAKE after slash
        let slashed = vs.slash_equivocation(1);
        assert_eq!(slashed, 5); // 10% of 50
        // Validator stays in set but is jailed; removal is governance-only.
        let v = vs.get(1).expect("validator must not be removed by slashing");
        assert!(v.jailed);
        assert_eq!(v.stake, 45);
    }

    #[test]
    fn test_slash_downtime() {
        let mut vs = make_validator_set(4, 1000);
        let slashed = vs.slash_downtime(2, 2);
        assert_eq!(slashed, 20); // 1% * 2 missed = 20
        let v = vs.get(2).unwrap();
        assert_eq!(v.stake, 980);
        assert!(!v.jailed); // Only 2 misses, jail at 3+
    }

    #[test]
    fn test_slash_downtime_jails_at_three() {
        let mut vs = make_validator_set(4, 1000);
        let slashed = vs.slash_downtime(3, 3);
        assert_eq!(slashed, 30);
        let v = vs.get(3).unwrap();
        assert!(v.jailed); // 3+ misses = jailed
    }

    #[test]
    fn test_jailed_validator_excluded_from_leader_rotation() {
        let mut vs = make_validator_set(4, 1000);
        // Jail validator 1
        vs.slash_equivocation(1);

        // Over 100 epochs, validator 1 should never be leader
        for epoch in 1..=100 {
            let leader = vs.leader_for_epoch(epoch).unwrap();
            assert_ne!(
                leader.id, 1,
                "Jailed validator 1 should not be leader at epoch {}",
                epoch
            );
        }
    }

    #[test]
    fn test_unjail() {
        let mut vs = make_validator_set(4, 1000);
        vs.slash_equivocation(1);
        assert!(vs.get(1).unwrap().jailed);

        assert!(vs.unjail(1));
        assert!(!vs.get(1).unwrap().jailed);

        // Now validator 1 can be leader again
        let can_lead = (1..=100).any(|e| vs.leader_for_epoch(e).unwrap().id == 1);
        assert!(
            can_lead,
            "Unjailed validator should participate in rotation"
        );
    }

    #[test]
    fn test_active_count() {
        let mut vs = make_validator_set(4, 1000);
        assert_eq!(vs.active_count(), 4);
        vs.slash_equivocation(1);
        assert_eq!(vs.active_count(), 3);
        vs.unjail(1);
        assert_eq!(vs.active_count(), 4);
    }

    #[test]
    fn test_total_weight_excludes_jailed() {
        let mut vs = make_validator_set(4, 1000);
        let total_before = vs.total_weight();
        vs.slash_equivocation(1);
        let total_after = vs.total_weight();
        assert!(
            total_after < total_before,
            "Jailed validator weight should be excluded"
        );
    }

    // ─── Epoch Transition Manager Tests ─────────────────────────────

    #[test]
    fn test_epoch_boundary_detection() {
        assert!(!EpochTransitionManager::is_epoch_boundary(0));
        assert!(!EpochTransitionManager::is_epoch_boundary(50));
        assert!(EpochTransitionManager::is_epoch_boundary(100));
        assert!(EpochTransitionManager::is_epoch_boundary(200));
        assert!(!EpochTransitionManager::is_epoch_boundary(101));
    }

    #[test]
    fn test_join_with_bonding_period() {
        let mut vs = make_validator_set(4, 1000);
        let mut mgr = EpochTransitionManager::new();

        // Queue join at epoch 5
        mgr.queue_change(ValidatorSetChange::Join(make_validator(5, 1000)), 5);
        assert_eq!(mgr.pending_count(), 1);

        // Epoch 6: bonding not elapsed (needs epoch 7)
        let result = mgr.apply_epoch_transition(&mut vs, 6);
        assert_eq!(vs.len(), 4, "Should not join yet");
        assert_eq!(result.deferred.len(), 1);

        // Epoch 7: bonding elapsed
        let result = mgr.apply_epoch_transition(&mut vs, 7);
        assert_eq!(vs.len(), 5);
        assert_eq!(result.applied.len(), 1);
        assert!(result.applied[0].contains("joined"));
    }

    #[test]
    fn test_leave_with_unbonding_period() {
        let mut vs = make_validator_set(5, 1000);
        let mut mgr = EpochTransitionManager::new();

        // Queue leave at epoch 10
        mgr.queue_change(ValidatorSetChange::Leave { validator_id: 5 }, 10);

        // Epoch 13: not yet (needs 14)
        let result = mgr.apply_epoch_transition(&mut vs, 13);
        assert_eq!(vs.len(), 5);
        assert_eq!(result.deferred.len(), 1);

        // Epoch 14: unbonding elapsed
        let result = mgr.apply_epoch_transition(&mut vs, 14);
        assert_eq!(vs.len(), 4);
        assert!(result.applied[0].contains("left"));
    }

    #[test]
    fn test_leave_rejected_below_minimum() {
        let mut vs = make_validator_set(3, 1000); // exactly MIN_VALIDATORS
        let mut mgr = EpochTransitionManager::new();

        mgr.queue_change(ValidatorSetChange::Leave { validator_id: 1 }, 0);

        let result = mgr.apply_epoch_transition(&mut vs, 10);
        assert_eq!(vs.len(), 3, "Should not drop below MIN_VALIDATORS");
        assert_eq!(result.rejected.len(), 1);
        assert!(result.rejected[0].contains("MIN_VALIDATORS"));
    }

    #[test]
    fn test_stake_update() {
        let mut vs = make_validator_set(4, 1000);
        let mut mgr = EpochTransitionManager::new();

        mgr.queue_change(
            ValidatorSetChange::StakeUpdate {
                validator_id: 2,
                new_stake: 5000,
            },
            0,
        );

        let result = mgr.apply_epoch_transition(&mut vs, 1);
        assert_eq!(vs.get(2).unwrap().stake, 5000);
        assert_eq!(result.applied.len(), 1);
    }

    #[test]
    fn test_stake_update_below_min_rejected() {
        let mut vs = make_validator_set(4, 1000);
        let mut mgr = EpochTransitionManager::new();

        mgr.queue_change(
            ValidatorSetChange::StakeUpdate {
                validator_id: 1,
                new_stake: 10, // below MIN_STAKE
            },
            0,
        );

        let result = mgr.apply_epoch_transition(&mut vs, 1);
        assert_eq!(vs.get(1).unwrap().stake, 1000, "Stake should be unchanged");
        assert_eq!(result.rejected.len(), 1);
    }

    #[test]
    fn test_max_churn_limit() {
        let mut vs = make_validator_set(4, 1000);
        let mut mgr = EpochTransitionManager::new();

        // Queue 3 joins at epoch 0 — max churn for 4 validators is ceil(4*0.33)=2
        for id in 5..=7 {
            mgr.queue_change(ValidatorSetChange::Join(make_validator(id, 1000)), 0);
        }

        let result = mgr.apply_epoch_transition(&mut vs, 2);
        // Should apply at most 2 joins, defer the rest
        assert!(vs.len() <= 6, "Max churn should limit joins");
        assert!(
            !result.deferred.is_empty(),
            "Excess joins should be deferred"
        );
    }

    #[test]
    fn test_duplicate_join_rejected() {
        let mut vs = make_validator_set(4, 1000);
        let mut mgr = EpochTransitionManager::new();

        // Try to join with id=1 which already exists
        mgr.queue_change(ValidatorSetChange::Join(make_validator(1, 2000)), 0);

        let result = mgr.apply_epoch_transition(&mut vs, 2);
        assert_eq!(vs.len(), 4);
        assert_eq!(result.rejected.len(), 1);
        assert!(result.rejected[0].contains("already exists"));
    }

    #[test]
    fn test_combined_transitions() {
        let mut vs = make_validator_set(5, 1000);
        let mut mgr = EpochTransitionManager::new();

        // Simultaneously: join 6, leave 5, update stake of 1
        mgr.queue_change(ValidatorSetChange::Join(make_validator(6, 1500)), 0);
        mgr.queue_change(ValidatorSetChange::Leave { validator_id: 5 }, 0);
        mgr.queue_change(
            ValidatorSetChange::StakeUpdate {
                validator_id: 1,
                new_stake: 3000,
            },
            0,
        );

        // Epoch 4: leave unbonding done, join bonding done at epoch 2
        let result = mgr.apply_epoch_transition(&mut vs, 4);

        // Stake update should apply
        assert_eq!(vs.get(1).unwrap().stake, 3000);
        // Join and leave depend on churn limits, but both should be processable
        assert!(
            result.applied.len() >= 2,
            "Stake update + at least one more: {:?}",
            result
        );
    }

    // ─── P0 #4 Phase 5 + 6: Delegation slashing & voting-power roll-up ──

    use evaporchain_state::db::{InMemoryStateDB, StateDB};
    use evaporchain_types::DelegationRecord;

    fn delegation(delegator_byte: u8, validator_id: u64, amount: u64) -> DelegationRecord {
        let mut addr = [0u8; 32];
        addr[0] = delegator_byte;
        DelegationRecord {
            delegator: addr,
            validator_id,
            amount,
            delegated_at_epoch: 0,
            unbonding_amount: 0,
            unbonding_epoch: None,
        }
    }

    #[test]
    fn test_refresh_delegated_stakes_sums_per_validator() {
        let mut vs = make_validator_set(3, 1000);
        let mut db = InMemoryStateDB::new();
        // Two delegations to validator 1, one to validator 2.
        db.put_delegation(delegation(10, 1, 500));
        db.put_delegation(delegation(11, 1, 700));
        db.put_delegation(delegation(12, 2, 200));

        super::refresh_delegated_stakes(&mut vs, &db);

        assert_eq!(vs.get(1).unwrap().delegated_stake, 1200);
        assert_eq!(vs.get(2).unwrap().delegated_stake, 200);
        assert_eq!(vs.get(3).unwrap().delegated_stake, 0);
        // total_stake() rolls up self + delegated for non-jailed.
        assert_eq!(vs.total_stake(), 1000 * 3 + 1200 + 200);
        assert_eq!(vs.total_self_stake(), 3000);
    }

    #[test]
    fn test_effective_stake_includes_delegations() {
        let mut info = ValidatorInfo::new(7, 1000, [7u8; 32]);
        assert_eq!(info.effective_stake(), 1000);
        info.delegated_stake = 500;
        assert_eq!(info.effective_stake(), 1500);
    }

    fn delegator_addr(b: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    #[test]
    fn test_slash_delegations_proportional_with_remove() {
        let mut db = InMemoryStateDB::new();
        // Three delegations to validator 7.
        db.put_delegation(delegation(10, 7, 1000));
        db.put_delegation(delegation(11, 7, 100));
        let mut d_unbonding = delegation(12, 7, 500);
        d_unbonding.unbonding_amount = 200;
        db.put_delegation(d_unbonding);

        // Slash 50%.
        let total = slash_delegations_for_validator(&mut db, 7, 0.5);
        // Expected: 500 (delegator 10) + 50 (delegator 11) + 250 (active 12)
        // + 100 (unbonding 12) = 900
        assert_eq!(total, 900);

        assert_eq!(
            db.get_delegation(&delegator_addr(10), 7).map(|r| r.amount),
            Some(500)
        );
        assert_eq!(
            db.get_delegation(&delegator_addr(11), 7).map(|r| r.amount),
            Some(50)
        );
        let r12 = db.get_delegation(&delegator_addr(12), 7).unwrap();
        assert_eq!(r12.amount, 250);
        assert_eq!(r12.unbonding_amount, 100);
    }

    #[test]
    fn test_slash_delegations_removes_zero_records() {
        let mut db = InMemoryStateDB::new();
        db.put_delegation(delegation(10, 9, 1000));
        // 100% slash zeroes amount AND unbonding -> record removed.
        let total = slash_delegations_for_validator(&mut db, 9, 1.0);
        assert_eq!(total, 1000);
        assert!(
            db.get_delegation(&delegator_addr(10), 9).is_none(),
            "fully-slashed record removed"
        );
    }

    #[test]
    fn test_slash_delegations_pct_zero_or_oor_no_op() {
        let mut db = InMemoryStateDB::new();
        db.put_delegation(delegation(10, 9, 1000));
        assert_eq!(slash_delegations_for_validator(&mut db, 9, 0.0), 0);
        assert_eq!(slash_delegations_for_validator(&mut db, 9, -0.1), 0);
        assert_eq!(slash_delegations_for_validator(&mut db, 9, 1.5), 0);
        assert_eq!(
            db.get_delegation(&delegator_addr(10), 9).unwrap().amount,
            1000
        );
    }

    #[test]
    fn test_slash_delegations_unknown_validator_no_op() {
        let mut db = InMemoryStateDB::new();
        db.put_delegation(delegation(10, 7, 1000));
        let total = slash_delegations_for_validator(&mut db, 99, 0.5);
        assert_eq!(total, 0);
        assert_eq!(
            db.get_delegation(&delegator_addr(10), 7).unwrap().amount,
            1000
        );
    }

    // ── Validator key rotation (punch-list 4b/4d) ──────────────────────

    #[test]
    fn test_rotate_validator_key_stashes_prev_and_sets_expiry() {
        use evaporchain_crypto::signatures::BlsKeypair;
        let kp_old = BlsKeypair::generate();
        let kp_new = BlsKeypair::generate();
        let old_pk = kp_old.public_key_bytes().0.clone();
        let new_pk = kp_new.public_key_bytes().0.clone();

        let mut vs = ValidatorSet::new();
        let mut info = ValidatorInfo::new(7, 1000, [7u8; 32]);
        info.bls_public_key = Some(old_pk.clone());
        info.pop_verified = true;
        vs.add_validator(info);

        // PoP signature for the new key (any valid PoP — not actually
        // verified here since rotate_validator_key trusts its caller).
        let new_pop = kp_new.proof_of_possession().0.clone();
        assert!(vs.rotate_validator_key(7, new_pk.clone(), new_pop, 100));

        let v = vs.get(7).unwrap();
        assert_eq!(v.bls_public_key.as_ref().unwrap(), &new_pk);
        assert_eq!(v.bls_public_key_prev.as_ref().unwrap(), &old_pk);
        assert_eq!(v.bls_prev_key_expiry_epoch, Some(100));
    }

    #[test]
    fn test_purge_expired_prev_keys_clears_stale_entries() {
        use evaporchain_crypto::signatures::BlsKeypair;
        let mut vs = ValidatorSet::new();
        for vid in 1u64..=3 {
            let kp = BlsKeypair::generate();
            let mut info = ValidatorInfo::new(vid, 1000, [vid as u8; 32]);
            info.bls_public_key = Some(kp.public_key_bytes().0.clone());
            info.bls_public_key_prev = Some(vec![0u8; 48]);
            // Give each validator a different expiry to exercise the boundary.
            info.bls_prev_key_expiry_epoch = Some((vid * 10) as u64);
            info.pop_verified = true;
            vs.add_validator(info);
        }

        // current_epoch = 15 → expiry 10 is past, expiry 20 + 30 are still valid.
        let purged = vs.purge_expired_prev_keys(15);
        assert_eq!(purged, 1);
        assert!(vs.get(1).unwrap().bls_public_key_prev.is_none());
        assert!(vs.get(2).unwrap().bls_public_key_prev.is_some());
        assert!(vs.get(3).unwrap().bls_public_key_prev.is_some());

        // current_epoch = 25 → 20 also expires, 30 still grace.
        let purged = vs.purge_expired_prev_keys(25);
        assert_eq!(purged, 1);
        assert!(vs.get(2).unwrap().bls_public_key_prev.is_none());
        assert!(vs.get(3).unwrap().bls_public_key_prev.is_some());
    }

    #[test]
    fn test_rotate_validator_key_unknown_validator_returns_false() {
        let mut vs = ValidatorSet::new();
        assert!(!vs.rotate_validator_key(999, vec![0u8; 48], vec![0u8; 96], 100));
    }

    #[test]
    fn test_rotate_validator_key_no_current_key_returns_false() {
        let mut vs = ValidatorSet::new();
        // Validator with no BLS key registered yet.
        vs.add_validator(ValidatorInfo::new(7, 1000, [7u8; 32]));
        assert!(!vs.rotate_validator_key(7, vec![0u8; 48], vec![0u8; 96], 100));
    }

    #[test]
    fn validator_set_source_trait_delegates_to_validator_set() {
        // Lane G.5: trait dispatch parity. Holding ValidatorSet behind
        // `&dyn ValidatorSetSource`, all six trait methods must produce
        // results bit-equal to direct calls. Locks in the seam so
        // alternative impls (Singh-Boltzmann, Self-Annealing) can swap
        // in at the consensus read path without consensus code changes.
        let vs = make_validator_set(4, 1000);

        // Empty-set behaviour (separate Default).
        let empty = ValidatorSet::new();
        let empty_dyn: &dyn ValidatorSetSource = &empty;
        assert!(empty_dyn.is_empty());
        assert_eq!(empty_dyn.len(), 0);
        assert_eq!(empty_dyn.active_count(), 0);
        assert_eq!(empty_dyn.total_stake(), 0);
        assert!(empty_dyn.get(0).is_none());
        assert!(empty_dyn.leader_for_epoch(0).is_none());

        // Populated set: dyn dispatch matches concrete inherent calls.
        let dyn_vs: &dyn ValidatorSetSource = &vs;
        assert_eq!(dyn_vs.len(), vs.len());
        assert!(!dyn_vs.is_empty());
        assert_eq!(dyn_vs.active_count(), vs.active_count());
        assert_eq!(dyn_vs.total_stake(), vs.total_stake());

        // Lookup parity for an existing validator.
        let by_id_concrete = vs.get(0);
        let by_id_dyn = dyn_vs.get(0);
        assert_eq!(by_id_concrete.map(|v| v.id), by_id_dyn.map(|v| v.id));

        // Leader-for-epoch parity at three different epochs.
        for ep in [0u64, 1, 12345] {
            assert_eq!(
                vs.leader_for_epoch(ep).map(|v| v.id),
                dyn_vs.leader_for_epoch(ep).map(|v| v.id),
                "leader-for-epoch dispatch must match at epoch {ep}"
            );
        }
    }
}
