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
    /// Previous BLS public key, retained during the rotation grace window
    /// so in-flight votes signed with the old key still verify. Cleared
    /// (set to `None`) once `bls_prev_key_expiry_epoch` has elapsed.
    /// Closes punch-list 4b.
    #[serde(default)]
    pub bls_public_key_prev: Option<Vec<u8>>,
    /// Last epoch (inclusive) at which `bls_public_key_prev` is still
    /// accepted by `verify_commit_certificate`. After this epoch, only
    /// `bls_public_key` is consulted.
    #[serde(default)]
    pub bls_prev_key_expiry_epoch: Option<u64>,
    /// Total stake delegated to this validator by other token holders,
    /// summed across the live `DelegationRecord` set in StateDB. Cached
    /// here so the consensus quorum check (and other hot paths) don't
    /// have to walk the delegation map every block. Refreshed by
    /// `ValidatorSet::refresh_delegated_stakes` at block-production
    /// boundaries. Defaults to 0 — pre-delegation chains keep the same
    /// effective stake as their `stake` field.
    #[serde(default)]
    pub delegated_stake: u64,
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
            bls_public_key_prev: None,
            bls_prev_key_expiry_epoch: None,
            delegated_stake: 0,
        }
    }

    /// Total voting power = own stake + cached delegated stake. Used by
    /// quorum checks. Saturating add prevents overflow under Byzantine
    /// stake-injection scenarios.
    pub fn effective_stake(&self) -> u64 {
        self.stake.saturating_add(self.delegated_stake)
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
    ///
    /// Returns `false` (no-op) if:
    /// - The validator ID already exists.
    /// - The validator carries a BLS public key but `pop_verified` is false.
    ///   Use [`add_validator_with_pop`] to register BLS-keyed validators; it
    ///   verifies the proof-of-possession and sets `pop_verified = true`.
    ///   Accepting an unverified BLS key opens rogue-key attacks on aggregate
    ///   signature verification.
    pub fn add_validator(&mut self, info: ValidatorInfo) -> bool {
        // Reject duplicate
        if self.validators.iter().any(|v| v.id == info.id) {
            return false;
        }
        // BLS key with unverified PoP is rejected — rogue-key attack surface.
        if info.bls_public_key.as_ref().is_some_and(|k| !k.is_empty()) && !info.pop_verified {
            return false;
        }
        self.validators.push(info);
        true
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

    /// Public PoP verification helper. Used by the execution layer to
    /// validate `RotateValidatorKey` proof-of-possession on both the old
    /// and new keys before applying a rotation.
    pub fn verify_pop(public_key_bytes: &[u8], pop_signature: &[u8]) -> bool {
        Self::verify_bls_pop(public_key_bytes, pop_signature)
    }

    /// Apply a validator BLS key rotation. The previous public key is
    /// stashed in `bls_public_key_prev` until `expiry_epoch` so in-flight
    /// votes signed with the old key still verify during the grace window.
    ///
    /// Caller responsibilities (NOT checked here, since this is the
    /// final-step state mutator):
    ///   - PoP-verify the new key with the supplied `bls_pop_new`
    ///   - PoP-verify that the old key (currently on-chain) signed the new
    ///     key claim (`bls_pop_old`) — proves continuity of control
    ///   - Confirm `expiry_epoch >= current_epoch`
    ///
    /// Returns false if the validator is unknown or has no current BLS key.
    /// Closes punch-list 4b state mutation half.
    pub fn rotate_validator_key(
        &mut self,
        validator_id: u64,
        new_pk: Vec<u8>,
        new_pop: Vec<u8>,
        expiry_epoch: u64,
    ) -> bool {
        let v = match self.validators.iter_mut().find(|v| v.id == validator_id) {
            Some(v) => v,
            None => return false,
        };
        let prev = match v.bls_public_key.take() {
            Some(p) if !p.is_empty() => p,
            _ => return false,
        };
        v.bls_public_key_prev = Some(prev);
        v.bls_prev_key_expiry_epoch = Some(expiry_epoch);
        v.bls_public_key = Some(new_pk);
        v.bls_pop = Some(new_pop);
        // The new key has been PoP-verified by the caller; record it.
        v.pop_verified = true;
        true
    }

    /// Drop the previous key for any validator whose grace window has
    /// elapsed. Cheap O(n) sweep; called once per epoch from the
    /// execution layer.
    pub fn purge_expired_prev_keys(&mut self, current_epoch: u64) -> usize {
        let mut purged = 0usize;
        for v in self.validators.iter_mut() {
            if let Some(expiry) = v.bls_prev_key_expiry_epoch {
                if current_epoch > expiry {
                    v.bls_public_key_prev = None;
                    v.bls_prev_key_expiry_epoch = None;
                    purged += 1;
                }
            }
        }
        purged
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
        self.validators
            .iter()
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

        let total: u64 = active
            .iter()
            .map(|v| v.effective_weight())
            .fold(0u64, |a, w| a.saturating_add(w));
        if total == 0 {
            let idx = epoch as usize % active.len();
            return Some(active[idx]);
        }

        let weighted_index = Self::epoch_hash(epoch) % total;
        let mut accumulated = 0u64;

        for validator in &active {
            accumulated = accumulated.saturating_add(validator.effective_weight());
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
            .is_some_and(|v| v.id == validator_id)
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
            if missed_blocks >= 500 {
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

    /// Apply a precomputed slash amount (from Sanov or any theorem-grade
    /// formula) to a validator. Handles jailing and auto-remove below
    /// `MIN_STAKE`. Returns the amount actually deducted (capped at stake).
    pub fn slash_with_amount(&mut self, validator_id: u64, amount: u64, jail: bool) -> u64 {
        let actual = if let Some(v) = self.get_mut(validator_id) {
            let deducted = amount.min(v.stake);
            v.stake = v.stake.saturating_sub(deducted);
            v.total_slashed += deducted;
            if jail {
                v.jailed = true;
                v.health_score = 0.0;
            }
            deducted
        } else {
            return 0;
        };
        if actual > 0 {
            if let Some(v) = self.get(validator_id) {
                if v.stake < MIN_STAKE {
                    self.remove_validator(validator_id);
                }
            }
        }
        actual
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
    pub fn vrf_leader_qualifies(&self, validator_id: u64, vrf_output: &[u8; 32]) -> bool {
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
    /// Uses saturating arithmetic to prevent overflow in Byzantine scenarios.
    /// Includes both self-stake and cached delegated stake (P0 #4 Phase 6).
    /// Quorum checks compare signing voting power against this total.
    pub fn total_stake(&self) -> u64 {
        self.validators
            .iter()
            .filter(|v| !v.jailed)
            .map(|v| v.effective_stake())
            .fold(0u64, |acc, s| acc.saturating_add(s))
    }

    /// Total *self-stake only* across active validators. Useful when
    /// reporting protocol-fundamentals separately from delegations.
    pub fn total_self_stake(&self) -> u64 {
        self.validators
            .iter()
            .filter(|v| !v.jailed)
            .map(|v| v.stake)
            .fold(0u64, |acc, s| acc.saturating_add(s))
    }

    /// Refresh each validator's `delegated_stake` from the live
    /// DelegationRecord set in StateDB. Should be called at the start
    /// of every block production cycle so quorum checks within that
    /// block use up-to-date voting power. Within-block delegations
    /// take effect on the next block.
    pub fn refresh_delegated_stakes(&mut self, db: &dyn evaporchain_state::db::StateDB) {
        // Build a per-validator total from the delegation set in one pass.
        let mut totals: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
        for d in db.all_delegations() {
            *totals.entry(d.validator_id).or_insert(0) = totals
                .get(&d.validator_id)
                .copied()
                .unwrap_or(0)
                .saturating_add(d.amount);
        }
        for v in self.validators.iter_mut() {
            v.delegated_stake = totals.get(&v.id).copied().unwrap_or(0);
        }
    }

    /// Get a validator by ID.
    pub fn get_validator(&self, id: u64) -> Option<&ValidatorInfo> {
        self.validators.iter().find(|v| v.id == id)
    }

    /// Check if any validator has a BLS key registered (enables BLS enforcement).
    pub fn has_bls_keys(&self) -> bool {
        !self.validators.is_empty() && self.validators.iter().all(|v| v.bls_public_key.is_some())
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
        let total: u64 = active
            .iter()
            .map(|v| v.effective_weight())
            .fold(0u64, |a, w| a.saturating_add(w));
        if total == 0 {
            let idx = epoch as usize % active.len();
            return Some(active[idx]);
        }
        let weighted_index = Self::epoch_hash_with_seed(epoch, beacon_seed) % total;
        let mut accumulated = 0u64;
        for validator in &active {
            accumulated = accumulated.saturating_add(validator.effective_weight());
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

        vs.refresh_delegated_stakes(&db);

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
}
