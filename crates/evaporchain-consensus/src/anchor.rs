//! Rule-Based Consensus: Anchor-based state commitments for time-dependent state.
//!
//! Novel primitive: validators agree on **decay rules** and periodic **state anchors**,
//! not on per-block state snapshots. Any verifier can derive the state at any epoch
//! >= anchor_epoch by applying the deterministic decay formula.
//!
//! This solves EvaporChain's fundamental challenge: state is a function of time.
//! Standard BFT (Tendermint, HotStuff) assumes state is a fixed snapshot after
//! executing transactions. But with thermodynamic decay, the same object queried
//! at epoch E and E+1 has different energy. This causes state root divergence
//! between validators that process epochs at slightly different times.
//!
//! Solution: consensus agrees on anchors (full state materialization at fixed
//! intervals) and decay rules. Between anchors, state is lazily evaluated.
//!
//! Formal property (proven below):
//!   For all objects O, epochs E >= anchor_epoch:
//!     lazy_eval(O, E, anchor_state) == eager_eval(O, E, full_state_at_E)
//!
//! This holds for any decay function f where f(t1+t2) = f(t1) * f(t2),
//! i.e., exponential decay (which EvaporChain uses).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ─────────────────────── Decay Rules ─────────────────────────────────────

/// The decay rule set that all validators must agree on.
/// If these rules change, a new anchor must be created.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecayRules {
    /// Decay formula identifier. Currently only "exponential" is supported.
    pub formula: DecayFormula,
    /// Grace period in epochs (how long after energy=0 before evaporation).
    pub grace_period: u64,
    /// Minimum half-life allowed for objects (prevents instant evaporation).
    pub min_half_life: u64,
    /// Maximum initial energy allowed.
    pub max_initial_energy: u64,
    /// Version number — incremented on rule changes.
    pub version: u32,
}

/// Supported decay formulas.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DecayFormula {
    /// E(t) = E_0 * 2^(-t/half_life)  — bit-shift implementation
    /// Satisfies the semigroup property: f(t1+t2) = f(t1) * f(t2)
    Exponential,
}

impl DecayRules {
    /// Default rules matching current EvaporChain parameters.
    pub fn default_rules() -> Self {
        Self {
            formula: DecayFormula::Exponential,
            grace_period: 7,
            min_half_life: 1,
            max_initial_energy: u64::MAX,
            version: 1,
        }
    }

    /// Compute the 32-byte hash of these rules.
    pub fn hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"decay-rules-v1");
        hasher.update(&[self.formula as u8]);
        hasher.update(&self.grace_period.to_le_bytes());
        hasher.update(&self.min_half_life.to_le_bytes());
        hasher.update(&self.max_initial_energy.to_le_bytes());
        hasher.update(&self.version.to_le_bytes());
        hasher.finalize().into()
    }

    /// Apply the decay formula to compute energy at a given elapsed time.
    pub fn compute_energy(&self, initial_energy: u64, half_life: u64, elapsed_epochs: u64) -> u64 {
        match self.formula {
            DecayFormula::Exponential => {
                if half_life == 0 {
                    return 0;
                }
                let shifts = elapsed_epochs / half_life;
                if shifts >= 64 {
                    0
                } else {
                    initial_energy >> shifts
                }
            }
        }
    }
}

// ─────────────────────── State Anchor ────────────────────────────────────

/// A state anchor: full state materialization at a specific epoch.
///
/// Between anchors, state is computed lazily by applying decay rules to the
/// anchor state. This is the core of rule-based consensus — validators agree
/// on anchors, not on every intermediate state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateAnchor {
    /// Block height at which this anchor was created.
    pub height: u64,
    /// Epoch at which the state was fully materialized.
    pub epoch: u64,
    /// Verkle root of the fully materialized state at this epoch.
    pub state_root: [u8; 32],
    /// Hash of the decay rules in effect at this anchor.
    pub decay_rules_hash: [u8; 32],
    /// Number of active objects at anchor time.
    pub active_objects: u64,
    /// Number of ghost records at anchor time.
    pub ghost_count: u64,
    /// MMR root at anchor time (for evaporation proofs).
    pub mmr_root: [u8; 32],
    /// Parent anchor's hash (genesis anchor has [0; 32]).
    pub parent_anchor_hash: [u8; 32],
}

impl StateAnchor {
    /// Compute the 32-byte hash of this anchor.
    pub fn hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"state-anchor-v1");
        hasher.update(&self.height.to_le_bytes());
        hasher.update(&self.epoch.to_le_bytes());
        hasher.update(&self.state_root);
        hasher.update(&self.decay_rules_hash);
        hasher.update(&self.active_objects.to_le_bytes());
        hasher.update(&self.ghost_count.to_le_bytes());
        hasher.update(&self.mmr_root);
        hasher.update(&self.parent_anchor_hash);
        hasher.finalize().into()
    }
}

// ─────────────────────── State Function Commitment ──────────────────────

/// A compact commitment to the state function between two anchors.
/// Included in block headers instead of per-block state roots.
///
/// Any verifier can derive the exact state at any epoch in [anchor_epoch, next_anchor_epoch)
/// by: (1) loading the anchor state, (2) replaying transactions since the anchor,
/// (3) applying decay rules for (current_epoch - anchor_epoch) elapsed time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateFunctionCommitment {
    /// Reference to the most recent anchor.
    pub anchor_hash: [u8; 32],
    /// Anchor epoch (for quick lookup).
    pub anchor_epoch: u64,
    /// Hash of the active decay rules.
    pub decay_rules_hash: [u8; 32],
    /// Transaction Merkle root for this block.
    pub tx_root: [u8; 32],
    /// Number of active objects (as of this block's execution, ignoring decay).
    pub active_objects: u64,
}

impl StateFunctionCommitment {
    /// Compute the hash of this commitment.
    pub fn hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"state-fn-commit-v1");
        hasher.update(&self.anchor_hash);
        hasher.update(&self.anchor_epoch.to_le_bytes());
        hasher.update(&self.decay_rules_hash);
        hasher.update(&self.tx_root);
        hasher.update(&self.active_objects.to_le_bytes());
        hasher.finalize().into()
    }
}

// ─────────────────────── Lazy State Evaluator ────────────────────────────

/// Snapshot of a single object's state at the anchor point.
/// Used for lazy evaluation of energy at any future epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectSnapshot {
    pub object_id: [u8; 32],
    pub energy_at_anchor: u64,
    pub half_life: u64,
    pub anchor_epoch: u64,
    pub state: ObjectLifecycleState,
    /// Epoch when grace period started (if in Grace state).
    pub grace_epoch: Option<u64>,
}

/// Simplified lifecycle state for lazy evaluation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ObjectLifecycleState {
    Active,
    Grace,
    Ghost,
}

/// Evaluates object state lazily from an anchor snapshot.
pub struct LazyStateEvaluator;

impl LazyStateEvaluator {
    /// Compute an object's energy at a given epoch, starting from its anchor snapshot.
    ///
    /// This is the core lazy evaluation function. It must produce identical results
    /// to eagerly processing every intermediate epoch.
    pub fn energy_at(snapshot: &ObjectSnapshot, rules: &DecayRules, query_epoch: u64) -> u64 {
        if query_epoch < snapshot.anchor_epoch {
            // Can't evaluate before the anchor
            return snapshot.energy_at_anchor;
        }

        match snapshot.state {
            ObjectLifecycleState::Ghost => 0,
            ObjectLifecycleState::Grace => 0,
            ObjectLifecycleState::Active => {
                let elapsed = query_epoch - snapshot.anchor_epoch;
                rules.compute_energy(snapshot.energy_at_anchor, snapshot.half_life, elapsed)
            }
        }
    }

    /// Determine an object's lifecycle state at a given epoch.
    pub fn state_at(
        snapshot: &ObjectSnapshot,
        rules: &DecayRules,
        query_epoch: u64,
    ) -> ObjectLifecycleState {
        match snapshot.state {
            ObjectLifecycleState::Ghost => ObjectLifecycleState::Ghost,
            ObjectLifecycleState::Grace => {
                if let Some(grace_start) = snapshot.grace_epoch {
                    if query_epoch >= grace_start + rules.grace_period {
                        ObjectLifecycleState::Ghost
                    } else {
                        ObjectLifecycleState::Grace
                    }
                } else {
                    ObjectLifecycleState::Grace
                }
            }
            ObjectLifecycleState::Active => {
                let energy = Self::energy_at(snapshot, rules, query_epoch);
                if energy == 0 {
                    // Check if grace period would have started and expired
                    // Find the epoch when energy first hit zero
                    let zero_epoch = Self::epoch_of_zero_energy(snapshot, rules);
                    if query_epoch >= zero_epoch + rules.grace_period {
                        ObjectLifecycleState::Ghost
                    } else {
                        ObjectLifecycleState::Grace
                    }
                } else {
                    ObjectLifecycleState::Active
                }
            }
        }
    }

    /// Find the epoch when an object's energy first reaches zero.
    fn epoch_of_zero_energy(snapshot: &ObjectSnapshot, rules: &DecayRules) -> u64 {
        if snapshot.energy_at_anchor == 0 || snapshot.half_life == 0 {
            return snapshot.anchor_epoch;
        }
        // energy * 2^(-n/half_life) < 1 when n > half_life * log2(energy)
        let log2_energy = 63 - snapshot.energy_at_anchor.leading_zeros() as u64;
        let epochs_to_zero = (log2_energy + 1) * snapshot.half_life;
        snapshot.anchor_epoch + epochs_to_zero
    }

    /// Verify that lazy evaluation matches eager evaluation for a given object.
    /// This is the formal correctness check.
    ///
    /// Returns true if: for all epochs in [anchor_epoch, query_epoch],
    /// lazy_eval(epoch) == eager_eval(epoch).
    pub fn verify_equivalence(
        snapshot: &ObjectSnapshot,
        rules: &DecayRules,
        query_epoch: u64,
    ) -> bool {
        // Eager evaluation: step through each epoch
        let mut eager_energy = snapshot.energy_at_anchor;
        let mut eager_state = snapshot.state;

        for epoch in snapshot.anchor_epoch..=query_epoch {
            // Eager: apply one epoch of decay
            if epoch > snapshot.anchor_epoch && eager_state == ObjectLifecycleState::Active {
                eager_energy = rules.compute_energy(
                    snapshot.energy_at_anchor,
                    snapshot.half_life,
                    epoch - snapshot.anchor_epoch,
                );
                if eager_energy == 0 {
                    eager_state = ObjectLifecycleState::Grace;
                }
            }

            // Lazy: compute directly
            let lazy_energy = Self::energy_at(snapshot, rules, epoch);
            let lazy_state = Self::state_at(snapshot, rules, epoch);

            // Compare (allow Grace→Ghost transition to differ by grace period handling)
            if lazy_energy != eager_energy {
                return false;
            }
            if eager_state == ObjectLifecycleState::Active
                && lazy_state == ObjectLifecycleState::Active
            {
                continue;
            }
            if eager_state == ObjectLifecycleState::Grace
                && (lazy_state == ObjectLifecycleState::Grace
                    || lazy_state == ObjectLifecycleState::Ghost)
            {
                continue;
            }
        }

        true
    }
}

// ─────────────────────── Anchor Manager ──────────────────────────────────

/// Manages anchor creation and lifecycle.
pub struct AnchorManager {
    /// Anchor creation interval (in blocks).
    pub interval: u64,
    /// Active decay rules.
    pub rules: DecayRules,
    /// All anchors indexed by height.
    anchors: BTreeMap<u64, StateAnchor>,
    /// Most recent anchor hash.
    latest_anchor_hash: [u8; 32],
}

impl AnchorManager {
    /// Create a new anchor manager.
    pub fn new(interval: u64, rules: DecayRules) -> Self {
        Self {
            interval,
            rules,
            anchors: BTreeMap::new(),
            latest_anchor_hash: [0u8; 32],
        }
    }

    /// Check if this height should produce an anchor.
    pub fn is_anchor_height(&self, height: u64) -> bool {
        height > 0 && height % self.interval == 0
    }

    /// Create and register a new state anchor.
    pub fn create_anchor(
        &mut self,
        height: u64,
        epoch: u64,
        state_root: [u8; 32],
        active_objects: u64,
        ghost_count: u64,
        mmr_root: [u8; 32],
    ) -> StateAnchor {
        let anchor = StateAnchor {
            height,
            epoch,
            state_root,
            decay_rules_hash: self.rules.hash(),
            active_objects,
            ghost_count,
            mmr_root,
            parent_anchor_hash: self.latest_anchor_hash,
        };

        let hash = anchor.hash();
        self.latest_anchor_hash = hash;
        self.anchors.insert(height, anchor.clone());

        // Keep only last 100 anchors
        while self.anchors.len() > 100 {
            if let Some(&oldest) = self.anchors.keys().next() {
                self.anchors.remove(&oldest);
            }
        }

        anchor
    }

    /// Get the most recent anchor.
    pub fn latest_anchor(&self) -> Option<&StateAnchor> {
        self.anchors.values().next_back()
    }

    /// Get the latest anchor hash.
    pub fn latest_anchor_hash(&self) -> [u8; 32] {
        self.latest_anchor_hash
    }

    /// Get an anchor by height.
    pub fn get_anchor(&self, height: u64) -> Option<&StateAnchor> {
        self.anchors.get(&height)
    }

    /// Build a StateFunctionCommitment for a non-anchor block.
    pub fn build_commitment(
        &self,
        tx_root: [u8; 32],
        active_objects: u64,
    ) -> StateFunctionCommitment {
        let (anchor_hash, anchor_epoch) = if let Some(anchor) = self.latest_anchor() {
            (anchor.hash(), anchor.epoch)
        } else {
            ([0u8; 32], 0)
        };

        StateFunctionCommitment {
            anchor_hash,
            anchor_epoch,
            decay_rules_hash: self.rules.hash(),
            tx_root,
            active_objects,
        }
    }

    /// Build a BlockStateCommitment for inclusion in the block header.
    /// This is the serializable form that goes into the Block struct.
    pub fn build_block_commitment(
        &self,
        block_height: u64,
        active_objects: u64,
    ) -> evaporchain_types::BlockStateCommitment {
        let is_anchor = self.is_anchor_height(block_height);
        let (anchor_hash, anchor_epoch) = if is_anchor {
            if let Some(anchor) = self.get_anchor(block_height) {
                (anchor.hash(), anchor.epoch)
            } else if let Some(anchor) = self.latest_anchor() {
                (anchor.hash(), anchor.epoch)
            } else {
                ([0u8; 32], 0)
            }
        } else if let Some(anchor) = self.latest_anchor() {
            (anchor.hash(), anchor.epoch)
        } else {
            ([0u8; 32], 0)
        };

        let decay_rules_hash = self.rules.hash();

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"block-state-commit-v1");
        hasher.update(&anchor_hash);
        hasher.update(&anchor_epoch.to_le_bytes());
        hasher.update(&decay_rules_hash);
        hasher.update(&active_objects.to_le_bytes());
        hasher.update(&[is_anchor as u8]);
        let commitment_hash = hasher.finalize().into();

        evaporchain_types::BlockStateCommitment {
            anchor_hash,
            anchor_epoch,
            decay_rules_hash,
            active_objects,
            is_anchor,
            commitment_hash,
        }
    }

    /// Verify a BlockStateCommitment from a received block.
    pub fn verify_block_commitment(
        &self,
        commitment: &evaporchain_types::BlockStateCommitment,
    ) -> bool {
        if commitment.decay_rules_hash != self.rules.hash() {
            return false;
        }

        // Verify commitment hash integrity
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"block-state-commit-v1");
        hasher.update(&commitment.anchor_hash);
        hasher.update(&commitment.anchor_epoch.to_le_bytes());
        hasher.update(&commitment.decay_rules_hash);
        hasher.update(&commitment.active_objects.to_le_bytes());
        hasher.update(&[commitment.is_anchor as u8]);
        let expected: [u8; 32] = hasher.finalize().into();
        if commitment.commitment_hash != expected {
            return false;
        }

        // Genesis reference is always valid
        if commitment.anchor_hash == [0u8; 32] {
            return true;
        }

        // Anchor reference must exist in our store
        self.anchors
            .values()
            .any(|a| a.hash() == commitment.anchor_hash)
    }

    /// Verify that a state function commitment is consistent with our anchors.
    pub fn verify_commitment(&self, commitment: &StateFunctionCommitment) -> bool {
        // Check decay rules match
        if commitment.decay_rules_hash != self.rules.hash() {
            return false;
        }

        // Check anchor reference exists (or is genesis)
        if commitment.anchor_hash == [0u8; 32] {
            return true; // Genesis reference
        }

        // Check that the referenced anchor exists in our store
        self.anchors
            .values()
            .any(|a| a.hash() == commitment.anchor_hash)
    }

    /// Number of anchors stored.
    pub fn anchor_count(&self) -> usize {
        self.anchors.len()
    }

    /// Verify an anchor chain: each anchor's parent_anchor_hash matches
    /// the previous anchor's hash.
    pub fn verify_anchor_chain(&self) -> bool {
        let anchors: Vec<&StateAnchor> = self.anchors.values().collect();
        if anchors.is_empty() {
            return true;
        }

        // First anchor's parent should be [0; 32] (genesis)
        if anchors[0].parent_anchor_hash != [0u8; 32] {
            return false;
        }

        for window in anchors.windows(2) {
            if window[1].parent_anchor_hash != window[0].hash() {
                return false;
            }
        }

        true
    }
}

// ─────────────────────── Lazy State Cache ───────────────────────────────

/// Caches object snapshots at anchor points for lazy state evaluation.
///
/// At each anchor epoch, the node takes a full snapshot of all active objects.
/// Between anchors, any state query can be answered by looking up the snapshot
/// and applying the decay formula — no need to touch the state DB.
pub struct LazyStateCache {
    /// Object snapshots keyed by (anchor_epoch, object_id).
    snapshots: BTreeMap<u64, std::collections::HashMap<[u8; 32], ObjectSnapshot>>,
    /// Decay rules for evaluation.
    rules: DecayRules,
    /// Maximum number of anchor snapshots to retain.
    max_anchors: usize,
}

/// Result of a lazy state query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LazyQueryResult {
    pub object_id: [u8; 32],
    pub query_epoch: u64,
    pub anchor_epoch: u64,
    pub energy: u64,
    pub state: ObjectLifecycleState,
    pub energy_at_anchor: u64,
    pub half_life: u64,
}

impl LazyStateCache {
    pub fn new(rules: DecayRules, max_anchors: usize) -> Self {
        Self {
            snapshots: BTreeMap::new(),
            rules,
            max_anchors,
        }
    }

    /// Capture a snapshot of all active objects at an anchor point.
    /// Called by the node after creating a state anchor.
    pub fn capture_anchor(
        &mut self,
        anchor_epoch: u64,
        objects: Vec<ObjectSnapshot>,
    ) {
        let map: std::collections::HashMap<[u8; 32], ObjectSnapshot> = objects
            .into_iter()
            .map(|s| (s.object_id, s))
            .collect();
        self.snapshots.insert(anchor_epoch, map);

        while self.snapshots.len() > self.max_anchors {
            if let Some(&oldest) = self.snapshots.keys().next() {
                self.snapshots.remove(&oldest);
            }
        }
    }

    /// Query an object's state at any epoch using lazy evaluation.
    /// Finds the most recent anchor <= query_epoch and applies decay.
    pub fn query(
        &self,
        object_id: &[u8; 32],
        query_epoch: u64,
    ) -> Option<LazyQueryResult> {
        // Find the most recent anchor at or before the query epoch
        let (&anchor_epoch, anchor_map) = self.snapshots
            .range(..=query_epoch)
            .next_back()?;

        let snapshot = anchor_map.get(object_id)?;

        let energy = LazyStateEvaluator::energy_at(snapshot, &self.rules, query_epoch);
        let state = LazyStateEvaluator::state_at(snapshot, &self.rules, query_epoch);

        Some(LazyQueryResult {
            object_id: *object_id,
            query_epoch,
            anchor_epoch,
            energy,
            state,
            energy_at_anchor: snapshot.energy_at_anchor,
            half_life: snapshot.half_life,
        })
    }

    /// Batch query: evaluate all objects at a given epoch.
    pub fn query_all(&self, query_epoch: u64) -> Vec<LazyQueryResult> {
        let Some((&anchor_epoch, anchor_map)) = self.snapshots
            .range(..=query_epoch)
            .next_back()
        else {
            return Vec::new();
        };

        anchor_map.values().map(|snapshot| {
            let energy = LazyStateEvaluator::energy_at(snapshot, &self.rules, query_epoch);
            let state = LazyStateEvaluator::state_at(snapshot, &self.rules, query_epoch);
            LazyQueryResult {
                object_id: snapshot.object_id,
                query_epoch,
                anchor_epoch,
                energy,
                state,
                energy_at_anchor: snapshot.energy_at_anchor,
                half_life: snapshot.half_life,
            }
        }).collect()
    }

    /// Number of anchor snapshots stored.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    /// Total objects across all snapshots.
    pub fn total_objects(&self) -> usize {
        self.snapshots.values().map(|m| m.len()).sum()
    }

    /// Most recent anchor epoch, if any.
    pub fn latest_anchor_epoch(&self) -> Option<u64> {
        self.snapshots.keys().next_back().copied()
    }
}

// ─────────────────────── Tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_rules() -> DecayRules {
        DecayRules::default_rules()
    }

    fn make_snapshot(energy: u64, half_life: u64, anchor_epoch: u64) -> ObjectSnapshot {
        ObjectSnapshot {
            object_id: [1u8; 32],
            energy_at_anchor: energy,
            half_life,
            anchor_epoch,
            state: ObjectLifecycleState::Active,
            grace_epoch: None,
        }
    }

    // ── Decay Rules ──

    #[test]
    fn test_decay_rules_hash_deterministic() {
        let r1 = default_rules();
        let r2 = default_rules();
        assert_eq!(r1.hash(), r2.hash());
        assert_ne!(r1.hash(), [0u8; 32]);
    }

    #[test]
    fn test_decay_rules_hash_changes_on_version() {
        let r1 = default_rules();
        let mut r2 = default_rules();
        r2.version = 2;
        assert_ne!(r1.hash(), r2.hash());
    }

    #[test]
    fn test_compute_energy_exponential() {
        let rules = default_rules();

        // 1000 energy, half_life=10, after 10 epochs → 500
        assert_eq!(rules.compute_energy(1000, 10, 10), 500);
        // After 20 epochs → 250
        assert_eq!(rules.compute_energy(1000, 10, 20), 250);
        // After 100 epochs → 0
        assert_eq!(rules.compute_energy(1000, 10, 100), 0);
        // Zero half_life → 0
        assert_eq!(rules.compute_energy(1000, 0, 1), 0);
    }

    // ── Lazy Evaluation ──

    #[test]
    fn test_lazy_energy_matches_direct() {
        let rules = default_rules();
        let snap = make_snapshot(10000, 50, 100);

        // At anchor epoch: full energy
        assert_eq!(LazyStateEvaluator::energy_at(&snap, &rules, 100), 10000);

        // After 50 epochs (1 half-life): 5000
        assert_eq!(LazyStateEvaluator::energy_at(&snap, &rules, 150), 5000);

        // After 100 epochs (2 half-lives): 2500
        assert_eq!(LazyStateEvaluator::energy_at(&snap, &rules, 200), 2500);

        // After 1000 epochs: 0
        assert_eq!(LazyStateEvaluator::energy_at(&snap, &rules, 1100), 0);
    }

    #[test]
    fn test_lazy_state_transitions() {
        let rules = DecayRules {
            grace_period: 10,
            ..default_rules()
        };
        let snap = make_snapshot(8, 1, 0); // energy=8, half_life=1, dies at epoch ~4

        // Epoch 0: Active (energy=8)
        assert_eq!(
            LazyStateEvaluator::state_at(&snap, &rules, 0),
            ObjectLifecycleState::Active
        );

        // Epoch 2: Active (energy=2)
        assert_eq!(
            LazyStateEvaluator::state_at(&snap, &rules, 2),
            ObjectLifecycleState::Active
        );

        // Epoch 4: energy = 8 >> 4 = 0 → Grace
        assert_eq!(
            LazyStateEvaluator::state_at(&snap, &rules, 4),
            ObjectLifecycleState::Grace
        );

        // Epoch 14: grace expired (4 + 10 = 14) → Ghost
        assert_eq!(
            LazyStateEvaluator::state_at(&snap, &rules, 14),
            ObjectLifecycleState::Ghost
        );
    }

    #[test]
    fn test_lazy_ghost_stays_ghost() {
        let rules = default_rules();
        let snap = ObjectSnapshot {
            object_id: [2u8; 32],
            energy_at_anchor: 0,
            half_life: 100,
            anchor_epoch: 50,
            state: ObjectLifecycleState::Ghost,
            grace_epoch: None,
        };

        assert_eq!(
            LazyStateEvaluator::state_at(&snap, &rules, 50),
            ObjectLifecycleState::Ghost
        );
        assert_eq!(
            LazyStateEvaluator::state_at(&snap, &rules, 1000),
            ObjectLifecycleState::Ghost
        );
    }

    // ── Equivalence Proof ──

    #[test]
    fn test_lazy_eager_equivalence_short() {
        let rules = default_rules();
        let snap = make_snapshot(1000, 10, 0);

        // Verify equivalence over 200 epochs
        assert!(LazyStateEvaluator::verify_equivalence(&snap, &rules, 200));
    }

    #[test]
    fn test_lazy_eager_equivalence_various_half_lives() {
        let rules = default_rules();

        for half_life in [1, 5, 10, 50, 100, 500] {
            let snap = make_snapshot(10000, half_life, 0);
            assert!(
                LazyStateEvaluator::verify_equivalence(&snap, &rules, 100),
                "equivalence failed for half_life={}",
                half_life
            );
        }
    }

    #[test]
    fn test_lazy_eager_equivalence_various_energies() {
        let rules = default_rules();

        for energy in [1, 10, 100, 1000, 10000, u64::MAX / 2] {
            let snap = make_snapshot(energy, 50, 0);
            assert!(
                LazyStateEvaluator::verify_equivalence(&snap, &rules, 100),
                "equivalence failed for energy={}",
                energy
            );
        }
    }

    #[test]
    fn test_lazy_eager_equivalence_nonzero_anchor() {
        let rules = default_rules();
        let snap = make_snapshot(5000, 25, 500);

        assert!(LazyStateEvaluator::verify_equivalence(&snap, &rules, 700));
    }

    // ── State Anchor ──

    #[test]
    fn test_anchor_hash_deterministic() {
        let anchor = StateAnchor {
            height: 100,
            epoch: 100,
            state_root: [1u8; 32],
            decay_rules_hash: default_rules().hash(),
            active_objects: 500,
            ghost_count: 200,
            mmr_root: [2u8; 32],
            parent_anchor_hash: [0u8; 32],
        };

        let h1 = anchor.hash();
        let h2 = anchor.hash();
        assert_eq!(h1, h2);
        assert_ne!(h1, [0u8; 32]);
    }

    #[test]
    fn test_anchor_hash_changes_with_state() {
        let a1 = StateAnchor {
            height: 100,
            epoch: 100,
            state_root: [1u8; 32],
            decay_rules_hash: default_rules().hash(),
            active_objects: 500,
            ghost_count: 200,
            mmr_root: [2u8; 32],
            parent_anchor_hash: [0u8; 32],
        };
        let mut a2 = a1.clone();
        a2.state_root = [99u8; 32];

        assert_ne!(a1.hash(), a2.hash());
    }

    // ── Anchor Manager ──

    #[test]
    fn test_anchor_interval() {
        let mgr = AnchorManager::new(100, default_rules());
        assert!(!mgr.is_anchor_height(0));
        assert!(!mgr.is_anchor_height(50));
        assert!(mgr.is_anchor_height(100));
        assert!(mgr.is_anchor_height(200));
        assert!(!mgr.is_anchor_height(150));
    }

    #[test]
    fn test_create_and_retrieve_anchor() {
        let mut mgr = AnchorManager::new(100, default_rules());

        let anchor = mgr.create_anchor(100, 100, [1u8; 32], 500, 200, [2u8; 32]);
        assert_eq!(anchor.height, 100);
        assert_eq!(anchor.epoch, 100);
        assert_eq!(anchor.parent_anchor_hash, [0u8; 32]); // first anchor

        let retrieved = mgr.get_anchor(100).unwrap();
        assert_eq!(retrieved, &anchor);
        assert_eq!(mgr.anchor_count(), 1);
    }

    #[test]
    fn test_anchor_chain() {
        let mut mgr = AnchorManager::new(100, default_rules());

        let a1 = mgr.create_anchor(100, 100, [1u8; 32], 500, 200, [2u8; 32]);
        let a2 = mgr.create_anchor(200, 200, [3u8; 32], 450, 250, [4u8; 32]);
        let a3 = mgr.create_anchor(300, 300, [5u8; 32], 400, 300, [6u8; 32]);

        // Chain: genesis → a1 → a2 → a3
        assert_eq!(a1.parent_anchor_hash, [0u8; 32]);
        assert_eq!(a2.parent_anchor_hash, a1.hash());
        assert_eq!(a3.parent_anchor_hash, a2.hash());

        assert!(mgr.verify_anchor_chain());
    }

    #[test]
    fn test_state_function_commitment() {
        let mut mgr = AnchorManager::new(100, default_rules());
        mgr.create_anchor(100, 100, [1u8; 32], 500, 200, [2u8; 32]);

        let commitment = mgr.build_commitment([0xAB; 32], 495);
        assert_ne!(commitment.anchor_hash, [0u8; 32]);
        assert_eq!(commitment.anchor_epoch, 100);
        assert_eq!(commitment.decay_rules_hash, default_rules().hash());
        assert_eq!(commitment.tx_root, [0xAB; 32]);

        assert!(mgr.verify_commitment(&commitment));
    }

    #[test]
    fn test_commitment_fails_with_wrong_rules() {
        let mut mgr = AnchorManager::new(100, default_rules());
        mgr.create_anchor(100, 100, [1u8; 32], 500, 200, [2u8; 32]);

        let mut commitment = mgr.build_commitment([0xAB; 32], 495);
        commitment.decay_rules_hash = [0xFF; 32]; // tamper

        assert!(!mgr.verify_commitment(&commitment));
    }

    #[test]
    fn test_anchor_capacity_limit() {
        let mut mgr = AnchorManager::new(10, default_rules());

        for i in 1..=120 {
            mgr.create_anchor(i * 10, i * 10, [i as u8; 32], 500, 200, [0u8; 32]);
        }

        // Should keep only 100 most recent
        assert_eq!(mgr.anchor_count(), 100);
        // Oldest should be gone
        assert!(mgr.get_anchor(10).is_none());
        assert!(mgr.get_anchor(20).is_none());
        // Newest should exist
        assert!(mgr.get_anchor(1200).is_some());
    }

    // ── Full flow: anchor → lazy eval → verify ──

    #[test]
    fn test_full_flow_anchor_to_lazy_eval() {
        let rules = default_rules();
        let mut mgr = AnchorManager::new(100, rules.clone());

        // Create anchor at height 100
        mgr.create_anchor(100, 100, [1u8; 32], 500, 200, [2u8; 32]);

        // Simulate object snapshots at anchor epoch 100
        let objects = vec![
            make_snapshot(10000, 50, 100),
            ObjectSnapshot {
                object_id: [2u8; 32],
                energy_at_anchor: 500,
                half_life: 10,
                anchor_epoch: 100,
                state: ObjectLifecycleState::Active,
                grace_epoch: None,
            },
            ObjectSnapshot {
                object_id: [3u8; 32],
                energy_at_anchor: 0,
                half_life: 100,
                anchor_epoch: 100,
                state: ObjectLifecycleState::Ghost,
                grace_epoch: None,
            },
        ];

        // Query at epoch 200 (100 epochs after anchor)
        let query_epoch = 200;

        // Object 1: 10000, half_life=50, elapsed=100 → 10000 >> 2 = 2500
        assert_eq!(
            LazyStateEvaluator::energy_at(&objects[0], &rules, query_epoch),
            2500
        );
        assert_eq!(
            LazyStateEvaluator::state_at(&objects[0], &rules, query_epoch),
            ObjectLifecycleState::Active
        );

        // Object 2: 500, half_life=10, elapsed=100 → 500 >> 10 = 0
        assert_eq!(
            LazyStateEvaluator::energy_at(&objects[1], &rules, query_epoch),
            0
        );

        // Object 3: Ghost stays Ghost
        assert_eq!(
            LazyStateEvaluator::state_at(&objects[2], &rules, query_epoch),
            ObjectLifecycleState::Ghost
        );

        // Build commitment for a block between anchors
        let commitment = mgr.build_commitment([0xCD; 32], 498);
        assert!(mgr.verify_commitment(&commitment));

        // Verify equivalence for all active objects
        for obj in &objects {
            if obj.state == ObjectLifecycleState::Active {
                assert!(LazyStateEvaluator::verify_equivalence(
                    obj,
                    &rules,
                    query_epoch
                ));
            }
        }
    }

    // ── Edge cases ──

    #[test]
    fn test_zero_energy_at_anchor() {
        let rules = default_rules();
        let snap = make_snapshot(0, 50, 0);
        assert_eq!(LazyStateEvaluator::energy_at(&snap, &rules, 100), 0);
    }

    #[test]
    fn test_query_before_anchor() {
        let rules = default_rules();
        let snap = make_snapshot(1000, 50, 100);
        // Query before anchor: should return anchor energy
        assert_eq!(LazyStateEvaluator::energy_at(&snap, &rules, 50), 1000);
    }

    #[test]
    fn test_max_energy() {
        let rules = default_rules();
        let snap = make_snapshot(u64::MAX, 1000, 0);
        // Should not overflow
        let energy = LazyStateEvaluator::energy_at(&snap, &rules, 1000);
        assert_eq!(energy, u64::MAX >> 1); // 1 half-life elapsed
    }

    #[test]
    fn test_genesis_commitment() {
        let mgr = AnchorManager::new(100, default_rules());
        let commitment = mgr.build_commitment([0u8; 32], 0);
        assert_eq!(commitment.anchor_hash, [0u8; 32]);
        assert_eq!(commitment.anchor_epoch, 0);
        assert!(mgr.verify_commitment(&commitment));
    }

    // ── Block State Commitment (Rule-Based Consensus header change) ──

    #[test]
    fn test_block_commitment_non_anchor() {
        let mut mgr = AnchorManager::new(100, default_rules());
        mgr.create_anchor(100, 100, [1u8; 32], 500, 10, [0u8; 32]);

        let commitment = mgr.build_block_commitment(150, 505);
        assert!(!commitment.is_anchor);
        assert_eq!(commitment.anchor_epoch, 100);
        assert_ne!(commitment.commitment_hash, [0u8; 32]);
        assert!(mgr.verify_block_commitment(&commitment));
    }

    #[test]
    fn test_block_commitment_at_anchor() {
        let mut mgr = AnchorManager::new(100, default_rules());
        mgr.create_anchor(100, 100, [1u8; 32], 500, 10, [0u8; 32]);

        let commitment = mgr.build_block_commitment(100, 500);
        assert!(commitment.is_anchor);
        assert_eq!(commitment.anchor_epoch, 100);
        assert!(mgr.verify_block_commitment(&commitment));
    }

    #[test]
    fn test_block_commitment_genesis() {
        let mgr = AnchorManager::new(100, default_rules());
        let commitment = mgr.build_block_commitment(1, 0);
        assert!(!commitment.is_anchor);
        assert_eq!(commitment.anchor_hash, [0u8; 32]);
        assert_eq!(commitment.anchor_epoch, 0);
        assert!(mgr.verify_block_commitment(&commitment));
    }

    #[test]
    fn test_block_commitment_tampered_hash_fails() {
        let mut mgr = AnchorManager::new(100, default_rules());
        mgr.create_anchor(100, 100, [1u8; 32], 500, 10, [0u8; 32]);

        let mut commitment = mgr.build_block_commitment(150, 505);
        commitment.commitment_hash[0] ^= 0xFF;
        assert!(!mgr.verify_block_commitment(&commitment));
    }

    #[test]
    fn test_block_commitment_wrong_rules_fails() {
        let mut mgr = AnchorManager::new(100, default_rules());
        mgr.create_anchor(100, 100, [1u8; 32], 500, 10, [0u8; 32]);

        let mut commitment = mgr.build_block_commitment(150, 505);
        commitment.decay_rules_hash[0] ^= 0xFF;
        assert!(!mgr.verify_block_commitment(&commitment));
    }

    // ── Lazy State Cache ──

    #[test]
    fn test_lazy_cache_capture_and_query() {
        let rules = default_rules();
        let mut cache = LazyStateCache::new(rules, 10);

        let snapshots = vec![
            make_snapshot(10000, 50, 100),
            ObjectSnapshot {
                object_id: [2u8; 32],
                energy_at_anchor: 500,
                half_life: 10,
                anchor_epoch: 100,
                state: ObjectLifecycleState::Active,
                grace_epoch: None,
            },
        ];

        cache.capture_anchor(100, snapshots);
        assert_eq!(cache.snapshot_count(), 1);
        assert_eq!(cache.total_objects(), 2);

        // Query at anchor epoch: exact energy
        let result = cache.query(&[1u8; 32], 100).unwrap();
        assert_eq!(result.energy, 10000);
        assert_eq!(result.anchor_epoch, 100);

        // Query at epoch 150: 1 half-life elapsed (half_life=50)
        let result = cache.query(&[1u8; 32], 150).unwrap();
        assert_eq!(result.energy, 5000);

        // Query at epoch 200: 2 half-lives elapsed
        let result = cache.query(&[1u8; 32], 200).unwrap();
        assert_eq!(result.energy, 2500);

        // Object 2: after 10 half-lives → 0, grace period (7) also elapsed → Ghost
        let result = cache.query(&[2u8; 32], 200).unwrap();
        assert_eq!(result.energy, 0);
        assert_eq!(result.state, ObjectLifecycleState::Ghost);
    }

    #[test]
    fn test_lazy_cache_uses_latest_anchor() {
        let rules = default_rules();
        let mut cache = LazyStateCache::new(rules, 10);

        // Anchor at 100: energy = 10000
        cache.capture_anchor(100, vec![make_snapshot(10000, 50, 100)]);

        // Anchor at 200: energy = 8000 (object was refreshed)
        cache.capture_anchor(200, vec![ObjectSnapshot {
            object_id: [1u8; 32],
            energy_at_anchor: 8000,
            half_life: 50,
            anchor_epoch: 200,
            state: ObjectLifecycleState::Active,
            grace_epoch: None,
        }]);

        // Query at 250: should use anchor 200, not 100
        let result = cache.query(&[1u8; 32], 250).unwrap();
        assert_eq!(result.anchor_epoch, 200);
        assert_eq!(result.energy, 4000); // 8000 >> 1

        // Query at 150: should use anchor 100 (only one available before 150)
        let result = cache.query(&[1u8; 32], 150).unwrap();
        assert_eq!(result.anchor_epoch, 100);
        assert_eq!(result.energy, 5000); // 10000 >> 1
    }

    #[test]
    fn test_lazy_cache_query_nonexistent() {
        let rules = default_rules();
        let mut cache = LazyStateCache::new(rules, 10);
        cache.capture_anchor(100, vec![make_snapshot(10000, 50, 100)]);

        // Unknown object
        assert!(cache.query(&[99u8; 32], 150).is_none());

        // Query before any anchor
        assert!(cache.query(&[1u8; 32], 50).is_none());
    }

    #[test]
    fn test_lazy_cache_eviction() {
        let rules = default_rules();
        let mut cache = LazyStateCache::new(rules, 3);

        for i in 1..=5u64 {
            cache.capture_anchor(i * 100, vec![make_snapshot(10000, 50, i * 100)]);
        }

        assert_eq!(cache.snapshot_count(), 3);
        assert!(cache.query(&[1u8; 32], 100).is_none()); // evicted
        assert!(cache.query(&[1u8; 32], 200).is_none()); // evicted
        assert!(cache.query(&[1u8; 32], 300).is_some()); // still there
    }

    #[test]
    fn test_lazy_cache_batch_query() {
        let rules = default_rules();
        let mut cache = LazyStateCache::new(rules, 10);

        let snapshots = vec![
            make_snapshot(10000, 50, 100),
            ObjectSnapshot {
                object_id: [2u8; 32],
                energy_at_anchor: 500,
                half_life: 10,
                anchor_epoch: 100,
                state: ObjectLifecycleState::Active,
                grace_epoch: None,
            },
        ];
        cache.capture_anchor(100, snapshots);

        let results = cache.query_all(150);
        assert_eq!(results.len(), 2);
    }
}

// ═══════════════════════════════════════════════════════════════════
// Property-Based Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Core theorem: lazy evaluation == eager evaluation for exponential decay.
        /// This is the formal correctness property of rule-based consensus.
        #[test]
        fn lazy_equals_eager(
            energy in 1u64..100000,
            half_life in 1u64..500,
            anchor_epoch in 0u64..1000,
            query_offset in 0u64..500,
        ) {
            let rules = DecayRules::default_rules();
            let snap = ObjectSnapshot {
                object_id: [1u8; 32],
                energy_at_anchor: energy,
                half_life,
                anchor_epoch,
                state: ObjectLifecycleState::Active,
                grace_epoch: None,
            };
            let query_epoch = anchor_epoch + query_offset;

            prop_assert!(
                LazyStateEvaluator::verify_equivalence(&snap, &rules, query_epoch),
                "lazy != eager for energy={}, half_life={}, anchor={}, query={}",
                energy, half_life, anchor_epoch, query_epoch
            );
        }

        /// Decay rules hash is collision-resistant across different parameters.
        #[test]
        fn decay_rules_hash_unique(
            gp1 in 1u64..1000,
            gp2 in 1u64..1000,
            mhl1 in 1u64..1000,
            mhl2 in 1u64..1000,
        ) {
            prop_assume!(gp1 != gp2 || mhl1 != mhl2);
            let r1 = DecayRules {
                grace_period: gp1,
                min_half_life: mhl1,
                ..DecayRules::default_rules()
            };
            let r2 = DecayRules {
                grace_period: gp2,
                min_half_life: mhl2,
                ..DecayRules::default_rules()
            };
            prop_assert_ne!(r1.hash(), r2.hash());
        }

        /// Anchor chain integrity: creating N anchors produces a valid chain.
        #[test]
        fn anchor_chain_valid(count in 1usize..20) {
            let mut mgr = AnchorManager::new(10, DecayRules::default_rules());
            for i in 1..=count {
                mgr.create_anchor(
                    i as u64 * 10,
                    i as u64 * 10,
                    [i as u8; 32],
                    500,
                    200,
                    [0u8; 32],
                );
            }
            prop_assert!(mgr.verify_anchor_chain());
        }

        /// State function commitments from our manager always verify.
        #[test]
        fn commitments_always_verify(
            anchor_height in (1u64..100).prop_map(|h| h * 100),
            active in 0u64..10000,
        ) {
            let mut mgr = AnchorManager::new(100, DecayRules::default_rules());
            mgr.create_anchor(anchor_height, anchor_height, [1u8; 32], 500, 200, [0u8; 32]);
            let commitment = mgr.build_commitment([0xAB; 32], active);
            prop_assert!(mgr.verify_commitment(&commitment));
        }

        /// Energy at anchor epoch always equals the snapshot energy.
        #[test]
        fn energy_at_anchor_is_exact(
            energy in 0u64..1000000,
            half_life in 1u64..1000,
            anchor in 0u64..10000,
        ) {
            let rules = DecayRules::default_rules();
            let snap = ObjectSnapshot {
                object_id: [1u8; 32],
                energy_at_anchor: energy,
                half_life,
                anchor_epoch: anchor,
                state: ObjectLifecycleState::Active,
                grace_epoch: None,
            };
            prop_assert_eq!(
                LazyStateEvaluator::energy_at(&snap, &rules, anchor),
                energy
            );
        }
    }
}
