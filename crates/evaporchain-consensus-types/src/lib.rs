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
}
