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

/// Minimum stake to remain a validator.
const MIN_STAKE: u64 = 100;

/// Slash penalty for equivocation (double-signing): 10% of stake.
const SLASH_EQUIVOCATION_PCT: f64 = 0.10;

/// Slash penalty for downtime (missed blocks): 1% of stake per miss.
const SLASH_DOWNTIME_PCT: f64 = 0.01;

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
    /// BLS12-381 public key for consensus attestation (48 bytes compressed).
    /// None if the validator hasn't registered a BLS key yet.
    #[serde(default)]
    pub bls_public_key: Option<Vec<u8>>,
    /// Post-quantum VRF public key (ML-DSA, 1952 bytes) for VRF-based
    /// leader election and randomness generation.
    #[serde(default)]
    pub vrf_public_key: Option<Vec<u8>>,
    /// Total blocks produced by this validator.
    pub blocks_produced: u64,
    /// Total evaporations processed across all blocks.
    pub evaporations_processed: u64,
    /// Health score (0.0–1.0) reflecting thermodynamic contribution.
    pub health_score: f64,
    /// Whether this validator has been jailed (temporarily removed from rotation).
    #[serde(default)]
    pub jailed: bool,
    /// Total stake slashed historically.
    #[serde(default)]
    pub total_slashed: u64,
    /// BLS proof-of-possession (signature over pk with POP DST).
    /// Prevents rogue-key attack on aggregate signatures.
    #[serde(default)]
    pub bls_pop: Option<Vec<u8>>,
    /// Whether the BLS proof-of-possession has been verified.
    #[serde(default)]
    pub pop_verified: bool,
}

impl ValidatorInfo {
    /// Create a new validator with the given id, stake, and address.
    pub fn new(id: u64, stake: u64, address: [u8; 32]) -> Self {
        Self {
            id,
            stake,
            address,
            bls_public_key: None,
            vrf_public_key: None,
            blocks_produced: 0,
            evaporations_processed: 0,
            health_score: 0.0,
            jailed: false,
            total_slashed: 0,
            bls_pop: None,
            pop_verified: false,
        }
    }

    /// Create a validator with a BLS public key and proof-of-possession.
    pub fn with_bls_key(id: u64, stake: u64, address: [u8; 32], bls_pk: Vec<u8>) -> Self {
        Self {
            bls_public_key: Some(bls_pk),
            ..Self::new(id, stake, address)
        }
    }

    /// Create a validator with a BLS key + verified proof-of-possession.
    pub fn with_bls_pop(
        id: u64,
        stake: u64,
        address: [u8; 32],
        bls_pk: Vec<u8>,
        pop: Vec<u8>,
    ) -> Self {
        Self {
            bls_public_key: Some(bls_pk),
            bls_pop: Some(pop),
            pop_verified: false, // Caller must verify via ValidatorSet::verify_pop
            ..Self::new(id, stake, address)
        }
    }

    /// Create a validator with both BLS and VRF public keys.
    pub fn with_keys(
        id: u64,
        stake: u64,
        address: [u8; 32],
        bls_pk: Option<Vec<u8>>,
        vrf_pk: Option<Vec<u8>>,
    ) -> Self {
        Self {
            bls_public_key: bls_pk,
            vrf_public_key: vrf_pk,
            ..Self::new(id, stake, address)
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    /// If the validator has a BLS key, proof-of-possession must be verified
    /// before calling this method. Use `add_validator_with_pop` for BLS-keyed
    /// validators.
    pub fn add_validator(&mut self, info: ValidatorInfo) {
        // Don't add duplicates
        if !self.validators.iter().any(|v| v.id == info.id) {
            self.validators.push(info);
        }
    }

    /// Add a validator with BLS proof-of-possession verification.
    ///
    /// The proof-of-possession is: BLS.Sign(secret_key, public_key_bytes).
    /// This prevents rogue-key attacks where an attacker crafts a public key
    /// that cancels out honest validators' keys in aggregate signatures.
    ///
    /// Returns false if:
    /// - No BLS key is provided
    /// - The proof-of-possession is invalid
    /// - The validator already exists
    pub fn add_validator_with_pop(
        &mut self,
        info: ValidatorInfo,
        proof_of_possession: &[u8],
    ) -> bool {
        // Check for duplicates
        if self.validators.iter().any(|v| v.id == info.id) {
            return false;
        }

        // Require BLS key
        let bls_pk_bytes = match &info.bls_public_key {
            Some(pk) if !pk.is_empty() => pk,
            _ => return false,
        };

        // Verify proof-of-possession: sig = BLS.Sign(sk, pk_bytes, DST=POP)
        // The message being signed IS the public key itself.
        if !Self::verify_bls_pop(bls_pk_bytes, proof_of_possession) {
            return false;
        }

        let mut validated = info;
        validated.bls_pop = Some(proof_of_possession.to_vec());
        validated.pop_verified = true;
        self.validators.push(validated);
        true
    }

    /// Verify a BLS proof-of-possession using real BLS12-381 signature
    /// verification with the POP domain separation tag.
    /// PoP = BLS.Sign(sk, pk_bytes, DST=BLS_POP_DST)
    /// Verify: BLS.Verify(pk, pk_bytes, pop_sig, DST=BLS_POP_DST)
    fn verify_bls_pop(public_key_bytes: &[u8], pop_signature: &[u8]) -> bool {
        use evaporchain_crypto::signatures::{BlsPublicKey, BlsSignature, BlsVerifier};

        let pk = BlsPublicKey(public_key_bytes.to_vec());
        let pop = BlsSignature(pop_signature.to_vec());
        BlsVerifier::verify_proof_of_possession(&pk, &pop)
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

    /// Compute total effective weight across all active (non-jailed) validators.
    pub fn total_weight(&self) -> u64 {
        self.validators.iter()
            .filter(|v| !v.jailed)
            .map(|v| v.effective_weight())
            .sum()
    }

    /// Deterministic leader selection for a given epoch.
    /// Jailed validators are excluded from leader rotation.
    ///
    /// Uses `hash(epoch || "leader") mod total_weight` to pick a weighted
    /// index, then iterates active validators accumulating weights until the
    /// accumulated weight exceeds the index.
    pub fn leader_for_epoch(&self, epoch: u64) -> Option<&ValidatorInfo> {
        let active: Vec<&ValidatorInfo> = self.validators.iter().filter(|v| !v.jailed).collect();
        if active.is_empty() {
            return None;
        }

        // Use base stake (NOT health-weighted effective_weight) for leader selection.
        // Health scores can diverge by 1-2 blocks between nodes, causing different
        // nodes to compute different proposers — breaking consensus.
        let total: u64 = active.iter().map(|v| v.stake).sum();
        if total == 0 {
            let idx = epoch as usize % active.len();
            return Some(active[idx]);
        }

        let weighted_index = Self::epoch_hash(epoch) % total;
        let mut accumulated = 0u64;

        for validator in &active {
            accumulated += validator.stake;
            if accumulated > weighted_index {
                return Some(validator);
            }
        }

        // Should not reach here, but fallback to last active validator
        active.last().copied()
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

    /// Get the number of active (non-jailed) validators.
    pub fn active_count(&self) -> usize {
        self.validators.iter().filter(|v| !v.jailed).count()
    }

    // ─────────────────── Slashing ────────────────────────────────────────

    /// Slash a validator for equivocation (double-signing).
    /// Removes 10% of stake and jails the validator.
    /// Returns the amount slashed.
    pub fn slash_equivocation(&mut self, validator_id: u64) -> u64 {
        if let Some(v) = self.get_mut(validator_id) {
            let penalty = (v.stake as f64 * SLASH_EQUIVOCATION_PCT).round() as u64;
            v.stake = v.stake.saturating_sub(penalty);
            v.total_slashed += penalty;
            v.jailed = true;
            v.health_score = 0.0;
            // Auto-remove if stake below minimum
            if v.stake < MIN_STAKE {
                self.remove_validator(validator_id);
            }
            penalty
        } else {
            0
        }
    }

    /// Slash a validator for downtime (missed block production).
    /// Removes 1% of stake per missed block. Jails after 3+ misses.
    /// Returns the amount slashed.
    pub fn slash_downtime(&mut self, validator_id: u64, missed_blocks: u64) -> u64 {
        if let Some(v) = self.get_mut(validator_id) {
            let per_miss = (v.stake as f64 * SLASH_DOWNTIME_PCT).round() as u64;
            let penalty = per_miss.saturating_mul(missed_blocks);
            v.stake = v.stake.saturating_sub(penalty);
            v.total_slashed += penalty;
            if missed_blocks >= 3 {
                v.jailed = true;
            }
            v.health_score = (v.health_score - missed_blocks as f64 * 0.1).max(0.0);
            // Auto-remove if stake below minimum
            if v.stake < MIN_STAKE {
                self.remove_validator(validator_id);
            }
            penalty
        } else {
            0
        }
    }

    /// Unjail a validator (allow them back into rotation).
    pub fn unjail(&mut self, validator_id: u64) -> bool {
        if let Some(v) = self.get_mut(validator_id) {
            if v.jailed && v.stake >= MIN_STAKE {
                v.jailed = false;
                return true;
            }
        }
        false
    }

    // ─────────────────── VRF-Based Leader Election ──────────────────────────

    /// Verify that a block's VRF proof is valid for the claimed proposer.
    ///
    /// Checks:
    /// 1. The proposer has a registered VRF public key
    /// 2. The VRF proof verifies against `leader_vrf_input(height, round)`
    /// 3. The VRF output matches the proof
    pub fn verify_vrf_proposal(
        &self,
        proposer_id: u64,
        height: u64,
        round: u32,
        vrf_output: &[u8; 32],
        vrf_proof: &[u8],
    ) -> bool {
        let validator = match self.get(proposer_id) {
            Some(v) => v,
            None => return false,
        };

        let vrf_pk = match &validator.vrf_public_key {
            Some(pk) => pk,
            None => return false,
        };

        let alpha = evaporchain_crypto::vrf::leader_vrf_input(height, round);
        evaporchain_crypto::vrf::vrf_verify(
            vrf_pk,
            &alpha,
            &evaporchain_crypto::vrf::VrfOutput(*vrf_output),
            &evaporchain_crypto::vrf::VrfProof(vrf_proof.to_vec()),
        )
    }

    /// Check if a validator's VRF output qualifies them as leader.
    /// Uses stake-weighted threshold: probability proportional to stake.
    pub fn vrf_leader_qualifies(
        &self,
        validator_id: u64,
        vrf_output: &[u8; 32],
    ) -> bool {
        let validator = match self.get(validator_id) {
            Some(v) if !v.jailed => v,
            _ => return false,
        };
        let total = self.total_stake();
        evaporchain_crypto::vrf::vrf_leader_check(
            &evaporchain_crypto::vrf::VrfOutput(*vrf_output),
            validator.stake,
            total,
        )
    }

    /// Compute committee seats for a validator using VRF sortition.
    pub fn vrf_sortition(
        &self,
        validator_id: u64,
        vrf_output: &[u8; 32],
        expected_committee_size: u64,
    ) -> u64 {
        let validator = match self.get(validator_id) {
            Some(v) if !v.jailed => v,
            _ => return 0,
        };
        let total = self.total_stake();
        evaporchain_crypto::vrf::sortition(
            &evaporchain_crypto::vrf::VrfOutput(*vrf_output),
            validator.stake,
            total,
            expected_committee_size,
        )
    }

    /// Total raw stake across all active (non-jailed) validators.
    pub fn total_stake(&self) -> u64 {
        self.validators.iter()
            .filter(|v| !v.jailed)
            .map(|v| v.stake)
            .sum()
    }

    /// Get a validator by ID.
    pub fn get_validator(&self, id: u64) -> Option<&ValidatorInfo> {
        self.validators.iter().find(|v| v.id == id)
    }

    /// Check if any validator has a VRF key registered (enables VRF mode).
    pub fn has_vrf_keys(&self) -> bool {
        self.validators.iter().any(|v| v.vrf_public_key.is_some())
    }

    pub fn leader_for_epoch_with_seed(
        &self,
        epoch: u64,
        beacon_seed: &[u8; 32],
    ) -> Option<&ValidatorInfo> {
        let active: Vec<&ValidatorInfo> = self.validators.iter().filter(|v| !v.jailed).collect();
        if active.is_empty() {
            return None;
        }
        let total: u64 = active.iter().map(|v| v.stake).sum();
        if total == 0 {
            let idx = epoch as usize % active.len();
            return Some(active[idx]);
        }
        let weighted_index = Self::epoch_hash_with_seed(epoch, beacon_seed) % total;
        let mut accumulated = 0u64;
        for validator in &active {
            accumulated += validator.stake;
            if accumulated > weighted_index {
                return Some(validator);
            }
        }
        active.last().copied()
    }

    /// Compute a deterministic hash for an epoch (used for leader selection).
    fn epoch_hash(epoch: u64) -> u64 {
        Self::epoch_hash_with_seed(epoch, &[0u8; 32])
    }

    fn epoch_hash_with_seed(epoch: u64, seed: &[u8; 32]) -> u64 {
        let mut input = Vec::with_capacity(46);
        input.extend_from_slice(&epoch.to_le_bytes());
        input.extend_from_slice(b"leader");
        input.extend_from_slice(seed);
        let hash = blake3_hash(&input);
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&hash[..8]);
        u64::from_le_bytes(buf)
    }
}

impl Default for ValidatorSet {
    fn default() -> Self {
        Self::new()
    }
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
        height > 0 && height % EPOCH_LENGTH == 0
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

        let max_churn = ((validator_set.active_count() as f64) * MAX_CHURN_FRACTION).ceil() as usize;
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
            if validator_set.validators().iter().any(|v| v.id == pj.info.id) {
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
                result.applied.push(format!("Validator {} left", pl.validator_id));
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
        let mut vs = ValidatorSet::with_validators(vec![
            make_validator(1, 1000),
            make_validator(2, 1000),
        ]);

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
    fn test_slash_equivocation_removes_if_below_min() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 50, [1u8; 32])); // Below MIN_STAKE after slash
        let slashed = vs.slash_equivocation(1);
        assert_eq!(slashed, 5); // 10% of 50
        assert!(vs.get(1).is_none()); // Removed because 45 < MIN_STAKE (100)
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
            assert_ne!(leader.id, 1, "Jailed validator 1 should not be leader at epoch {}", epoch);
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
        assert!(can_lead, "Unjailed validator should participate in rotation");
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
        assert!(total_after < total_before, "Jailed validator weight should be excluded");
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
        mgr.queue_change(
            ValidatorSetChange::Join(make_validator(5, 1000)),
            5,
        );
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
            mgr.queue_change(
                ValidatorSetChange::Join(make_validator(id, 1000)),
                0,
            );
        }

        let result = mgr.apply_epoch_transition(&mut vs, 2);
        // Should apply at most 2 joins, defer the rest
        assert!(vs.len() <= 6, "Max churn should limit joins");
        assert!(!result.deferred.is_empty(), "Excess joins should be deferred");
    }

    #[test]
    fn test_duplicate_join_rejected() {
        let mut vs = make_validator_set(4, 1000);
        let mut mgr = EpochTransitionManager::new();

        // Try to join with id=1 which already exists
        mgr.queue_change(
            ValidatorSetChange::Join(make_validator(1, 2000)),
            0,
        );

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
        assert!(result.applied.len() >= 2, "Stake update + at least one more: {:?}", result);
    }
}
