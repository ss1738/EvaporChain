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

use evaporchain_crypto::hash::blake3_hash;
use serde::{Deserialize, Serialize};

/// Maximum health score bonus (20% extra weight).
const HEALTH_BONUS_CAP: f64 = 0.2;

/// Health score decay per epoch (small decay to keep validators active).
const HEALTH_DECAY_RATE: f64 = 0.01;

/// Health score increment per evaporation processed.
const HEALTH_PER_EVAPORATION: f64 = 0.05;

/// Maximum health score.
const MAX_HEALTH_SCORE: f64 = 1.0;

// ─────────────────────── ValidatorInfo ────────────────────────────────────

/// Information about a registered validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorInfo {
    /// Unique validator identifier.
    pub id: u64,
    /// Staked amount (determines base weight in leader selection).
    pub stake: u64,
    /// Validator's network address / public key.
    pub address: [u8; 32],
    /// Total blocks produced by this validator.
    pub blocks_produced: u64,
    /// Total evaporations processed across all blocks.
    pub evaporations_processed: u64,
    /// Health score (0.0–1.0) reflecting thermodynamic contribution.
    pub health_score: f64,
}

impl ValidatorInfo {
    /// Create a new validator with the given id, stake, and address.
    pub fn new(id: u64, stake: u64, address: [u8; 32]) -> Self {
        Self {
            id,
            stake,
            address,
            blocks_produced: 0,
            evaporations_processed: 0,
            health_score: 0.0,
        }
    }

    /// Compute this validator's effective weight for leader selection.
    /// weight = stake * (1.0 + min(health_score, 1.0) * 0.2)
    pub fn effective_weight(&self) -> u64 {
        let health_capped = self.health_score.min(MAX_HEALTH_SCORE);
        let multiplier = 1.0 + health_capped * HEALTH_BONUS_CAP;
        (self.stake as f64 * multiplier).round() as u64
    }
}

// ─────────────────────── ValidatorSet ─────────────────────────────────────

/// Set of validators with energy-weighted leader selection.
#[derive(Debug, Clone)]
pub struct ValidatorSet {
    validators: Vec<ValidatorInfo>,
}

impl ValidatorSet {
    /// Create an empty validator set.
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }

    /// Create a validator set from a list of validators.
    pub fn with_validators(validators: Vec<ValidatorInfo>) -> Self {
        Self { validators }
    }

    /// Add a validator to the set.
    pub fn add_validator(&mut self, info: ValidatorInfo) {
        // Don't add duplicates
        if !self.validators.iter().any(|v| v.id == info.id) {
            self.validators.push(info);
        }
    }

    /// Remove a validator by id.
    pub fn remove_validator(&mut self, id: u64) -> bool {
        let len_before = self.validators.len();
        self.validators.retain(|v| v.id != id);
        self.validators.len() < len_before
    }

    /// Number of validators.
    pub fn len(&self) -> usize {
        self.validators.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.validators.is_empty()
    }

    /// Get a validator by id.
    pub fn get(&self, id: u64) -> Option<&ValidatorInfo> {
        self.validators.iter().find(|v| v.id == id)
    }

    /// Get a mutable reference to a validator by id.
    pub fn get_mut(&mut self, id: u64) -> Option<&mut ValidatorInfo> {
        self.validators.iter_mut().find(|v| v.id == id)
    }

    /// Get all validators.
    pub fn validators(&self) -> &[ValidatorInfo] {
        &self.validators
    }

    /// Compute total effective weight across all validators.
    pub fn total_weight(&self) -> u64 {
        self.validators.iter().map(|v| v.effective_weight()).sum()
    }

    /// Deterministic leader selection for a given epoch.
    ///
    /// Uses `hash(epoch || "leader") mod total_weight` to pick a weighted
    /// index, then iterates validators accumulating weights until the
    /// accumulated weight exceeds the index.
    pub fn leader_for_epoch(&self, epoch: u64) -> Option<&ValidatorInfo> {
        if self.validators.is_empty() {
            return None;
        }

        let total = self.total_weight();
        if total == 0 {
            // Fallback to simple round-robin if all weights are zero
            let idx = epoch as usize % self.validators.len();
            return Some(&self.validators[idx]);
        }

        let weighted_index = Self::epoch_hash(epoch) % total;
        let mut accumulated = 0u64;

        for validator in &self.validators {
            accumulated += validator.effective_weight();
            if accumulated > weighted_index {
                return Some(validator);
            }
        }

        // Should not reach here, but fallback to last validator
        self.validators.last()
    }

    /// Check if a given validator is the leader for the given epoch.
    pub fn is_leader(&self, validator_id: u64, epoch: u64) -> bool {
        self.leader_for_epoch(epoch)
            .map_or(false, |v| v.id == validator_id)
    }

    /// Update a validator's health score after it produced a block.
    ///
    /// Health score increases based on evaporations processed in the block.
    /// This incentivizes validators to run efficient evaporation, not just
    /// produce empty blocks.
    pub fn update_health_score(&mut self, validator_id: u64, evaporations_in_block: usize) {
        if let Some(v) = self.get_mut(validator_id) {
            v.blocks_produced += 1;
            v.evaporations_processed += evaporations_in_block as u64;

            // Increase health score based on evaporations processed
            let health_increase = evaporations_in_block as f64 * HEALTH_PER_EVAPORATION;
            v.health_score = (v.health_score + health_increase).min(MAX_HEALTH_SCORE);
        }
    }

    /// Apply health score decay to all validators (called once per epoch).
    /// This ensures validators must keep contributing to maintain their bonus.
    pub fn decay_health_scores(&mut self) {
        for v in &mut self.validators {
            v.health_score = (v.health_score - HEALTH_DECAY_RATE).max(0.0);
        }
    }

    /// Compute a deterministic hash for an epoch (used for leader selection).
    fn epoch_hash(epoch: u64) -> u64 {
        let mut input = Vec::with_capacity(14);
        input.extend_from_slice(&epoch.to_le_bytes());
        input.extend_from_slice(b"leader");
        let hash = blake3_hash(&input);
        // Use first 8 bytes as u64
        u64::from_le_bytes(hash[..8].try_into().unwrap())
    }
}

impl Default for ValidatorSet {
    fn default() -> Self {
        Self::new()
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
    fn test_health_score_bonus_increases_turns() {
        // Two validators with equal stake
        let mut vs = ValidatorSet::with_validators(vec![
            make_validator(1, 1000),
            make_validator(2, 1000),
        ]);

        // Count turns with no health bonus
        let mut base_counts = [0u64; 2];
        for epoch in 1..=5000 {
            let leader = vs.leader_for_epoch(epoch).unwrap().id;
            base_counts[(leader - 1) as usize] += 1;
        }

        // Give validator 1 max health score
        vs.get_mut(1).unwrap().health_score = 1.0;

        // Count turns with health bonus
        let mut bonus_counts = [0u64; 2];
        for epoch in 1..=5000 {
            let leader = vs.leader_for_epoch(epoch).unwrap().id;
            bonus_counts[(leader - 1) as usize] += 1;
        }

        // Validator 1 should get MORE turns with health bonus
        // effective_weight(1) = 1000 * 1.2 = 1200, effective_weight(2) = 1000
        // ratio should be ~1.2
        assert!(
            bonus_counts[0] > base_counts[0],
            "Health bonus should increase turns: {} vs {}",
            bonus_counts[0],
            base_counts[0]
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
}
