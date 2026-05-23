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

/// Minimum stake to remain active. Below this, the validator is force-jailed.
/// Removal from the set is governance-only (Leave proposal).
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
    /// Fraction of delegator rewards the validator keeps before passing
    /// the remainder to delegators, in parts-per-million.
    /// 100_000 = 10% commission. Range 0–500_000 (0–50%).
    /// TOKENOMICS §2.2 / Q7 ceremony decision 2026-05-08: default 10%.
    #[serde(default = "ValidatorInfo::default_commission_ppm")]
    pub commission_ppm: u64,
}

impl ValidatorInfo {
    pub const fn default_commission_ppm() -> u64 {
        100_000 // 10%
    }

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
            commission_ppm: Self::default_commission_ppm(),
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
    /// Jails the validator; removal is governance-only, never triggered by slashing.
    pub fn slash_equivocation(&mut self, validator_id: u64) -> u64 {
        if let Some(v) = self.get_mut(validator_id) {
            let penalty = (v.stake as f64 * SLASH_EQUIVOCATION_PCT).round() as u64;
            v.stake = v.stake.saturating_sub(penalty);
            v.total_slashed += penalty;
            v.jailed = true;
            v.health_score = 0.0;
            penalty
        } else {
            0
        }
    }

    /// Slash a validator for downtime (missed block production).
    /// Jails when stake falls below MIN_STAKE; removal is governance-only.
    pub fn slash_downtime(&mut self, validator_id: u64, missed_blocks: u64) -> u64 {
        if let Some(v) = self.get_mut(validator_id) {
            let per_miss = (v.stake as f64 * SLASH_DOWNTIME_PCT).round() as u64;
            let penalty = per_miss.saturating_mul(missed_blocks);
            v.stake = v.stake.saturating_sub(penalty);
            v.total_slashed += penalty;
            if missed_blocks >= 3 || v.stake < MIN_STAKE {
                v.jailed = true;
            }
            v.health_score = (v.health_score - missed_blocks as f64 * 0.1).max(0.0);
            penalty
        } else {
            0
        }
    }

    /// Apply a precomputed slash amount to a validator.
    /// Jails the validator if stake falls below MIN_STAKE; removal is governance-only.
    pub fn slash_with_amount(&mut self, validator_id: u64, amount: u64, jail: bool) -> u64 {
        if let Some(v) = self.get_mut(validator_id) {
            let deducted = amount.min(v.stake);
            v.stake = v.stake.saturating_sub(deducted);
            v.total_slashed += deducted;
            if jail || v.stake < MIN_STAKE {
                v.jailed = true;
                v.health_score = 0.0;
            }
            deducted
        } else {
            0
        }
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
    ///
    /// H-1 (audit 2026-05-17): `chain_id` is now bound into the VRF input
    /// to prevent cross-chain leader-claim replay.
    pub fn verify_vrf_proposal(
        &self,
        chain_id: &str,
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
        let alpha = evaporchain_crypto::vrf::leader_vrf_input(chain_id, height, round);
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
    /// Chain ID bound into BLS vote messages — must match the signer's chain_id.
    chain_id: String,
}

impl LightClientVerifier {
    /// Create a new verifier with a genesis trusted state.
    pub fn new(genesis_header: LightBlockHeader, current_time: u64, chain_id: &str) -> Self {
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
            chain_id: chain_id.to_string(),
        }
    }

    /// Create with a custom trust period (useful for testing).
    pub fn with_trust_period(
        genesis_header: LightBlockHeader,
        current_time: u64,
        trust_period: u64,
        chain_id: &str,
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
            chain_id: chain_id.to_string(),
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

        let msg = bls_vote_message(&self.chain_id, cert.height, cert.round, &cert.block_hash);
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

/// Construct the BLS vote message for verification — must match tendermint.rs format exactly.
/// Format: u8(len(chain_id)) || chain_id || "precommit" || height_le8 || round_le4 || block_hash
pub fn bls_vote_message(chain_id: &str, height: u64, round: u32, block_hash: &[u8; 32]) -> Vec<u8> {
    let chain_id_bytes = chain_id.as_bytes();
    debug_assert!(
        chain_id_bytes.len() < 256,
        "chain_id too long for u8 length prefix"
    );
    let mut msg = Vec::with_capacity(1 + chain_id_bytes.len() + 9 + 8 + 4 + 32);
    msg.push(chain_id_bytes.len() as u8);
    msg.extend_from_slice(chain_id_bytes);
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

    // ── with_bls_pop constructor ─────────────────────────────────────────

    #[test]
    fn with_bls_pop_stores_key_and_pop_unverified() {
        use evaporchain_crypto::BlsKeypair;
        let kp = BlsKeypair::generate();
        let pk = kp.public_key_bytes().0;
        let pop = kp.proof_of_possession().0;

        let v = ValidatorInfo::with_bls_pop(7, 5000, [0xAA; 32], pk.clone(), pop.clone());

        assert_eq!(v.id, 7);
        assert_eq!(v.stake, 5000);
        assert_eq!(v.address, [0xAA; 32]);
        assert_eq!(v.bls_public_key.as_ref().unwrap(), &pk);
        assert_eq!(v.bls_pop.as_ref().unwrap(), &pop);
        // pop_verified must be false — caller must call verify_pop explicitly
        assert!(!v.pop_verified);
    }

    // ── add_validator_with_pop / verify_pop ─────────────────────────────

    #[test]
    fn add_validator_with_pop_and_verify_pop_happy_path() {
        use evaporchain_crypto::BlsKeypair;
        let kp = BlsKeypair::generate();
        let pk = kp.public_key_bytes().0.clone();
        let pop = kp.proof_of_possession().0.clone();

        // verify_pop must accept a freshly-generated keypair's PoP
        assert!(
            ValidatorSet::verify_pop(&pk, &pop),
            "genuine PoP must verify"
        );

        let mut vs = ValidatorSet::new();
        let info = ValidatorInfo::with_bls_pop(3, 2000, [0x11; 32], pk.clone(), pop.clone());
        let added = vs.add_validator_with_pop(info, pop.clone(), true);
        assert!(added);
        let v = vs.get(3).unwrap();
        assert!(v.pop_verified);
        assert_eq!(v.bls_pop.as_ref().unwrap(), &pop);
    }

    #[test]
    fn verify_pop_rejects_wrong_signature() {
        use evaporchain_crypto::BlsKeypair;
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let pk1 = kp1.public_key_bytes().0;
        let pop2 = kp2.proof_of_possession().0; // wrong PoP for pk1

        assert!(
            !ValidatorSet::verify_pop(&pk1, &pop2),
            "mismatched pk/pop must not verify"
        );
    }

    #[test]
    fn add_validator_with_pop_duplicate_id_returns_false() {
        use evaporchain_crypto::BlsKeypair;
        let kp = BlsKeypair::generate();
        let pop = kp.proof_of_possession().0;

        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(5, 1000, [0x22; 32]));
        let added = vs.add_validator_with_pop(
            ValidatorInfo::new(5, 999, [0x33; 32]),
            pop,
            true,
        );
        assert!(!added, "duplicate id must be rejected");
    }

    // ── leader fallthrough (zero-total-stake branch) ──────────────────────

    #[test]
    fn leader_for_epoch_zero_stake_falls_through_to_last() {
        // All validators have stake = 0 → total = 0 → fallthrough branch.
        // With 3 validators, epochs 0/3/6 pick idx 0, epochs 1/4/7 pick idx 1, etc.
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 0, [0x01; 32]));
        vs.add_validator(ValidatorInfo::new(2, 0, [0x02; 32]));
        vs.add_validator(ValidatorInfo::new(3, 0, [0x03; 32]));

        let leader = vs.leader_for_epoch(0).expect("should elect a leader");
        // epoch 0 % 3 = 0 → first validator (id=1)
        assert_eq!(leader.id, 1);

        let leader1 = vs.leader_for_epoch(1).expect("should elect a leader");
        assert_eq!(leader1.id, 2);

        let leader2 = vs.leader_for_epoch(2).expect("should elect a leader");
        assert_eq!(leader2.id, 3);
    }

    // ── is_leader ────────────────────────────────────────────────────────

    #[test]
    fn is_leader_true_for_epoch_winner_false_for_others() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x01; 32]));
        vs.add_validator(ValidatorInfo::new(2, 1000, [0x02; 32]));

        // Find an epoch where validator 1 wins, then assert is_leader
        let mut found_epoch_for_v1 = None;
        let mut found_epoch_for_v2 = None;
        for epoch in 0..500u64 {
            match vs.leader_for_epoch(epoch).map(|v| v.id) {
                Some(1) if found_epoch_for_v1.is_none() => {
                    found_epoch_for_v1 = Some(epoch)
                }
                Some(2) if found_epoch_for_v2.is_none() => {
                    found_epoch_for_v2 = Some(epoch)
                }
                _ => {}
            }
        }
        let e1 = found_epoch_for_v1.expect("v1 should win at least one epoch in 500");
        let e2 = found_epoch_for_v2.expect("v2 should win at least one epoch in 500");

        assert!(vs.is_leader(1, e1), "v1 is_leader at its winning epoch");
        assert!(!vs.is_leader(2, e1), "v2 is not leader at v1's epoch");
        assert!(vs.is_leader(2, e2), "v2 is_leader at its winning epoch");
        assert!(!vs.is_leader(1, e2), "v1 is not leader at v2's epoch");
        assert!(!vs.is_leader(99, e1), "unknown validator is never leader");
    }

    // ── unjail ────────────────────────────────────────────────────────────

    #[test]
    fn unjail_clears_jailed_flag_when_stake_sufficient() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, MIN_STAKE, [0x01; 32]));
        vs.jail_tombstoned_by_address(&[[0x01; 32]]);
        assert!(vs.get(1).unwrap().jailed);

        let result = vs.unjail(1);
        assert!(result, "unjail must return true");
        assert!(!vs.get(1).unwrap().jailed, "validator must no longer be jailed");
    }

    #[test]
    fn unjail_non_jailed_validator_returns_false() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, MIN_STAKE, [0x01; 32]));
        assert!(!vs.unjail(1), "unjailing a non-jailed validator must return false");
    }

    #[test]
    fn unjail_nonexistent_validator_returns_false() {
        let mut vs = ValidatorSet::new();
        assert!(!vs.unjail(999), "unjailing absent validator must return false");
    }

    #[test]
    fn unjail_below_min_stake_returns_false() {
        let mut vs = ValidatorSet::new();
        // Give 0 stake — below MIN_STAKE, so unjail must refuse.
        vs.add_validator(ValidatorInfo::new(1, 0, [0x01; 32]));
        // Manually jail it (slash_equivocation jails)
        vs.slash_equivocation(1);
        // After slash, stake is still 0 and validator is jailed.
        assert!(vs.get(1).unwrap().jailed, "should be jailed after equivocation");
        assert!(!vs.unjail(1), "cannot unjail a validator below min_stake");
    }

    // ── verify_vrf_proposal ───────────────────────────────────────────────

    #[test]
    fn verify_vrf_proposal_accepts_valid_proof() {
        use evaporchain_crypto::vrf::{VrfKeypair, leader_vrf_input};

        let vrf_kp = VrfKeypair::generate();
        let pk_bytes = vrf_kp.public_key_bytes();

        let chain_id = "test-chain";
        let height = 42u64;
        let round = 0u32;
        let alpha = leader_vrf_input(chain_id, height, round);
        let (output, proof) = vrf_kp.evaluate(&alpha);

        let mut vs = ValidatorSet::new();
        let info = ValidatorInfo::with_keys(1, 1000, [0x01; 32], None, Some(pk_bytes));
        vs.add_validator(info);

        assert!(
            vs.verify_vrf_proposal(chain_id, 1, height, round, &output.0, &proof.0),
            "valid VRF proof must be accepted"
        );
    }

    #[test]
    fn verify_vrf_proposal_rejects_wrong_chain_id() {
        use evaporchain_crypto::vrf::{VrfKeypair, leader_vrf_input};

        let vrf_kp = VrfKeypair::generate();
        let pk_bytes = vrf_kp.public_key_bytes();

        let height = 10u64;
        let round = 0u32;
        // Proof generated for "chain-A"
        let alpha = leader_vrf_input("chain-A", height, round);
        let (output, proof) = vrf_kp.evaluate(&alpha);

        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::with_keys(1, 1000, [0x01; 32], None, Some(pk_bytes)));

        // Verify against "chain-B" — must fail (H-1 cross-chain replay guard)
        assert!(
            !vs.verify_vrf_proposal("chain-B", 1, height, round, &output.0, &proof.0),
            "proof for chain-A must not verify on chain-B"
        );
    }

    #[test]
    fn verify_vrf_proposal_rejects_missing_vrf_key() {
        let mut vs = ValidatorSet::new();
        // Validator has no VRF key registered
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x01; 32]));

        let dummy_output = [0u8; 32];
        let dummy_proof = vec![0u8; 32];
        assert!(
            !vs.verify_vrf_proposal("test-chain", 1, 1, 0, &dummy_output, &dummy_proof),
            "absent VRF key must return false"
        );
    }

    #[test]
    fn verify_vrf_proposal_rejects_unknown_proposer() {
        let vs = ValidatorSet::new();
        let dummy_output = [0u8; 32];
        let dummy_proof = vec![0u8; 32];
        assert!(
            !vs.verify_vrf_proposal("test-chain", 99, 1, 0, &dummy_output, &dummy_proof),
            "unknown proposer must return false"
        );
    }

    // ── with_bls_key / with_validators ────────────────────────────────────

    #[test]
    fn with_bls_key_stores_key_only() {
        let pk = vec![0xBBu8; 48];
        let v = ValidatorInfo::with_bls_key(9, 3000, [0x77; 32], pk.clone());
        assert_eq!(v.bls_public_key.as_ref().unwrap(), &pk);
        assert!(v.bls_pop.is_none());
        assert!(!v.pop_verified);
    }

    #[test]
    fn with_validators_constructor_populates_set() {
        let vs = ValidatorSet::with_validators(vec![
            ValidatorInfo::new(1, 100, [0x01; 32]),
            ValidatorInfo::new(2, 200, [0x02; 32]),
        ]);
        assert_eq!(vs.len(), 2);
        assert!(!vs.is_empty());
        assert_eq!(vs.validators().len(), 2);
    }

    // ── rotate_validator_key / purge_expired_prev_keys ────────────────────

    #[test]
    fn rotate_validator_key_happy_path() {
        use evaporchain_crypto::BlsKeypair;
        let old_kp = BlsKeypair::generate();
        let new_kp = BlsKeypair::generate();
        let old_pk = old_kp.public_key_bytes().0;
        let new_pk = new_kp.public_key_bytes().0;
        let new_pop = new_kp.proof_of_possession().0;

        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::with_bls_pop(
            1, 1000, [0x01; 32], old_pk.clone(), old_kp.proof_of_possession().0,
        ));

        assert!(vs.rotate_validator_key(1, new_pk.clone(), new_pop.clone(), 10));
        let v = vs.get(1).unwrap();
        assert_eq!(v.bls_public_key.as_deref().unwrap(), new_pk.as_slice());
        assert_eq!(v.bls_public_key_prev.as_deref().unwrap(), old_pk.as_slice());
        assert_eq!(v.bls_prev_key_expiry_epoch.unwrap(), 10);
        assert!(v.pop_verified);
    }

    #[test]
    fn rotate_validator_key_no_existing_key_returns_false() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x01; 32]));
        assert!(!vs.rotate_validator_key(1, vec![1u8; 48], vec![2u8; 96], 5));
    }

    #[test]
    fn rotate_validator_key_unknown_id_returns_false() {
        let mut vs = ValidatorSet::new();
        assert!(!vs.rotate_validator_key(99, vec![1u8; 48], vec![2u8; 96], 5));
    }

    #[test]
    fn purge_expired_prev_keys_removes_past_expiry() {
        use evaporchain_crypto::BlsKeypair;
        let old_kp = BlsKeypair::generate();
        let new_kp = BlsKeypair::generate();

        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::with_bls_pop(
            1, 1000, [0x01; 32],
            old_kp.public_key_bytes().0, old_kp.proof_of_possession().0,
        ));
        vs.rotate_validator_key(1, new_kp.public_key_bytes().0, new_kp.proof_of_possession().0, 5);
        assert!(vs.get(1).unwrap().bls_public_key_prev.is_some());

        // Epoch 5 — not yet expired (expiry = 5, check is strictly >)
        assert_eq!(vs.purge_expired_prev_keys(5), 0);
        // Epoch 6 — now past expiry
        assert_eq!(vs.purge_expired_prev_keys(6), 1);
        assert!(vs.get(1).unwrap().bls_public_key_prev.is_none());
        assert!(vs.get(1).unwrap().bls_prev_key_expiry_epoch.is_none());
    }

    // ── remove_validator / len / is_empty / validators ────────────────────

    #[test]
    fn remove_validator_present_returns_true() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 100, [0x01; 32]));
        assert!(vs.remove_validator(1));
        assert_eq!(vs.len(), 0);
        assert!(vs.is_empty());
    }

    #[test]
    fn remove_validator_absent_returns_false() {
        let mut vs = ValidatorSet::new();
        assert!(!vs.remove_validator(99));
    }

    // ── total_weight / update_health_score / decay_health_scores ──────────

    #[test]
    fn total_weight_excludes_jailed_validators() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x01; 32]));
        vs.add_validator(ValidatorInfo::new(2, 1000, [0x02; 32]));
        vs.jail_tombstoned_by_address(&[[0x02; 32]]);
        // Only validator 1 active, health=0 → weight = 1000
        assert_eq!(vs.total_weight(), 1000);
    }

    #[test]
    fn update_health_score_increments_blocks_produced_and_caps_score() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x01; 32]));
        // One evaporation → health_score = 0.05
        vs.update_health_score(1, 1);
        let v = vs.get(1).unwrap();
        assert_eq!(v.blocks_produced, 1);
        assert_eq!(v.evaporations_processed, 1);
        assert!((v.health_score - HEALTH_PER_EVAPORATION).abs() < 1e-9);

        // Drive health to MAX
        for _ in 0..100 {
            vs.update_health_score(1, 20);
        }
        assert_eq!(vs.get(1).unwrap().health_score, MAX_HEALTH_SCORE);
    }

    #[test]
    fn decay_health_scores_reduces_all_validators() {
        let mut vs = ValidatorSet::new();
        let mut v = ValidatorInfo::new(1, 1000, [0x01; 32]);
        v.health_score = 0.5;
        vs.add_validator(v);
        vs.decay_health_scores();
        let after = vs.get(1).unwrap().health_score;
        assert!((after - (0.5 - HEALTH_DECAY_RATE)).abs() < 1e-9);
    }

    #[test]
    fn decay_health_scores_floors_at_zero() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x01; 32])); // health=0.0
        vs.decay_health_scores();
        assert_eq!(vs.get(1).unwrap().health_score, 0.0);
    }

    // ── active_count / total_stake / total_self_stake / get_validator ──────

    #[test]
    fn active_count_excludes_jailed() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 100, [0x01; 32]));
        vs.add_validator(ValidatorInfo::new(2, 100, [0x02; 32]));
        vs.slash_equivocation(2);
        assert_eq!(vs.active_count(), 1);
    }

    #[test]
    fn total_stake_and_self_stake_exclude_jailed() {
        let mut vs = ValidatorSet::new();
        let mut v1 = ValidatorInfo::new(1, 500, [0x01; 32]);
        v1.delegated_stake = 200;
        vs.add_validator(v1);
        let mut v2 = ValidatorInfo::new(2, 300, [0x02; 32]);
        v2.delegated_stake = 100;
        vs.add_validator(v2);
        vs.slash_equivocation(2); // jail v2

        // total_stake = effective_stake of v1 only = 500 + 200 = 700
        assert_eq!(vs.total_stake(), 700);
        // total_self_stake = v1.stake only = 500
        assert_eq!(vs.total_self_stake(), 500);
    }

    #[test]
    fn get_validator_mirrors_get() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(3, 100, [0x03; 32]));
        assert_eq!(vs.get_validator(3).map(|v| v.id), Some(3));
        assert!(vs.get_validator(99).is_none());
    }

    // ── slash_downtime / slash_with_amount ────────────────────────────────

    #[test]
    fn slash_downtime_jails_on_three_or_more_misses() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 10000, [0x01; 32]));
        let penalty = vs.slash_downtime(1, 3);
        assert!(penalty > 0);
        assert!(vs.get(1).unwrap().jailed);
    }

    #[test]
    fn slash_downtime_does_not_jail_on_two_misses() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 10000, [0x01; 32]));
        vs.slash_downtime(1, 2);
        assert!(!vs.get(1).unwrap().jailed);
    }

    #[test]
    fn slash_downtime_unknown_id_returns_zero() {
        let mut vs = ValidatorSet::new();
        assert_eq!(vs.slash_downtime(99, 5), 0);
    }

    #[test]
    fn slash_with_amount_jails_when_flag_set() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 10000, [0x01; 32]));
        let deducted = vs.slash_with_amount(1, 500, true);
        assert_eq!(deducted, 500);
        assert_eq!(vs.get(1).unwrap().stake, 9500);
        assert!(vs.get(1).unwrap().jailed);
    }

    #[test]
    fn slash_with_amount_no_jail_when_flag_clear_and_stake_sufficient() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 10000, [0x01; 32]));
        vs.slash_with_amount(1, 100, false);
        assert!(!vs.get(1).unwrap().jailed);
    }

    #[test]
    fn slash_with_amount_caps_at_available_stake() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 100, [0x01; 32]));
        let deducted = vs.slash_with_amount(1, 999, false);
        assert_eq!(deducted, 100);
        assert_eq!(vs.get(1).unwrap().stake, 0);
    }

    #[test]
    fn slash_with_amount_unknown_id_returns_zero() {
        let mut vs = ValidatorSet::new();
        assert_eq!(vs.slash_with_amount(99, 100, false), 0);
    }

    // ── has_bls_keys / has_vrf_keys ────────────────────────────────────────

    #[test]
    fn has_bls_keys_true_when_all_validators_have_key() {
        use evaporchain_crypto::BlsKeypair;
        let kp = BlsKeypair::generate();
        let pk = kp.public_key_bytes().0;
        let pop = kp.proof_of_possession().0;
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::with_bls_pop(1, 100, [0x01; 32], pk, pop));
        assert!(vs.has_bls_keys());
    }

    #[test]
    fn has_bls_keys_false_when_any_validator_missing_key() {
        use evaporchain_crypto::BlsKeypair;
        let kp = BlsKeypair::generate();
        let pk = kp.public_key_bytes().0;
        let pop = kp.proof_of_possession().0;
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::with_bls_pop(1, 100, [0x01; 32], pk, pop));
        vs.add_validator(ValidatorInfo::new(2, 100, [0x02; 32])); // no BLS key
        assert!(!vs.has_bls_keys());
    }

    #[test]
    fn has_vrf_keys_true_when_at_least_one_has_vrf() {
        use evaporchain_crypto::vrf::VrfKeypair;
        let vrf_kp = VrfKeypair::generate();
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::with_keys(1, 100, [0x01; 32], None, Some(vrf_kp.public_key_bytes())));
        vs.add_validator(ValidatorInfo::new(2, 100, [0x02; 32]));
        assert!(vs.has_vrf_keys());
    }

    #[test]
    fn has_vrf_keys_false_when_none_registered() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 100, [0x01; 32]));
        assert!(!vs.has_vrf_keys());
    }

    // ── vrf_leader_qualifies / vrf_sortition ──────────────────────────────

    #[test]
    fn vrf_leader_qualifies_rejects_jailed_validator() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x01; 32]));
        vs.slash_equivocation(1);
        assert!(!vs.vrf_leader_qualifies(1, &[0u8; 32]));
    }

    #[test]
    fn vrf_leader_qualifies_rejects_unknown_validator() {
        let vs = ValidatorSet::new();
        assert!(!vs.vrf_leader_qualifies(99, &[0u8; 32]));
    }

    #[test]
    fn vrf_sortition_returns_zero_for_jailed() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x01; 32]));
        vs.slash_equivocation(1);
        assert_eq!(vs.vrf_sortition(1, &[0u8; 32], 10), 0);
    }

    #[test]
    fn vrf_sortition_returns_zero_for_unknown() {
        let vs = ValidatorSet::new();
        assert_eq!(vs.vrf_sortition(99, &[0u8; 32], 10), 0);
    }

    // ── leader_for_epoch_with_seed ────────────────────────────────────────

    #[test]
    fn leader_for_epoch_with_seed_returns_none_when_all_jailed() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x01; 32]));
        vs.slash_equivocation(1);
        assert!(vs.leader_for_epoch_with_seed(0, &[0u8; 32]).is_none());
    }

    #[test]
    fn leader_for_epoch_with_seed_consistent_for_same_input() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x01; 32]));
        vs.add_validator(ValidatorInfo::new(2, 1000, [0x02; 32]));
        let seed = [0xFFu8; 32];
        let a = vs.leader_for_epoch_with_seed(7, &seed).unwrap().id;
        let b = vs.leader_for_epoch_with_seed(7, &seed).unwrap().id;
        assert_eq!(a, b, "deterministic: same epoch+seed must pick same leader");
    }

    #[test]
    fn leader_for_epoch_with_seed_zero_stake_uses_fallthrough() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 0, [0x01; 32]));
        vs.add_validator(ValidatorInfo::new(2, 0, [0x02; 32]));
        // Must not panic; returns Some
        assert!(vs.leader_for_epoch_with_seed(0, &[0u8; 32]).is_some());
    }

    // ── remaining one-liner gaps ──────────────────────────────────────────

    #[test]
    fn slash_equivocation_unknown_id_returns_zero() {
        let mut vs = ValidatorSet::new();
        assert_eq!(vs.slash_equivocation(99), 0);
    }

    #[test]
    fn default_validator_set_is_empty() {
        let vs = ValidatorSet::default();
        assert!(vs.is_empty());
    }

    #[test]
    fn leader_for_epoch_returns_none_when_all_jailed() {
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::new(1, 1000, [0x01; 32]));
        vs.slash_equivocation(1);
        assert!(vs.leader_for_epoch(0).is_none());
    }

    #[test]
    fn vrf_leader_qualifies_and_sortition_active_path() {
        use evaporchain_crypto::vrf::{VrfKeypair, leader_vrf_input};

        let vrf_kp = VrfKeypair::generate();
        let pk_bytes = vrf_kp.public_key_bytes();
        let mut vs = ValidatorSet::new();
        vs.add_validator(ValidatorInfo::with_keys(
            1, 1_000_000, [0x01; 32], None, Some(pk_bytes),
        ));

        let alpha = leader_vrf_input("test-chain", 1, 0);
        let (output, _proof) = vrf_kp.evaluate(&alpha);

        // vrf_leader_qualifies reaches the active path (validator found, not jailed)
        let _ = vs.vrf_leader_qualifies(1, &output.0);
        // vrf_sortition also reaches the active path
        let _ = vs.vrf_sortition(1, &output.0, 10);
    }

    // ── LightClientVerifier ───────────────────────────────────────────────

    /// Build a `LightBlockHeader` signed by a set of BLS keypairs.
    fn make_signed_header(
        height: u64,
        block_hash: [u8; 32],
        keypairs: &[(u64, evaporchain_crypto::BlsKeypair)],
        chain_id: &str,
    ) -> LightBlockHeader {
        use evaporchain_crypto::signatures::{BlsSignature, BlsVerifier};
        use evaporchain_types::CommitCertificate;

        let mut vs = ValidatorSet::new();
        for (id, kp) in keypairs {
            let pk = kp.public_key_bytes().0;
            let pop = kp.proof_of_possession().0;
            vs.add_validator(ValidatorInfo::with_bls_pop(*id, 1000, [(*id) as u8; 32], pk, pop));
        }

        let round = 0u32;
        let msg = bls_vote_message(chain_id, height, round, &block_hash);
        let sigs: Vec<BlsSignature> = keypairs.iter().map(|(_, kp)| kp.sign(&msg)).collect();
        let agg = BlsVerifier::aggregate_signatures(&sigs).unwrap();
        let signer_ids: Vec<u64> = keypairs.iter().map(|(id, _)| *id).collect();

        LightBlockHeader {
            height,
            epoch: 0,
            block_hash,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            timestamp: 1_000_000,
            validator_set: vs,
            commit_certificate: CommitCertificate {
                height,
                round,
                block_hash,
                aggregate_signature: agg.0,
                signer_ids,
            },
        }
    }

    #[test]
    fn light_client_verifier_new_stores_genesis_and_returns_latest_height() {
        use evaporchain_crypto::BlsKeypair;
        let kp = BlsKeypair::generate();
        let genesis = make_signed_header(0, [0xABu8; 32], &[(1, kp)], "test-chain");
        let vcr = LightClientVerifier::new(genesis, 1_000_000, "test-chain");
        assert_eq!(vcr.latest_trusted_height(), Some(0));
        assert!(vcr.trusted_state_at(0).is_some());
        assert!(vcr.trusted_state_at(99).is_none());
        assert_eq!(vcr.trusted_count(), 1);
    }

    #[test]
    fn light_client_verifier_with_trust_period_and_sequential_verify() {
        use evaporchain_crypto::BlsKeypair;
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let kp3 = BlsKeypair::generate();

        let chain_id = "test-chain";
        let keypairs = vec![(1u64, kp1), (2u64, kp2), (3u64, kp3)];

        let genesis = make_signed_header(0, [0x00u8; 32], &keypairs, chain_id);
        let block1 = make_signed_header(1, [0x01u8; 32], &keypairs, chain_id);

        let mut vcr =
            LightClientVerifier::with_trust_period(genesis, 0, 9999, chain_id);

        // Sequential verify (height gap = 1)
        let result = vcr.verify(&block1, 0);
        assert_eq!(result, VerificationResult::Valid);
        assert_eq!(vcr.latest_trusted_height(), Some(1));
        assert_eq!(vcr.trusted_count(), 2);
    }

    #[test]
    fn light_client_verifier_expired_trust_period_returns_invalid() {
        use evaporchain_crypto::BlsKeypair;
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let kp3 = BlsKeypair::generate();
        let keypairs = vec![(1u64, kp1), (2u64, kp2), (3u64, kp3)];
        let chain_id = "test-chain";

        let genesis = make_signed_header(0, [0x00u8; 32], &keypairs, chain_id);
        let block1 = make_signed_header(1, [0x01u8; 32], &keypairs, chain_id);

        // Trust period = 100s. Current time for verify = 200 → expired.
        let mut vcr = LightClientVerifier::with_trust_period(genesis, 0, 100, chain_id);
        match vcr.verify(&block1, 200) {
            VerificationResult::Invalid(msg) => assert!(msg.contains("Trust period")),
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn light_client_verifier_no_trusted_state_returns_invalid() {
        use evaporchain_crypto::BlsKeypair;
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let kp3 = BlsKeypair::generate();
        let keypairs = vec![(1u64, kp1), (2u64, kp2), (3u64, kp3)];
        let chain_id = "test-chain";

        let genesis = make_signed_header(0, [0x00u8; 32], &keypairs, chain_id);
        // Attempt to verify block at height 0 with no prior state → no trusted state
        let mut vcr = LightClientVerifier::with_trust_period(genesis.clone(), 0, 9999, chain_id);
        // height=0 → saturating_sub(1) = 0, but genesis IS the trusted state at 0
        // Instead: verify a block at height 0 from scratch on an EMPTY verifier.
        // We can't easily create an empty verifier from the API, so test the
        // "no state below untrusted height" case by using height=0 on genesis verifier:
        // The best_trusted_state_for(0.saturating_sub(1)) = best_trusted_state_for(0) = genesis.
        // Actually this is tricky. Instead test: block at height=0 on a verifier whose genesis is height 5.
        let genesis5 = make_signed_header(5, [0x05u8; 32], &keypairs, chain_id);
        let mut vcr2 = LightClientVerifier::with_trust_period(genesis5, 0, 9999, chain_id);
        let block1 = make_signed_header(1, [0x01u8; 32], &keypairs, chain_id);
        match vcr2.verify(&block1, 0) {
            VerificationResult::Invalid(msg) => assert!(msg.contains("No trusted state")),
            other => panic!("expected Invalid(No trusted state), got {:?}", other),
        }
    }

    #[test]
    fn light_client_verifier_large_gap_returns_need_bisection() {
        use evaporchain_crypto::BlsKeypair;
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let kp3 = BlsKeypair::generate();
        let keypairs = vec![(1u64, kp1), (2u64, kp2), (3u64, kp3)];
        let chain_id = "test-chain";

        let genesis = make_signed_header(0, [0x00u8; 32], &keypairs, chain_id);
        // Block at height = MAX_SKIP_HEIGHT_GAP + 2 — forces NeedBisection
        let far_block = make_signed_header(
            MAX_SKIP_HEIGHT_GAP + 2,
            [0xFFu8; 32],
            &keypairs,
            chain_id,
        );

        let mut vcr = LightClientVerifier::with_trust_period(genesis, 0, 9_999_999, chain_id);
        match vcr.verify(&far_block, 0) {
            VerificationResult::NeedBisection {
                trusted_height,
                target_height,
            } => {
                assert_eq!(trusted_height, 0);
                assert_eq!(target_height, MAX_SKIP_HEIGHT_GAP + 2);
                // bisection_target utility
                let mid = vcr.bisection_target(trusted_height, target_height);
                assert_eq!(mid, (0 + MAX_SKIP_HEIGHT_GAP + 2) / 2);
            }
            other => panic!("expected NeedBisection, got {:?}", other),
        }
    }

    #[test]
    fn light_client_verifier_prune_expired_removes_stale_states() {
        use evaporchain_crypto::BlsKeypair;
        let kp1 = BlsKeypair::generate();
        let kp2 = BlsKeypair::generate();
        let kp3 = BlsKeypair::generate();
        let keypairs = vec![(1u64, kp1), (2u64, kp2), (3u64, kp3)];
        let chain_id = "test-chain";

        let genesis = make_signed_header(0, [0x00u8; 32], &keypairs, chain_id);
        let block1 = make_signed_header(1, [0x01u8; 32], &keypairs, chain_id);

        // Trust period = 100s; genesis added at t=0 → expires at t=100
        let mut vcr = LightClientVerifier::with_trust_period(genesis, 0, 100, chain_id);
        vcr.verify(&block1, 0); // adds block1 trusted state, also expires at 100

        assert_eq!(vcr.trusted_count(), 2);
        vcr.prune_expired(200); // both states expired at t=100
        assert_eq!(vcr.trusted_count(), 0);
    }

    #[test]
    fn bls_vote_message_encodes_correctly() {
        let msg = bls_vote_message("test", 5, 2, &[0xABu8; 32]);
        // u8(4) + "test" + "precommit" + 8 bytes height + 4 bytes round + 32 bytes hash
        assert_eq!(msg.len(), 1 + 4 + 9 + 8 + 4 + 32);
        assert_eq!(msg[0], 4); // len("test")
        assert_eq!(&msg[1..5], b"test");
        assert_eq!(&msg[5..14], b"precommit");
    }
}
