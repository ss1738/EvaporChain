//! Protocol types for EvaporChain BFT consensus.
//!
//! Extracted from `evaporchain-consensus` 2026-05-08 to break the
//! Light Client SDK's transitive dep on `evaporchain-state` (which
//! pulls RocksDB and breaks `wasm32-unknown-unknown` builds — see
//! `crates/evaporchain-light-client-wasm/README.md` for the full
//! diagnosis).
//!
//! ## Status
//!
//! Phase 1 (scaffold): complete (commit 46bfdd4).
//! Phase 2-4 (type movements): in progress — see commit log for
//! the order in which types land here.
//! Phase 5 (SDK dep switch): not yet started.
//!
//! After all phases, `evaporchain-light-client` drops its
//! `evaporchain-consensus` dep entirely and the WASM scaffold at
//! `crates/evaporchain-light-client-wasm/` builds (modulo Refactor B
//! for the BLS backend abstraction).
//!
//! ## What lives here vs in `evaporchain-consensus`
//!
//! Here:
//!   * Pure type definitions (`ValidatorInfo`, `ValidatorSet`,
//!     `LightBlockHeader`, `TrustedState`, `VerificationResult`,
//!     `LightClientError`).
//!   * The BLS-using verifier (`LightClientVerifier`) — uses
//!     `evaporchain-crypto`, no state-DB dep.
//!   * Read-only / pure-type methods on those types.
//!   * Constants used by the types (e.g. `HEALTH_BONUS_CAP`).
//!
//! Stays in `evaporchain-consensus`:
//!   * Tendermint consensus loop, mempool, fork-choice.
//!   * Slashing logic + slashing constants.
//!   * Epoch-transition manager.
//!   * `ValidatorSetSource` trait + state-DB-attached impls.
//!   * Anything that touches `evaporchain-state`.

use evaporchain_crypto::hash::blake3_hash;
use serde::{Deserialize, Serialize};

// ─────────────────────── Constants ────────────────────────────────────

/// Maximum health-score bonus applied to a validator's stake when
/// computing leader-selection weight. Caps at +20% so even a perfectly
/// healthy validator never gets more than 1.2× their raw stake weight.
pub const HEALTH_BONUS_CAP: f64 = 0.2;

/// Maximum health-score value. Used to clamp `ValidatorInfo.health_score`
/// before computing effective weight (defends against >1.0 scores from
/// arithmetic drift).
pub const MAX_HEALTH_SCORE: f64 = 1.0;

/// Health score decay per epoch (small decay to keep validators active).
pub const HEALTH_DECAY_RATE: f64 = 0.01;

/// Health score increment per evaporation processed.
pub const HEALTH_PER_EVAPORATION: f64 = 0.05;

/// Minimum stake to remain a validator. Below this, validators are
/// auto-removed (after slashing or natural attrition).
pub const MIN_STAKE: u64 = 100;

/// Slash penalty for equivocation (double-signing): 10% of stake.
pub const SLASH_EQUIVOCATION_PCT: f64 = 0.10;

/// Slash penalty for downtime (missed blocks): 1% of stake per miss.
pub const SLASH_DOWNTIME_PCT: f64 = 0.01;

// ─────────────────────── ValidatorInfo ────────────────────────────────

/// Information about a registered validator.
///
/// Moved from `evaporchain-consensus/src/validator_set.rs` 2026-05-08
/// (Phase 3a of the wasm32-unblock refactor). All previous callers
/// continue to import via `evaporchain_consensus::validator_set::ValidatorInfo`
/// — the consensus crate re-exports from here to preserve API stability.
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

// ─────────────────────── ValidatorSet ─────────────────────────────────

/// Set of validators with energy-weighted leader selection.
///
/// Moved from `evaporchain-consensus/src/validator_set.rs` 2026-05-08
/// (Phase 3b of the wasm32-unblock refactor). The single method that
/// depends on `evaporchain-state` (`refresh_delegated_stakes`) was
/// extracted as a free function and stays in `evaporchain-consensus`,
/// keeping this crate state-DB-free.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Add a validator to the set. Returns `true` if added, `false`
    /// if a validator with the same id already exists.
    pub fn add_validator(&mut self, info: ValidatorInfo) -> bool {
        if self.get(info.id).is_some() {
            return false;
        }
        self.validators.push(info);
        true
    }

    /// Add a validator with a BLS proof-of-possession. The PoP MUST be
    /// pre-verified by the caller (e.g. via `Self::verify_pop`).
    pub fn add_validator_with_pop(
        &mut self,
        mut info: ValidatorInfo,
        pop: Vec<u8>,
        pop_verified: bool,
    ) -> bool {
        info.bls_pop = Some(pop);
        info.pop_verified = pop_verified;
        self.add_validator(info)
    }

    /// Verify a BLS proof-of-possession (pk, signature) pair.
    /// Defends against rogue-key attacks on aggregate signatures.
    fn verify_bls_pop(public_key_bytes: &[u8], pop_signature: &[u8]) -> bool {
        use evaporchain_crypto::signatures::{BlsPublicKey, BlsSignature, BlsVerifier};
        let pk = BlsPublicKey(public_key_bytes.to_vec());
        let pop = BlsSignature(pop_signature.to_vec());
        BlsVerifier::verify_proof_of_possession(&pk, &pop)
    }

    /// Public PoP-verify entry point — see [`Self::verify_bls_pop`].
    pub fn verify_pop(public_key_bytes: &[u8], pop_signature: &[u8]) -> bool {
        Self::verify_bls_pop(public_key_bytes, pop_signature)
    }

    /// Rotate a validator's BLS key. Old key kept for the grace
    /// window so in-flight votes still verify.
    /// Caller MUST have PoP-verified `new_pk` and `new_pop` before
    /// calling — this is the final-step state mutator only.
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
    /// CRITICAL DETERMINISM INVARIANT: leader selection uses base `stake`,
    /// NOT health-weighted `effective_weight`. Different nodes may compute
    /// slightly different health_score values (timing of health updates is
    /// not strictly synchronised), and any divergence here would cause
    /// different nodes to pick different leaders — breaking liveness.
    /// effective_weight is reserved for non-consensus-critical paths.
    pub fn leader_for_epoch(&self, epoch: u64) -> Option<&ValidatorInfo> {
        let active: Vec<&ValidatorInfo> = self.validators.iter().filter(|v| !v.jailed).collect();
        if active.is_empty() {
            return None;
        }

        let total: u64 = active
            .iter()
            .map(|v| v.stake)
            .fold(0u64, |a, w| a.saturating_add(w));
        if total == 0 {
            let idx = epoch as usize % active.len();
            return Some(active[idx]);
        }

        let weighted_index = Self::epoch_hash(epoch) % total;
        let mut accumulated = 0u64;

        for validator in &active {
            accumulated = accumulated.saturating_add(validator.stake);
            if accumulated > weighted_index {
                return Some(validator);
            }
        }

        active.last().copied()
    }

    /// Check if a given validator is the leader for the given epoch.
    pub fn is_leader(&self, validator_id: u64, epoch: u64) -> bool {
        self.leader_for_epoch(epoch)
            .is_some_and(|v| v.id == validator_id)
    }

    /// Update a validator's health score after it produced a block.
    pub fn update_health_score(&mut self, validator_id: u64, evaporations_in_block: usize) {
        if let Some(v) = self.get_mut(validator_id) {
            v.blocks_produced += 1;
            v.evaporations_processed += evaporations_in_block as u64;
            let health_increase = evaporations_in_block as f64 * HEALTH_PER_EVAPORATION;
            v.health_score = (v.health_score + health_increase).min(MAX_HEALTH_SCORE);
        }
    }

    /// Apply health score decay to all validators (called once per epoch).
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
    pub fn slash_equivocation(&mut self, validator_id: u64) -> u64 {
        if let Some(v) = self.get_mut(validator_id) {
            let penalty = (v.stake as f64 * SLASH_EQUIVOCATION_PCT).round() as u64;
            v.stake = v.stake.saturating_sub(penalty);
            v.total_slashed += penalty;
            v.jailed = true;
            v.health_score = 0.0;
            if v.stake < MIN_STAKE {
                self.remove_validator(validator_id);
            }
            penalty
        } else {
            0
        }
    }

    /// Slash a validator for downtime (missed block production).
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
            if v.stake < MIN_STAKE {
                self.remove_validator(validator_id);
            }
            penalty
        } else {
            0
        }
    }

    /// Apply a precomputed slash amount to a validator.
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

    /// Jail every active validator whose address appears in
    /// `tombstone_addresses`. Doctrine: per
    /// `evaporchain-tombstone::EulogyTrie`, "the chain's death is
    /// final" — once an account is memorialised in the eulogy trie,
    /// its validator must not appear in leader rotation. Already-
    /// jailed validators are not double-counted. Returns the number
    /// newly jailed.
    ///
    /// Idempotent: safe to call repeatedly with the same address set.
    /// O(n_validators × n_tombstones), which is bounded by the
    /// validator-set cap and the eulogy-trie's append-only growth
    /// rate (one tombstone per zero-balance event).
    pub fn jail_tombstoned_by_address(&mut self, tombstone_addresses: &[[u8; 32]]) -> usize {
        if tombstone_addresses.is_empty() {
            return 0;
        }
        let mut newly_jailed = 0;
        for v in self.validators.iter_mut() {
            if !v.jailed && tombstone_addresses.contains(&v.address) {
                v.jailed = true;
                v.health_score = 0.0;
                newly_jailed += 1;
            }
        }
        newly_jailed
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
    pub fn total_stake(&self) -> u64 {
        self.validators
            .iter()
            .filter(|v| !v.jailed)
            .map(|v| v.effective_stake())
            .fold(0u64, |acc, s| acc.saturating_add(s))
    }

    /// Total *self-stake only* across active validators.
    pub fn total_self_stake(&self) -> u64 {
        self.validators
            .iter()
            .filter(|v| !v.jailed)
            .map(|v| v.stake)
            .fold(0u64, |acc, s| acc.saturating_add(s))
    }

    /// Get a validator by ID.
    pub fn get_validator(&self, id: u64) -> Option<&ValidatorInfo> {
        self.validators.iter().find(|v| v.id == id)
    }

    /// Check if any validator has a BLS key registered.
    pub fn has_bls_keys(&self) -> bool {
        !self.validators.is_empty() && self.validators.iter().all(|v| v.bls_public_key.is_some())
    }

    /// Check if any validator has a VRF key registered.
    pub fn has_vrf_keys(&self) -> bool {
        self.validators.iter().any(|v| v.vrf_public_key.is_some())
    }

    /// Leader selection with a beacon seed (used post-genesis).
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
            .map(|v| v.stake)
            .fold(0u64, |a, w| a.saturating_add(w));
        if total == 0 {
            let idx = epoch as usize % active.len();
            return Some(active[idx]);
        }
        let weighted_index = Self::epoch_hash_with_seed(epoch, beacon_seed) % total;
        let mut accumulated = 0u64;
        for validator in &active {
            accumulated = accumulated.saturating_add(validator.stake);
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

// ─────────────────────── Light Client Types ──────────────────────────
//
// Phase 2 + 4 (2026-05-08): the light-client surface — header types,
// trust state, error/result enums, constants, and the BLS-using
// verifier — moved from `evaporchain-consensus/src/light_client.rs`.

/// A light block header — the minimal data needed to verify consensus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightBlockHeader {
    pub height: u64,
    pub epoch: u64,
    pub block_hash: [u8; 32],
    pub parent_hash: [u8; 32],
    pub state_root: [u8; 32],
    pub timestamp: u64,
    /// The validator set that signed this block.
    pub validator_set: ValidatorSet,
    /// BLS commit certificate proving 2/3+ attestation.
    pub commit_certificate: evaporchain_types::CommitCertificate,
}

/// Trusted state stored by the light client.
#[derive(Debug, Clone)]
pub struct TrustedState {
    pub header: LightBlockHeader,
    pub trust_expires_at: u64,
}

/// Result of light client verification.
#[derive(Debug, Clone, PartialEq)]
pub enum VerificationResult {
    Valid,
    Invalid(String),
    NeedBisection {
        trusted_height: u64,
        target_height: u64,
    },
}

/// Error from the light client.
#[derive(Debug, Clone)]
pub enum LightClientError {
    NoTrustedState,
    ExpiredTrustPeriod,
    InsufficientValidatorOverlap,
    InvalidCertificate(String),
    HeightMismatch,
}

/// Trust period in seconds. Default: 2 weeks.
pub const TRUST_PERIOD_SECS: u64 = 14 * 24 * 3600;

/// Trust threshold numerator (1/3 = 33% stake overlap required for skip).
pub const TRUST_THRESHOLD_NUMERATOR: u64 = 1;

/// Trust threshold denominator.
pub const TRUST_THRESHOLD_DENOMINATOR: u64 = 3;

/// Maximum height gap for skip verification without bisection.
pub const MAX_SKIP_HEIGHT_GAP: u64 = 10_000;

// ─────────────────────── LightClientVerifier ─────────────────────────

/// Verifies block headers using commit certificates and validator
/// set tracking. BFT BLS aggregate-sig verification, trust-period
/// tracking, sequential + skip verification modes per ICS-007.
pub struct LightClientVerifier {
    trusted_states: std::collections::BTreeMap<u64, TrustedState>,
    trust_period: u64,
}

impl LightClientVerifier {
    /// Create a new verifier with a genesis trusted state.
    pub fn new(genesis_header: LightBlockHeader, current_time: u64) -> Self {
        let height = genesis_header.height;
        let mut trusted_states = std::collections::BTreeMap::new();
        trusted_states.insert(
            height,
            TrustedState {
                header: genesis_header,
                trust_expires_at: current_time + TRUST_PERIOD_SECS,
            },
        );
        Self {
            trusted_states,
            trust_period: TRUST_PERIOD_SECS,
        }
    }

    /// Create with a custom trust period (useful for testing).
    pub fn with_trust_period(
        genesis_header: LightBlockHeader,
        current_time: u64,
        trust_period: u64,
    ) -> Self {
        let height = genesis_header.height;
        let mut trusted_states = std::collections::BTreeMap::new();
        trusted_states.insert(
            height,
            TrustedState {
                header: genesis_header,
                trust_expires_at: current_time + trust_period,
            },
        );
        Self {
            trusted_states,
            trust_period,
        }
    }

    pub fn latest_trusted_height(&self) -> Option<u64> {
        self.trusted_states.keys().next_back().copied()
    }

    pub fn trusted_state_at(&self, height: u64) -> Option<&TrustedState> {
        self.trusted_states.get(&height)
    }

    fn best_trusted_state_for(&self, target_height: u64) -> Option<&TrustedState> {
        self.trusted_states
            .range(..=target_height)
            .next_back()
            .map(|(_, ts)| ts)
    }

    /// Verify an untrusted header against trusted state.
    pub fn verify(
        &mut self,
        untrusted: &LightBlockHeader,
        current_time: u64,
    ) -> VerificationResult {
        let trusted = match self.best_trusted_state_for(untrusted.height.saturating_sub(1)) {
            Some(ts) => ts.clone(),
            None => return VerificationResult::Invalid("No trusted state found".into()),
        };

        if current_time > trusted.trust_expires_at {
            return VerificationResult::Invalid(
                "Trust period expired — need fresh checkpoint".into(),
            );
        }

        match self.verify_commit_certificate(untrusted) {
            Ok(()) => {}
            Err(e) => return VerificationResult::Invalid(format!("Invalid certificate: {}", e)),
        }

        let height_gap = untrusted.height.saturating_sub(trusted.header.height);
        if height_gap == 1 {
            self.add_trusted(untrusted.clone(), current_time);
            return VerificationResult::Valid;
        }

        if height_gap > MAX_SKIP_HEIGHT_GAP {
            return VerificationResult::NeedBisection {
                trusted_height: trusted.header.height,
                target_height: untrusted.height,
            };
        }

        match self.check_validator_overlap(&trusted.header, untrusted) {
            Ok(()) => {
                self.add_trusted(untrusted.clone(), current_time);
                VerificationResult::Valid
            }
            Err(_) => VerificationResult::NeedBisection {
                trusted_height: trusted.header.height,
                target_height: untrusted.height,
            },
        }
    }

    fn verify_commit_certificate(&self, header: &LightBlockHeader) -> Result<(), String> {
        use evaporchain_crypto::signatures::{BlsPublicKey, BlsSignature, BlsVerifier};

        let cert = &header.commit_certificate;

        if cert.height != header.height {
            return Err(format!(
                "Certificate height {} != header height {}",
                cert.height, header.height
            ));
        }
        if cert.block_hash != header.block_hash {
            return Err("Certificate block hash mismatch".into());
        }

        let unique_signers: std::collections::HashSet<u64> =
            cert.signer_ids.iter().copied().collect();
        if unique_signers.len() != cert.signer_ids.len() {
            return Err("Duplicate signer IDs in commit certificate".into());
        }

        let quorum = (header.validator_set.active_count() * 2 / 3) + 1;
        if cert.signer_ids.len() < quorum {
            return Err(format!(
                "Insufficient signers: {} < quorum {}",
                cert.signer_ids.len(),
                quorum
            ));
        }

        let mut pks = Vec::new();
        let mut signing_stake = 0u64;
        for &vid in &cert.signer_ids {
            match header.validator_set.get(vid) {
                Some(v) => {
                    if let Some(ref bls_pk) = v.bls_public_key {
                        pks.push(BlsPublicKey(bls_pk.clone()));
                        signing_stake = signing_stake.saturating_add(v.stake);
                    } else {
                        return Err(format!("Signer {} has no BLS key", vid));
                    }
                }
                None => return Err(format!("Signer {} not in validator set", vid)),
            }
        }

        let total_stake = header.validator_set.total_stake();
        if (signing_stake as u128) * 3 < (total_stake as u128) * 2 {
            return Err(format!(
                "Insufficient signing stake: {} < 2/3 of {}",
                signing_stake, total_stake
            ));
        }

        let msg = bls_vote_message(cert.height, cert.round, &cert.block_hash);
        let agg_sig = BlsSignature(cert.aggregate_signature.clone());
        if !BlsVerifier::aggregate_verify(&msg, &agg_sig, &pks) {
            return Err("BLS aggregate signature verification failed".into());
        }

        Ok(())
    }

    fn check_validator_overlap(
        &self,
        trusted: &LightBlockHeader,
        untrusted: &LightBlockHeader,
    ) -> Result<(), String> {
        let trusted_total_stake = trusted.validator_set.total_stake();
        let threshold =
            trusted_total_stake * TRUST_THRESHOLD_NUMERATOR / TRUST_THRESHOLD_DENOMINATOR;

        let mut overlap_stake = 0u64;
        for &signer_id in &untrusted.commit_certificate.signer_ids {
            if let Some(trusted_validator) = trusted.validator_set.get(signer_id) {
                overlap_stake += trusted_validator.stake;
            }
        }

        if overlap_stake >= threshold {
            Ok(())
        } else {
            Err(format!(
                "Insufficient validator overlap: {} < threshold {}",
                overlap_stake, threshold
            ))
        }
    }

    fn add_trusted(&mut self, header: LightBlockHeader, current_time: u64) {
        let height = header.height;
        self.trusted_states.insert(
            height,
            TrustedState {
                header,
                trust_expires_at: current_time + self.trust_period,
            },
        );
        while self.trusted_states.len() > 100 {
            let oldest = *self.trusted_states.keys().next().unwrap();
            self.trusted_states.remove(&oldest);
        }
    }

    pub fn bisection_target(&self, trusted_height: u64, target_height: u64) -> u64 {
        (trusted_height + target_height) / 2
    }

    pub fn trusted_count(&self) -> usize {
        self.trusted_states.len()
    }

    pub fn prune_expired(&mut self, current_time: u64) {
        self.trusted_states
            .retain(|_, ts| ts.trust_expires_at > current_time);
    }
}

/// Construct the BLS vote message for verification (matches tendermint.rs format).
pub fn bls_vote_message(height: u64, round: u32, block_hash: &[u8; 32]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(48);
    msg.extend_from_slice(b"precommit");
    msg.extend_from_slice(&height.to_le_bytes());
    msg.extend_from_slice(&round.to_le_bytes());
    msg.extend_from_slice(block_hash);
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validator_info_new_defaults_correct() {
        let v = ValidatorInfo::new(1, 1000, [0x42; 32]);
        assert_eq!(v.id, 1);
        assert_eq!(v.stake, 1000);
        assert_eq!(v.address, [0x42; 32]);
        assert_eq!(v.health_score, 0.0);
        assert_eq!(v.delegated_stake, 0);
        assert!(!v.jailed);
        assert_eq!(v.total_slashed, 0);
        assert!(v.bls_public_key.is_none());
    }

    #[test]
    fn effective_stake_includes_delegations() {
        let mut v = ValidatorInfo::new(1, 1000, [0; 32]);
        v.delegated_stake = 500;
        assert_eq!(v.effective_stake(), 1500);
    }

    #[test]
    fn effective_stake_saturates_on_overflow() {
        let mut v = ValidatorInfo::new(1, u64::MAX, [0; 32]);
        v.delegated_stake = 1;
        assert_eq!(v.effective_stake(), u64::MAX);
    }

    #[test]
    fn effective_weight_zero_health_equals_stake() {
        let v = ValidatorInfo::new(1, 1000, [0; 32]);
        // health_score = 0 → multiplier = 1.0 → weight == stake
        assert_eq!(v.effective_weight(), 1000);
    }

    #[test]
    fn effective_weight_max_health_caps_at_120pct() {
        let mut v = ValidatorInfo::new(1, 1000, [0; 32]);
        v.health_score = 1.0;
        // 1.0 * 0.2 = 0.2 bonus → 1000 * 1.2 = 1200
        assert_eq!(v.effective_weight(), 1200);
    }

    #[test]
    fn effective_weight_clamps_score_above_one() {
        let mut v = ValidatorInfo::new(1, 1000, [0; 32]);
        v.health_score = 5.0; // Out-of-bounds — must clamp.
        assert_eq!(v.effective_weight(), 1200); // Same as score=1.0
    }

    #[test]
    fn jail_tombstoned_by_address_jails_matching_validator() {
        let mut vs = ValidatorSet::new();
        let addr_dead = [0x03u8; 32];
        let addr_alive = [0x05u8; 32];
        vs.add_validator(ValidatorInfo::new(1, 1000, addr_dead));
        vs.add_validator(ValidatorInfo::new(2, 1000, addr_alive));

        let n = vs.jail_tombstoned_by_address(&[addr_dead]);
        assert_eq!(n, 1, "exactly the tombstoned validator was newly jailed");
        assert!(vs.get(1).unwrap().jailed, "matched validator must be jailed");
        assert!(!vs.get(2).unwrap().jailed, "non-matching validator unaffected");
        // Health zeroed on jail.
        assert_eq!(vs.get(1).unwrap().health_score, 0.0);
    }

    #[test]
    fn jail_tombstoned_by_address_idempotent() {
        let mut vs = ValidatorSet::new();
        let addr_dead = [0x03u8; 32];
        vs.add_validator(ValidatorInfo::new(1, 1000, addr_dead));

        assert_eq!(vs.jail_tombstoned_by_address(&[addr_dead]), 1);
        // Already jailed — second call must not re-count.
        assert_eq!(
            vs.jail_tombstoned_by_address(&[addr_dead]),
            0,
            "already-jailed validator must not be re-counted"
        );
        assert!(vs.get(1).unwrap().jailed);
    }

    #[test]
    fn jail_tombstoned_by_address_empty_input_no_op() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x03; 32]));
        assert_eq!(vs.jail_tombstoned_by_address(&[]), 0);
        assert!(!vs.get(1).unwrap().jailed);
    }

    #[test]
    fn jail_tombstoned_excludes_dead_validator_from_leader_rotation() {
        // Doctrine end-to-end: a tombstoned validator must never be
        // returned by leader_for_epoch.
        let mut vs = ValidatorSet::new();
        let addr_dead = [0x03u8; 32];
        let addr_alive = [0x05u8; 32];
        vs.add_validator(ValidatorInfo::new(1, 1000, addr_dead));
        vs.add_validator(ValidatorInfo::new(2, 1000, addr_alive));

        vs.jail_tombstoned_by_address(&[addr_dead]);

        // Across a wide range of epochs, the tombstoned validator
        // (id=1) must never be elected.
        for epoch in 0..1000u64 {
            let leader = vs.leader_for_epoch(epoch).expect("a leader must exist");
            assert_ne!(
                leader.id, 1,
                "tombstoned validator was elected at epoch {epoch}",
            );
        }
    }
}
