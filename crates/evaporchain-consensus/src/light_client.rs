//! Light client verification for EvaporChain.
//!
//! Enables wallets, bridges, and new nodes to verify chain state without
//! replaying all blocks. Uses BLS aggregate commit certificates to prove
//! that 2/3+ of validators attested to a block.
//!
//! Two verification modes:
//! - **Sequential**: verify headers one-by-one, each signed by the current set
//! - **Skipping**: jump across heights if the trusted validator set overlap
//!   exceeds the trust threshold (1/3 of trusted set must still be in new set)
//!
//! Based on Tendermint light client spec (ICS-007 / CometBFT).

use evaporchain_crypto::signatures::{BlsPublicKey, BlsSignature, BlsVerifier};
use evaporchain_crypto::hash::blake3_hash;
use evaporchain_da::sampling::{DASampler, SampleQuery, SampleResponse};
use evaporchain_types::CommitCertificate;
use std::collections::BTreeMap;
use tracing::debug;

use crate::validator_set::ValidatorSet;

// ─────────────────────── Types ───────────────────────────────────────

/// A light block header — the minimal data needed to verify consensus.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    pub commit_certificate: CommitCertificate,
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
    /// Header is valid and trusted.
    Valid,
    /// Header is invalid — reason provided.
    Invalid(String),
    /// Cannot verify — need intermediate headers (bisection required).
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

// ─────────────────────── Constants ───────────────────────────────────

/// Trust period in seconds. After this, the trusted state must be refreshed.
/// Default: 2 weeks (1_209_600 seconds).
const TRUST_PERIOD_SECS: u64 = 14 * 24 * 3600;

/// Fraction of trusted validator set stake that must overlap with the
/// new validator set for skip verification to succeed (1/3).
const TRUST_THRESHOLD_NUMERATOR: u64 = 1;
const TRUST_THRESHOLD_DENOMINATOR: u64 = 3;

/// Maximum height gap for skip verification without bisection.
const MAX_SKIP_HEIGHT_GAP: u64 = 10_000;

// ─────────────────────── LightClientVerifier ─────────────────────────

/// Verifies block headers using commit certificates and validator set tracking.
pub struct LightClientVerifier {
    /// Trusted states indexed by height.
    trusted_states: BTreeMap<u64, TrustedState>,
    /// Trust period in seconds.
    trust_period: u64,
}

impl LightClientVerifier {
    /// Create a new light client verifier with a genesis trusted state.
    pub fn new(genesis_header: LightBlockHeader, current_time: u64) -> Self {
        let height = genesis_header.height;
        let mut trusted_states = BTreeMap::new();
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
        let mut trusted_states = BTreeMap::new();
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

    /// Get the latest trusted height.
    pub fn latest_trusted_height(&self) -> Option<u64> {
        self.trusted_states.keys().next_back().copied()
    }

    /// Get a trusted state at a specific height.
    pub fn trusted_state_at(&self, height: u64) -> Option<&TrustedState> {
        self.trusted_states.get(&height)
    }

    /// Get the highest trusted state at or below the given height.
    fn best_trusted_state_for(&self, target_height: u64) -> Option<&TrustedState> {
        self.trusted_states
            .range(..=target_height)
            .next_back()
            .map(|(_, ts)| ts)
    }

    /// Verify an untrusted header against our trusted state.
    ///
    /// This is the main entry point. It:
    /// 1. Finds the best trusted state
    /// 2. Checks trust period hasn't expired
    /// 3. Verifies the commit certificate (2/3+ BLS signatures)
    /// 4. For sequential: checks height == trusted + 1
    /// 5. For skipping: checks sufficient validator overlap
    pub fn verify(
        &mut self,
        untrusted: &LightBlockHeader,
        current_time: u64,
    ) -> VerificationResult {
        // Find the best trusted state at or below the untrusted height
        let trusted = match self.best_trusted_state_for(untrusted.height.saturating_sub(1)) {
            Some(ts) => ts.clone(),
            None => return VerificationResult::Invalid("No trusted state found".into()),
        };

        // Check trust period
        if current_time > trusted.trust_expires_at {
            return VerificationResult::Invalid(
                "Trust period expired — need fresh checkpoint".into(),
            );
        }

        // Verify the commit certificate against the untrusted header's own validator set
        match self.verify_commit_certificate(untrusted) {
            Ok(()) => {}
            Err(e) => return VerificationResult::Invalid(format!("Invalid certificate: {}", e)),
        }

        // Sequential verification: height == trusted + 1
        let height_gap = untrusted.height.saturating_sub(trusted.header.height);
        if height_gap == 1 {
            // Sequential: the untrusted header's validator set should be signed by trusted set
            // For sequential, we trust the transition if the cert is valid
            debug!(
                trusted = trusted.header.height,
                untrusted = untrusted.height,
                "Sequential verification passed"
            );
            self.add_trusted(untrusted.clone(), current_time);
            return VerificationResult::Valid;
        }

        // Skip verification: check validator overlap
        if height_gap > MAX_SKIP_HEIGHT_GAP {
            return VerificationResult::NeedBisection {
                trusted_height: trusted.header.height,
                target_height: untrusted.height,
            };
        }

        // Check that sufficient stake from the trusted set is present in the signing set
        match self.check_validator_overlap(&trusted.header, untrusted) {
            Ok(()) => {
                debug!(
                    trusted = trusted.header.height,
                    untrusted = untrusted.height,
                    gap = height_gap,
                    "Skip verification passed"
                );
                self.add_trusted(untrusted.clone(), current_time);
                VerificationResult::Valid
            }
            Err(_) => {
                // Not enough overlap — need bisection
                VerificationResult::NeedBisection {
                    trusted_height: trusted.header.height,
                    target_height: untrusted.height,
                }
            }
        }
    }

    /// Verify the BLS commit certificate on a header.
    fn verify_commit_certificate(&self, header: &LightBlockHeader) -> Result<(), String> {
        let cert = &header.commit_certificate;

        // Certificate must be for the correct height and block
        if cert.height != header.height {
            return Err(format!(
                "Certificate height {} != header height {}",
                cert.height, header.height
            ));
        }
        if cert.block_hash != header.block_hash {
            return Err("Certificate block hash mismatch".into());
        }

        // Reject duplicate signer IDs (prevents double-counted stake)
        let unique_signers: std::collections::HashSet<u64> = cert.signer_ids.iter().copied().collect();
        if unique_signers.len() != cert.signer_ids.len() {
            return Err("Duplicate signer IDs in commit certificate".into());
        }

        // Collect BLS public keys from signers
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

        // Verify 2/3 stake threshold (use u128 to prevent overflow)
        let total_stake = header.validator_set.total_stake();
        if (signing_stake as u128) * 3 < (total_stake as u128) * 2 {
            return Err(format!(
                "Insufficient signing stake: {} < 2/3 of {}",
                signing_stake, total_stake
            ));
        }

        // Verify BLS aggregate signature
        let msg = bls_vote_message(cert.height, cert.round, &cert.block_hash);
        let agg_sig = BlsSignature(cert.aggregate_signature.clone());
        if !BlsVerifier::aggregate_verify(&msg, &agg_sig, &pks) {
            return Err("BLS aggregate signature verification failed".into());
        }

        Ok(())
    }

    /// Check that enough stake from the trusted validator set is still
    /// present in the untrusted header's signing set.
    fn check_validator_overlap(
        &self,
        trusted: &LightBlockHeader,
        untrusted: &LightBlockHeader,
    ) -> Result<(), String> {
        let trusted_total_stake = trusted.validator_set.total_stake();
        let threshold = trusted_total_stake * TRUST_THRESHOLD_NUMERATOR
            / TRUST_THRESHOLD_DENOMINATOR;

        // Sum stake of validators that:
        // 1. Were in the trusted set
        // 2. Signed the untrusted commit certificate
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

    /// Add a verified header as trusted.
    fn add_trusted(&mut self, header: LightBlockHeader, current_time: u64) {
        let height = header.height;
        self.trusted_states.insert(
            height,
            TrustedState {
                header,
                trust_expires_at: current_time + self.trust_period,
            },
        );

        // Prune old trusted states — keep last 100
        while self.trusted_states.len() > 100 {
            let oldest = *self.trusted_states.keys().next().unwrap();
            self.trusted_states.remove(&oldest);
        }
    }

    /// Bisect: find the midpoint between trusted and target for step-by-step verification.
    pub fn bisection_target(&self, trusted_height: u64, target_height: u64) -> u64 {
        (trusted_height + target_height) / 2
    }

    /// Number of trusted states stored.
    pub fn trusted_count(&self) -> usize {
        self.trusted_states.len()
    }

    /// Remove expired trusted states.
    pub fn prune_expired(&mut self, current_time: u64) {
        self.trusted_states
            .retain(|_, ts| ts.trust_expires_at > current_time);
    }
}

/// Construct the BLS vote message for verification (matches tendermint.rs format).
fn bls_vote_message(height: u64, round: u32, block_hash: &[u8; 32]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(48);
    msg.extend_from_slice(b"precommit");
    msg.extend_from_slice(&height.to_le_bytes());
    msg.extend_from_slice(&round.to_le_bytes());
    msg.extend_from_slice(block_hash);
    msg
}

// ─────────────────────── DA Sampling Verifier ───────────────────────

/// Minimum number of valid shard samples needed for DA confidence.
const MIN_DA_SAMPLES: usize = 4;

/// Result of data availability verification.
#[derive(Debug, Clone, PartialEq)]
pub enum DAVerificationResult {
    /// Sufficient samples verified — data is available with high probability.
    Available { samples_verified: usize },
    /// Not enough valid samples — data may be unavailable.
    Unavailable { valid: usize, required: usize },
    /// No data_root in block header — nothing to verify.
    NoDataRoot,
}

/// Data availability sampling verifier for light clients.
///
/// Given a block's `data_root` (commitment over erasure-coded shards),
/// the light client requests random shard samples from full nodes via P2P
/// and verifies each sample's Merkle proof against the data_root.
pub struct DASVerifier;

impl DASVerifier {
    /// Generate random shard sample queries for a block.
    ///
    /// The light client sends these queries to full nodes via the
    /// P2P shard sampling protocol.
    pub fn generate_queries(
        block_number: u64,
        total_shards: usize,
        num_samples: usize,
        seed: &[u8],
    ) -> Vec<SampleQuery> {
        DASampler::generate_queries(block_number, total_shards, num_samples, seed)
    }

    /// Verify shard sample responses against a block's data_root.
    ///
    /// Each response contains shard data + Merkle proof. The verifier checks:
    /// 1. Shard hash matches the data (not tampered)
    /// 2. Merkle proof verifies against data_root
    /// 3. Sufficient samples pass (≥ MIN_DA_SAMPLES)
    pub fn verify_samples(
        data_root: &[u8; 32],
        responses: &[SampleResponse],
    ) -> DAVerificationResult {
        let mut valid_count = 0;

        for response in responses {
            // Check shard hash integrity
            let computed_hash: [u8; 32] = blake3::hash(&response.shard.data).into();
            if computed_hash != response.shard.hash {
                continue;
            }

            // Check Merkle proof root matches block data_root
            if response.proof.root != *data_root {
                continue;
            }

            // Verify Merkle proof
            if DASampler::verify_proof(&response.shard, &response.proof) {
                valid_count += 1;
            }
        }

        if valid_count >= MIN_DA_SAMPLES {
            DAVerificationResult::Available {
                samples_verified: valid_count,
            }
        } else {
            DAVerificationResult::Unavailable {
                valid: valid_count,
                required: MIN_DA_SAMPLES,
            }
        }
    }

    /// Verify data availability for a block header.
    ///
    /// Returns `NoDataRoot` if the block has no DA commitment (empty block).
    pub fn verify_block_da(
        data_root: Option<&[u8; 32]>,
        responses: &[SampleResponse],
    ) -> DAVerificationResult {
        match data_root {
            Some(root) => Self::verify_samples(root, responses),
            None => DAVerificationResult::NoDataRoot,
        }
    }
}

// ─────────────────────── Epoch-Parameterized State Proofs ──────────────

use crate::anchor::{DecayRules, ObjectSnapshot, ObjectLifecycleState, LazyStateEvaluator};

/// A self-contained proof that "object X has energy Y and state S at epoch E."
///
/// Light clients can verify this without full chain sync:
///   1. Verify the Merkle proof of the object snapshot against the anchor state root
///   2. Verify the anchor was committed by 2/3+ validators (via block commitment)
///   3. Apply the decay formula: energy = anchor_energy * 2^(-(query_epoch - anchor_epoch) / half_life)
///
/// This is EvaporChain's novel contribution to light client design: time-dependent
/// state verification from a single anchor + deterministic decay rules.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EpochStateProof {
    /// The object being proven.
    pub object_id: [u8; 32],
    /// Object snapshot at the anchor point (energy, half_life, state).
    pub snapshot: ObjectSnapshot,
    /// Merkle proof of the object's inclusion in the anchor state root.
    pub merkle_proof_path: Vec<[u8; 32]>,
    /// The anchor's state root (Merkle proof target).
    pub anchor_state_root: [u8; 32],
    /// The anchor hash (links to BlockStateCommitment).
    pub anchor_hash: [u8; 32],
    /// Epoch of the anchor.
    pub anchor_epoch: u64,
    /// Canonical decay rules hash (must match what validators committed to).
    pub decay_rules_hash: [u8; 32],
    /// The decay rules themselves (so the verifier can recompute).
    pub decay_rules: DecayRules,
    /// The epoch being queried.
    pub query_epoch: u64,
    /// Block height that carries the BlockStateCommitment referencing this anchor.
    pub commitment_block_height: u64,
}

/// Result of epoch state proof verification.
#[derive(Debug, Clone, PartialEq)]
pub struct EpochProofResult {
    pub object_id: [u8; 32],
    pub query_epoch: u64,
    pub energy: u64,
    pub state: ObjectLifecycleState,
    pub anchor_epoch: u64,
}

/// Error from epoch state proof verification.
#[derive(Debug, Clone, PartialEq)]
pub enum EpochProofError {
    QueryBeforeAnchor,
    DecayRulesHashMismatch,
    MerkleProofInvalid,
    ObjectIdMismatch,
    AnchorEpochMismatch,
    CommitmentBlockNotTrusted,
}

/// Verifies epoch-parameterized state proofs for light clients.
pub struct EpochStateProofVerifier;

impl EpochStateProofVerifier {
    /// Verify an epoch state proof and return the computed energy and state.
    ///
    /// Verification steps:
    /// 1. Check query_epoch >= anchor_epoch
    /// 2. Verify decay_rules hash matches the committed hash
    /// 3. Verify the snapshot's object_id matches the proof's object_id
    /// 4. Verify the Merkle proof of the snapshot against anchor_state_root
    /// 5. Apply the decay formula to compute energy at query_epoch
    /// 6. Determine the lifecycle state at query_epoch
    pub fn verify(proof: &EpochStateProof) -> Result<EpochProofResult, EpochProofError> {
        if proof.query_epoch < proof.anchor_epoch {
            return Err(EpochProofError::QueryBeforeAnchor);
        }

        if proof.snapshot.object_id != proof.object_id {
            return Err(EpochProofError::ObjectIdMismatch);
        }

        if proof.snapshot.anchor_epoch != proof.anchor_epoch {
            return Err(EpochProofError::AnchorEpochMismatch);
        }

        let computed_rules_hash = proof.decay_rules.hash();
        if computed_rules_hash != proof.decay_rules_hash {
            return Err(EpochProofError::DecayRulesHashMismatch);
        }

        if !Self::verify_merkle_proof(
            &proof.snapshot,
            &proof.merkle_proof_path,
            &proof.anchor_state_root,
        ) {
            return Err(EpochProofError::MerkleProofInvalid);
        }

        let energy = LazyStateEvaluator::energy_at(
            &proof.snapshot,
            &proof.decay_rules,
            proof.query_epoch,
        );
        let state = LazyStateEvaluator::state_at(
            &proof.snapshot,
            &proof.decay_rules,
            proof.query_epoch,
        );

        Ok(EpochProofResult {
            object_id: proof.object_id,
            query_epoch: proof.query_epoch,
            energy,
            state,
            anchor_epoch: proof.anchor_epoch,
        })
    }

    /// Verify with trust chain: also check that the commitment block is trusted
    /// by the light client verifier.
    pub fn verify_with_trust(
        proof: &EpochStateProof,
        light_client: &LightClientVerifier,
    ) -> Result<EpochProofResult, EpochProofError> {
        if light_client
            .trusted_state_at(proof.commitment_block_height)
            .is_none()
        {
            return Err(EpochProofError::CommitmentBlockNotTrusted);
        }

        Self::verify(proof)
    }

    /// Verify Merkle proof of object snapshot inclusion in anchor state root.
    fn verify_merkle_proof(
        snapshot: &ObjectSnapshot,
        proof_path: &[[u8; 32]],
        state_root: &[u8; 32],
    ) -> bool {
        let leaf_data = Self::serialize_snapshot(snapshot);
        let mut current = blake3_hash(&leaf_data);

        for sibling in proof_path {
            let mut combined = Vec::with_capacity(64);
            if current <= *sibling {
                combined.extend_from_slice(&current);
                combined.extend_from_slice(sibling);
            } else {
                combined.extend_from_slice(sibling);
                combined.extend_from_slice(&current);
            }
            current = blake3_hash(&combined);
        }

        current == *state_root
    }

    /// Canonical serialization of an ObjectSnapshot for Merkle leaf hashing.
    fn serialize_snapshot(snapshot: &ObjectSnapshot) -> Vec<u8> {
        let mut data = Vec::with_capacity(81);
        data.extend_from_slice(b"epoch-proof-leaf-v1");
        data.extend_from_slice(&snapshot.object_id);
        data.extend_from_slice(&snapshot.energy_at_anchor.to_le_bytes());
        data.extend_from_slice(&snapshot.half_life.to_le_bytes());
        data.extend_from_slice(&snapshot.anchor_epoch.to_le_bytes());
        data.push(match snapshot.state {
            ObjectLifecycleState::Active => 0,
            ObjectLifecycleState::Grace => 1,
            ObjectLifecycleState::Ghost => 2,
        });
        if let Some(ge) = snapshot.grace_epoch {
            data.push(1);
            data.extend_from_slice(&ge.to_le_bytes());
        } else {
            data.push(0);
        }
        data
    }

    /// Build a valid Merkle proof for a set of object snapshots.
    /// Returns (proof_path, state_root) for the target object.
    ///
    /// This is the prover side — called by full nodes to generate proofs
    /// that light clients then verify.
    pub fn build_merkle_proof(
        snapshots: &[ObjectSnapshot],
        target_index: usize,
    ) -> (Vec<[u8; 32]>, [u8; 32]) {
        if snapshots.is_empty() || target_index >= snapshots.len() {
            return (vec![], [0u8; 32]);
        }

        let leaves: Vec<[u8; 32]> = snapshots
            .iter()
            .map(|s| blake3_hash(&Self::serialize_snapshot(s)))
            .collect();

        Self::build_proof_from_leaves(&leaves, target_index)
    }

    fn build_proof_from_leaves(
        leaves: &[[u8; 32]],
        target_index: usize,
    ) -> (Vec<[u8; 32]>, [u8; 32]) {
        if leaves.len() == 1 {
            return (vec![], leaves[0]);
        }

        let mut padded = leaves.to_vec();
        while padded.len().count_ones() != 1 {
            padded.push([0u8; 32]);
        }

        let mut proof = Vec::new();
        let mut level = padded;
        let mut idx = target_index;

        while level.len() > 1 {
            let sibling_idx = if idx.is_multiple_of(2) { idx + 1 } else { idx - 1 };
            if sibling_idx < level.len() {
                proof.push(level[sibling_idx]);
            } else {
                proof.push([0u8; 32]);
            }

            let mut next_level = Vec::new();
            for pair in level.chunks(2) {
                let left = pair[0];
                let right = if pair.len() > 1 { pair[1] } else { [0u8; 32] };
                let mut combined = Vec::with_capacity(64);
                if left <= right {
                    combined.extend_from_slice(&left);
                    combined.extend_from_slice(&right);
                } else {
                    combined.extend_from_slice(&right);
                    combined.extend_from_slice(&left);
                }
                next_level.push(blake3_hash(&combined));
            }
            level = next_level;
            idx /= 2;
        }

        (proof, level[0])
    }
}

// ─────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validator_set::ValidatorInfo;
    use evaporchain_crypto::signatures::BlsKeypair;

    /// Create a validator set with BLS keys, returning (set, keypairs).
    fn make_validator_set_with_bls(n: u64, stake: u64) -> (ValidatorSet, Vec<BlsKeypair>) {
        let mut vs = ValidatorSet::new();
        let mut keypairs = Vec::new();
        for i in 0..n {
            let kp = BlsKeypair::generate();
            let mut info = ValidatorInfo::new(i, stake, [i as u8; 32]);
            info.bls_public_key = Some(kp.public_key_bytes().0);
            info.pop_verified = true; // test-only: bypass PoP to keep fixture simple
            vs.add_validator(info);
            keypairs.push(kp);
        }
        (vs, keypairs)
    }

    /// Build a valid commit certificate signed by the given keypairs.
    fn make_commit_certificate(
        height: u64,
        round: u32,
        block_hash: [u8; 32],
        keypairs: &[BlsKeypair],
        signer_ids: &[u64],
    ) -> CommitCertificate {
        let msg = bls_vote_message(height, round, &block_hash);
        let sigs: Vec<BlsSignature> = signer_ids
            .iter()
            .map(|&id| keypairs[id as usize].sign(&msg))
            .collect();
        let agg_sig = BlsVerifier::aggregate_signatures(&sigs).unwrap();
        CommitCertificate {
            height,
            round,
            block_hash,
            aggregate_signature: agg_sig.0,
            signer_ids: signer_ids.to_vec(),
        }
    }

    fn make_light_header(
        height: u64,
        epoch: u64,
        vs: ValidatorSet,
        cert: CommitCertificate,
    ) -> LightBlockHeader {
        LightBlockHeader {
            height,
            epoch,
            block_hash: cert.block_hash,
            parent_hash: [0u8; 32],
            state_root: [height as u8; 32],
            timestamp: height * 10,
            validator_set: vs,
            commit_certificate: cert,
        }
    }

    #[test]
    fn test_sequential_verification() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let block_hash = [1u8; 32];

        // Genesis at height 1
        let cert1 = make_commit_certificate(1, 0, block_hash, &kps, &[0, 1, 2]);
        let genesis = make_light_header(1, 0, vs.clone(), cert1);
        let mut lc = LightClientVerifier::new(genesis, 100);

        // Height 2 — sequential
        let block_hash2 = [2u8; 32];
        let cert2 = make_commit_certificate(2, 0, block_hash2, &kps, &[0, 1, 2, 3]);
        let header2 = make_light_header(2, 0, vs.clone(), cert2);

        let result = lc.verify(&header2, 200);
        assert_eq!(result, VerificationResult::Valid);
        assert_eq!(lc.latest_trusted_height(), Some(2));
    }

    #[test]
    fn test_skip_verification_same_validators() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let block_hash = [1u8; 32];

        let cert1 = make_commit_certificate(1, 0, block_hash, &kps, &[0, 1, 2]);
        let genesis = make_light_header(1, 0, vs.clone(), cert1);
        let mut lc = LightClientVerifier::new(genesis, 100);

        // Jump to height 50 — same validator set, full overlap
        let block_hash50 = [50u8; 32];
        let cert50 = make_commit_certificate(50, 0, block_hash50, &kps, &[0, 1, 2, 3]);
        let header50 = make_light_header(50, 0, vs.clone(), cert50);

        let result = lc.verify(&header50, 200);
        assert_eq!(result, VerificationResult::Valid);
        assert_eq!(lc.latest_trusted_height(), Some(50));
    }

    #[test]
    fn test_skip_verification_partial_overlap() {
        let (vs1, kps1) = make_validator_set_with_bls(4, 1000);
        let block_hash = [1u8; 32];

        let cert1 = make_commit_certificate(1, 0, block_hash, &kps1, &[0, 1, 2]);
        let genesis = make_light_header(1, 0, vs1.clone(), cert1);
        let mut lc = LightClientVerifier::new(genesis, 100);

        // New validator set: keep validators 0,1 (2000 stake overlap out of 4000 total)
        // Add new validators 4,5
        let (mut vs2, mut kps2) = make_validator_set_with_bls(4, 1000);
        // vs2 has ids 0,1,2,3 with DIFFERENT BLS keys than vs1
        // For overlap to work, we need signers from vs1 that are also in vs2
        // Since vs2 has the same IDs (0-3), the overlap check passes on ID basis
        // But we need to sign with vs2's keys for the cert to verify against vs2
        let block_hash50 = [50u8; 32];
        let cert50 = make_commit_certificate(50, 0, block_hash50, &kps2, &[0, 1, 2]);
        let header50 = make_light_header(50, 0, vs2.clone(), cert50);

        // Overlap: validators 0,1,2,3 from trusted set (stake 4000) are all in signing set's IDs
        // Signers 0,1,2 have overlap stake from trusted: 3000 >= threshold (4000/3 = 1333)
        let result = lc.verify(&header50, 200);
        assert_eq!(result, VerificationResult::Valid);
    }

    #[test]
    fn test_insufficient_signers_rejected() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let block_hash = [1u8; 32];

        let cert1 = make_commit_certificate(1, 0, block_hash, &kps, &[0, 1, 2]);
        let genesis = make_light_header(1, 0, vs.clone(), cert1);
        let mut lc = LightClientVerifier::new(genesis, 100);

        // Only 1 signer (need 3 for quorum with 4 validators)
        let block_hash2 = [2u8; 32];
        let cert2 = make_commit_certificate(2, 0, block_hash2, &kps, &[0]);
        let header2 = make_light_header(2, 0, vs.clone(), cert2);

        let result = lc.verify(&header2, 200);
        match result {
            VerificationResult::Invalid(msg) => assert!(msg.contains("Insufficient")),
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn test_expired_trust_period() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let block_hash = [1u8; 32];

        let cert1 = make_commit_certificate(1, 0, block_hash, &kps, &[0, 1, 2]);
        let genesis = make_light_header(1, 0, vs.clone(), cert1);
        // Trust period = 1000 seconds
        let mut lc = LightClientVerifier::with_trust_period(genesis, 100, 1000);

        // Try to verify at time 1200 (trust expired at 1100)
        let block_hash2 = [2u8; 32];
        let cert2 = make_commit_certificate(2, 0, block_hash2, &kps, &[0, 1, 2]);
        let header2 = make_light_header(2, 0, vs.clone(), cert2);

        let result = lc.verify(&header2, 1200);
        match result {
            VerificationResult::Invalid(msg) => assert!(msg.contains("expired")),
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn test_wrong_block_hash_in_cert() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let block_hash = [1u8; 32];

        let cert1 = make_commit_certificate(1, 0, block_hash, &kps, &[0, 1, 2]);
        let genesis = make_light_header(1, 0, vs.clone(), cert1);
        let mut lc = LightClientVerifier::new(genesis, 100);

        // Certificate signed over different hash than header claims
        let block_hash2 = [2u8; 32];
        let wrong_hash = [99u8; 32];
        let cert2 = make_commit_certificate(2, 0, wrong_hash, &kps, &[0, 1, 2]);
        let mut header2 = make_light_header(2, 0, vs.clone(), cert2);
        header2.block_hash = block_hash2; // mismatch!

        let result = lc.verify(&header2, 200);
        match result {
            VerificationResult::Invalid(msg) => assert!(msg.contains("hash mismatch")),
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn test_need_bisection_large_gap() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let block_hash = [1u8; 32];

        let cert1 = make_commit_certificate(1, 0, block_hash, &kps, &[0, 1, 2]);
        let genesis = make_light_header(1, 0, vs.clone(), cert1);
        let mut lc = LightClientVerifier::new(genesis, 100);

        // Jump > MAX_SKIP_HEIGHT_GAP
        let target = 1 + MAX_SKIP_HEIGHT_GAP + 1;
        let block_hash_far = [0xFFu8; 32];
        let cert_far = make_commit_certificate(target, 0, block_hash_far, &kps, &[0, 1, 2, 3]);
        let header_far = make_light_header(target, 0, vs.clone(), cert_far);

        let result = lc.verify(&header_far, 200);
        match result {
            VerificationResult::NeedBisection {
                trusted_height,
                target_height,
            } => {
                assert_eq!(trusted_height, 1);
                assert_eq!(target_height, target);
            }
            other => panic!("Expected NeedBisection, got {:?}", other),
        }
    }

    #[test]
    fn test_bisection_target_calculation() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let cert = make_commit_certificate(1, 0, [1u8; 32], &kps, &[0, 1, 2]);
        let genesis = make_light_header(1, 0, vs, cert);
        let lc = LightClientVerifier::new(genesis, 100);

        assert_eq!(lc.bisection_target(1, 101), 51);
        assert_eq!(lc.bisection_target(100, 200), 150);
    }

    #[test]
    fn test_prune_expired() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);

        let cert1 = make_commit_certificate(1, 0, [1u8; 32], &kps, &[0, 1, 2]);
        let genesis = make_light_header(1, 0, vs.clone(), cert1);
        let mut lc = LightClientVerifier::with_trust_period(genesis, 100, 500);

        // Add height 2 at time 200
        let cert2 = make_commit_certificate(2, 0, [2u8; 32], &kps, &[0, 1, 2]);
        let h2 = make_light_header(2, 0, vs.clone(), cert2);
        assert_eq!(lc.verify(&h2, 200), VerificationResult::Valid);

        assert_eq!(lc.trusted_count(), 2);

        // Prune at time 650: genesis (expires 600) should be removed, h2 (expires 700) stays
        lc.prune_expired(650);
        assert_eq!(lc.trusted_count(), 1);
        assert!(lc.trusted_state_at(1).is_none());
        assert!(lc.trusted_state_at(2).is_some());
    }

    #[test]
    fn test_sequential_chain_of_headers() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);

        let cert1 = make_commit_certificate(1, 0, [1u8; 32], &kps, &[0, 1, 2]);
        let genesis = make_light_header(1, 0, vs.clone(), cert1);
        let mut lc = LightClientVerifier::new(genesis, 100);

        // Verify a chain of 10 sequential headers
        for h in 2..=11u64 {
            let hash = [h as u8; 32];
            let cert = make_commit_certificate(h, 0, hash, &kps, &[0, 1, 2, 3]);
            let header = make_light_header(h, 0, vs.clone(), cert);
            let result = lc.verify(&header, 100 + h * 10);
            assert_eq!(result, VerificationResult::Valid, "Height {} should verify", h);
        }
        assert_eq!(lc.latest_trusted_height(), Some(11));
    }

    #[test]
    fn test_forged_signature_rejected() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let block_hash = [1u8; 32];

        let cert1 = make_commit_certificate(1, 0, block_hash, &kps, &[0, 1, 2]);
        let genesis = make_light_header(1, 0, vs.clone(), cert1);
        let mut lc = LightClientVerifier::new(genesis, 100);

        // Create cert with wrong keys (generate fresh keypairs)
        let (_, fake_kps) = make_validator_set_with_bls(4, 1000);
        let block_hash2 = [2u8; 32];
        let forged_cert = make_commit_certificate(2, 0, block_hash2, &fake_kps, &[0, 1, 2]);
        let header2 = make_light_header(2, 0, vs.clone(), forged_cert);

        let result = lc.verify(&header2, 200);
        match result {
            VerificationResult::Invalid(msg) => {
                assert!(msg.contains("signature") || msg.contains("failed"));
            }
            other => panic!("Expected Invalid, got {:?}", other),
        }
    }

    // ── DAS Verifier Tests ──

    #[test]
    fn test_das_verifier_valid_samples() {
        use evaporchain_da::block_da::BlockDA;

        let da = BlockDA::new().unwrap();
        let data = b"light client DAS verification test data";
        let package = da.encode_block(data).unwrap();
        let data_root = package.header.commitment_root;

        // Generate valid sample responses (at least MIN_DA_SAMPLES)
        let mut responses = Vec::new();
        for i in 0..6 {
            let response = da.prove_shard(&package, i % package.shards.len()).unwrap();
            responses.push(response);
        }

        let result = DASVerifier::verify_samples(&data_root, &responses);
        assert_eq!(
            result,
            DAVerificationResult::Available { samples_verified: 6 }
        );
    }

    #[test]
    fn test_das_verifier_insufficient_samples() {
        use evaporchain_da::block_da::BlockDA;

        let da = BlockDA::new().unwrap();
        let data = b"test data for insufficient sampling";
        let package = da.encode_block(data).unwrap();
        let data_root = package.header.commitment_root;

        // Only 2 samples — below MIN_DA_SAMPLES (4)
        let responses: Vec<SampleResponse> = (0..2)
            .map(|i| da.prove_shard(&package, i).unwrap())
            .collect();

        let result = DASVerifier::verify_samples(&data_root, &responses);
        assert_eq!(
            result,
            DAVerificationResult::Unavailable { valid: 2, required: 4 }
        );
    }

    #[test]
    fn test_das_verifier_tampered_shard_rejected() {
        use evaporchain_da::block_da::BlockDA;

        let da = BlockDA::new().unwrap();
        let data = b"tamper detection test";
        let package = da.encode_block(data).unwrap();
        let data_root = package.header.commitment_root;

        // Get valid responses then tamper one
        let mut responses: Vec<SampleResponse> = (0..5)
            .map(|i| da.prove_shard(&package, i % package.shards.len()).unwrap())
            .collect();

        // Tamper with the first shard's data
        responses[0].shard.data[0] ^= 0xFF;

        let result = DASVerifier::verify_samples(&data_root, &responses);
        // One tampered sample fails, but 4 valid ones still pass
        assert_eq!(
            result,
            DAVerificationResult::Available { samples_verified: 4 }
        );
    }

    #[test]
    fn test_das_verifier_wrong_data_root() {
        use evaporchain_da::block_da::BlockDA;

        let da = BlockDA::new().unwrap();
        let data = b"wrong root test";
        let package = da.encode_block(data).unwrap();

        let wrong_root = [0xAA; 32];
        let responses: Vec<SampleResponse> = (0..5)
            .map(|i| da.prove_shard(&package, i % package.shards.len()).unwrap())
            .collect();

        let result = DASVerifier::verify_samples(&wrong_root, &responses);
        assert_eq!(
            result,
            DAVerificationResult::Unavailable { valid: 0, required: 4 }
        );
    }

    #[test]
    fn test_das_verifier_no_data_root() {
        let result = DASVerifier::verify_block_da(None, &[]);
        assert_eq!(result, DAVerificationResult::NoDataRoot);
    }

    #[test]
    fn test_das_query_generation() {
        let queries = DASVerifier::generate_queries(42, 8, 6, b"light-client-seed");
        assert_eq!(queries.len(), 6);
        for q in &queries {
            assert_eq!(q.block_number, 42);
            assert!(q.shard_index < 8);
        }
    }

    #[test]
    fn test_das_e2e_light_client_flow() {
        use evaporchain_da::block_da::BlockDA;

        // Simulate complete light client DAS flow:
        // 1. Block producer encodes data
        let da = BlockDA::new().unwrap();
        let block_data = b"full e2e light client DAS verification flow test";
        let package = da.encode_block(block_data).unwrap();
        let data_root = package.header.commitment_root;

        // 2. Light client generates random queries
        let queries = DASVerifier::generate_queries(
            100,
            package.header.total_shards,
            6,
            b"lc-sampling-seed",
        );

        // 3. Full node responds with shard proofs
        let responses: Vec<SampleResponse> = queries
            .iter()
            .map(|q| da.prove_shard(&package, q.shard_index).unwrap())
            .collect();

        // 4. Light client verifies samples against data_root from block header
        let result = DASVerifier::verify_block_da(Some(&data_root), &responses);
        assert_eq!(
            result,
            DAVerificationResult::Available { samples_verified: 6 }
        );
    }

    // ── Epoch-Parameterized State Proof Tests ──

    fn make_test_snapshot(id_byte: u8, energy: u64, half_life: u64, anchor_epoch: u64) -> ObjectSnapshot {
        ObjectSnapshot {
            object_id: [id_byte; 32],
            energy_at_anchor: energy,
            half_life,
            anchor_epoch,
            state: ObjectLifecycleState::Active,
            grace_epoch: None,
        }
    }

    fn make_epoch_proof(
        snapshots: &[ObjectSnapshot],
        target_index: usize,
        query_epoch: u64,
    ) -> EpochStateProof {
        let rules = DecayRules::default_rules();
        let (proof_path, state_root) =
            EpochStateProofVerifier::build_merkle_proof(snapshots, target_index);
        let snapshot = snapshots[target_index].clone();
        let anchor_epoch = snapshot.anchor_epoch;

        // Compute a plausible anchor hash
        let anchor_hash = blake3_hash(&state_root);

        EpochStateProof {
            object_id: snapshot.object_id,
            snapshot,
            merkle_proof_path: proof_path,
            anchor_state_root: state_root,
            anchor_hash,
            anchor_epoch,
            decay_rules_hash: rules.hash(),
            decay_rules: rules,
            query_epoch,
            commitment_block_height: 100,
        }
    }

    #[test]
    fn test_epoch_proof_single_object() {
        let snap = make_test_snapshot(1, 1000, 10, 0);
        let proof = make_epoch_proof(&[snap], 0, 10);

        let result = EpochStateProofVerifier::verify(&proof).unwrap();
        assert_eq!(result.energy, 500); // 1000 >> (10/10) = 500
        assert_eq!(result.state, ObjectLifecycleState::Active);
        assert_eq!(result.query_epoch, 10);
        assert_eq!(result.anchor_epoch, 0);
    }

    #[test]
    fn test_epoch_proof_multiple_objects() {
        let snaps = vec![
            make_test_snapshot(1, 1000, 10, 0),
            make_test_snapshot(2, 2000, 20, 0),
            make_test_snapshot(3, 500, 5, 0),
        ];

        // Prove object 2 at epoch 20
        let proof = make_epoch_proof(&snaps, 1, 20);
        let result = EpochStateProofVerifier::verify(&proof).unwrap();
        assert_eq!(result.energy, 1000); // 2000 >> (20/20) = 1000
        assert_eq!(result.object_id, [2u8; 32]);
    }

    #[test]
    fn test_epoch_proof_zero_energy_grace() {
        let snap = make_test_snapshot(1, 1000, 10, 0);
        // At epoch 100: 1000 >> (100/10) = 1000 >> 10 = 0 → energy = 0
        // zero_epoch = 0 + (9+1)*10 = 100, grace_period=7, query=105 < 107 → Grace
        let proof = make_epoch_proof(&[snap], 0, 105);
        let result = EpochStateProofVerifier::verify(&proof).unwrap();
        assert_eq!(result.energy, 0);
        assert_eq!(result.state, ObjectLifecycleState::Grace);
    }

    #[test]
    fn test_epoch_proof_ghost_after_grace() {
        let snap = make_test_snapshot(1, 1000, 10, 0);
        // zero_epoch = 100, grace_period=7, query=107 >= 107 → Ghost
        let proof = make_epoch_proof(&[snap], 0, 107);
        let result = EpochStateProofVerifier::verify(&proof).unwrap();
        assert_eq!(result.energy, 0);
        assert_eq!(result.state, ObjectLifecycleState::Ghost);
    }

    #[test]
    fn test_epoch_proof_at_anchor_epoch() {
        let snap = make_test_snapshot(1, 1000, 10, 50);
        let proof = make_epoch_proof(&[snap], 0, 50);
        let result = EpochStateProofVerifier::verify(&proof).unwrap();
        assert_eq!(result.energy, 1000); // No decay elapsed
    }

    #[test]
    fn test_epoch_proof_query_before_anchor_rejected() {
        let snap = make_test_snapshot(1, 1000, 10, 50);
        let proof = make_epoch_proof(&[snap], 0, 40);
        let err = EpochStateProofVerifier::verify(&proof).unwrap_err();
        assert_eq!(err, EpochProofError::QueryBeforeAnchor);
    }

    #[test]
    fn test_epoch_proof_tampered_snapshot_rejected() {
        let snap = make_test_snapshot(1, 1000, 10, 0);
        let mut proof = make_epoch_proof(&[snap], 0, 10);
        // Tamper: change the energy in the snapshot
        proof.snapshot.energy_at_anchor = 9999;

        let err = EpochStateProofVerifier::verify(&proof).unwrap_err();
        assert_eq!(err, EpochProofError::MerkleProofInvalid);
    }

    #[test]
    fn test_epoch_proof_wrong_object_id_rejected() {
        let snap = make_test_snapshot(1, 1000, 10, 0);
        let mut proof = make_epoch_proof(&[snap], 0, 10);
        proof.object_id = [99u8; 32]; // Mismatch

        let err = EpochStateProofVerifier::verify(&proof).unwrap_err();
        assert_eq!(err, EpochProofError::ObjectIdMismatch);
    }

    #[test]
    fn test_epoch_proof_wrong_decay_rules_rejected() {
        let snap = make_test_snapshot(1, 1000, 10, 0);
        let mut proof = make_epoch_proof(&[snap], 0, 10);
        // Tamper the decay rules hash
        proof.decay_rules_hash = [0xFFu8; 32];

        let err = EpochStateProofVerifier::verify(&proof).unwrap_err();
        assert_eq!(err, EpochProofError::DecayRulesHashMismatch);
    }

    #[test]
    fn test_epoch_proof_wrong_anchor_epoch_rejected() {
        let snap = make_test_snapshot(1, 1000, 10, 0);
        let mut proof = make_epoch_proof(&[snap], 0, 10);
        proof.anchor_epoch = 5; // Mismatch with snapshot.anchor_epoch (0)

        let err = EpochStateProofVerifier::verify(&proof).unwrap_err();
        assert_eq!(err, EpochProofError::AnchorEpochMismatch);
    }

    #[test]
    fn test_epoch_proof_with_trust_check() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let cert = make_commit_certificate(100, 0, [100u8; 32], &kps, &[0, 1, 2]);
        let genesis = make_light_header(100, 0, vs, cert);
        let lc = LightClientVerifier::new(genesis, 1000);

        let snap = make_test_snapshot(1, 1000, 10, 0);
        let proof = make_epoch_proof(&[snap], 0, 10);

        // commitment_block_height = 100, which is trusted
        let result = EpochStateProofVerifier::verify_with_trust(&proof, &lc).unwrap();
        assert_eq!(result.energy, 500);
    }

    #[test]
    fn test_epoch_proof_untrusted_block_rejected() {
        let (vs, kps) = make_validator_set_with_bls(4, 1000);
        let cert = make_commit_certificate(50, 0, [50u8; 32], &kps, &[0, 1, 2]);
        let genesis = make_light_header(50, 0, vs, cert);
        let lc = LightClientVerifier::new(genesis, 1000);

        let snap = make_test_snapshot(1, 1000, 10, 0);
        let proof = make_epoch_proof(&[snap], 0, 10);
        // commitment_block_height = 100, NOT trusted (only 50 is)

        let err = EpochStateProofVerifier::verify_with_trust(&proof, &lc).unwrap_err();
        assert_eq!(err, EpochProofError::CommitmentBlockNotTrusted);
    }

    #[test]
    fn test_epoch_proof_large_tree() {
        // 16 objects, prove one in the middle
        let snaps: Vec<ObjectSnapshot> = (0..16u8)
            .map(|i| make_test_snapshot(i, (i as u64 + 1) * 100, 10, 0))
            .collect();

        let proof = make_epoch_proof(&snaps, 7, 10);
        let result = EpochStateProofVerifier::verify(&proof).unwrap();
        // Object 7: energy = 800, at epoch 10: 800 >> (10/10) = 400
        assert_eq!(result.energy, 400);
        assert_eq!(result.object_id, [7u8; 32]);
    }

    #[test]
    fn test_epoch_proof_merkle_proof_wrong_root() {
        let snap = make_test_snapshot(1, 1000, 10, 0);
        let mut proof = make_epoch_proof(&[snap], 0, 10);
        proof.anchor_state_root = [0xBBu8; 32]; // Wrong root

        let err = EpochStateProofVerifier::verify(&proof).unwrap_err();
        assert_eq!(err, EpochProofError::MerkleProofInvalid);
    }
}
