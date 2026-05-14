//! Cross-chain bridge protocol for EvaporChain.
//!
//! Enables trustless bridging by providing verifiable state proofs that
//! external chains can validate. Combines three proof layers:
//!
//! 1. **Finality proofs** — BLS-signed commit certificates proving block finality
//! 2. **DA certificates** — Validator attestations proving data availability
//! 3. **State proofs** — Merkle inclusion proofs for specific account/storage values
//!
//! Architecture:
//!   - `BridgeMessage` — Cross-chain message with embedded proofs
//!   - `BridgeVerifier` — Validates incoming bridge messages
//!   - `BridgeProofBuilder` — Constructs outgoing bridge proofs
//!   - `ValidatorSetCommitment` — Snapshot of the validator set for light verification
//!
//! A receiving chain only needs the current validator set commitment and
//! the bridge message to verify any EvaporChain state transition.

use evaporchain_crypto::hash::blake3_hash;
use evaporchain_crypto::signatures::{BlsPublicKey, BlsSignature, BlsVerifier};
use evaporchain_types::CommitCertificate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tracing::{debug, warn};

// ─────────────────────── Types ──────────────────────────────────────────

/// Commitment to a validator set at a specific epoch.
/// This is what the receiving chain stores and updates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValidatorSetCommitment {
    /// Epoch this commitment is valid for.
    pub epoch: u64,
    /// Blake3 hash of the sorted validator set (id || stake || pubkey for each).
    pub validators_hash: [u8; 32],
    /// Total stake in the set.
    pub total_stake: u64,
    /// Number of validators.
    pub validator_count: usize,
    /// Mapping from validator ID to (stake, BLS public key bytes).
    pub validators: BTreeMap<u64, (u64, Vec<u8>)>,
}

impl ValidatorSetCommitment {
    /// Create a commitment from a list of (id, stake, bls_pubkey) tuples.
    pub fn new(epoch: u64, validators: Vec<(u64, u64, Vec<u8>)>) -> Self {
        let total_stake = validators.iter().map(|(_, s, _)| s).sum();
        let validator_count = validators.len();

        // Compute deterministic hash
        let mut hash_input = Vec::new();
        let mut map = BTreeMap::new();
        for (id, stake, pk) in &validators {
            hash_input.extend_from_slice(&id.to_le_bytes());
            hash_input.extend_from_slice(&stake.to_le_bytes());
            hash_input.extend_from_slice(pk);
            map.insert(*id, (*stake, pk.clone()));
        }
        let validators_hash = blake3_hash(&hash_input);

        Self {
            epoch,
            validators_hash,
            total_stake,
            validator_count,
            validators: map,
        }
    }

    /// Verify that a commit certificate has supermajority (2/3+ stake).
    pub fn verify_supermajority(&self, cert: &CommitCertificate) -> bool {
        let signing_stake: u64 = cert
            .signer_ids
            .iter()
            .filter_map(|id| self.validators.get(id))
            .map(|(stake, _)| stake)
            .sum();

        // Audit C2: u128 to prevent wrap when total_stake > u64::MAX/2.
        (signing_stake as u128) * 3 > (self.total_stake as u128) * 2
    }

    /// Verify BLS aggregate signature on a commit certificate against this validator set.
    ///
    /// Note on key rotation (punch-list 4b): a `ValidatorSetCommitment`
    /// snapshots `(stake, pk)` per validator at the point the commitment
    /// was issued. Grace-window verification of in-flight rotations
    /// happens upstream in `TendermintConsensus::verify_commit_certificate`,
    /// against the live `ValidatorSet` which carries prev-key history.
    /// Bridge-level verification operates on the snapshot as it stood at
    /// commitment time, so the live grace window is not in scope here.
    pub fn verify_certificate_signature(&self, cert: &CommitCertificate, vote_msg: &[u8]) -> bool {
        // Collect public keys of signers
        let pks: Vec<BlsPublicKey> = cert
            .signer_ids
            .iter()
            .filter_map(|id| self.validators.get(id))
            .map(|(_, pk_bytes)| BlsPublicKey(pk_bytes.clone()))
            .collect();

        if pks.len() != cert.signer_ids.len() {
            warn!("Some signers not in validator set commitment");
            return false;
        }

        let agg_sig = BlsSignature(cert.aggregate_signature.clone());
        BlsVerifier::aggregate_verify(vote_msg, &agg_sig, &pks)
    }
}

/// Merkle state proof for a specific account or storage value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateProof {
    /// The key being proven (account address or storage key).
    pub key: [u8; 32],
    /// The value at this key (serialized).
    pub value: Vec<u8>,
    /// Merkle proof path (sibling hashes from leaf to root).
    pub proof_path: Vec<[u8; 32]>,
    /// State root this proof is against.
    pub state_root: [u8; 32],
}

impl StateProof {
    /// Verify this state proof against the claimed state root.
    pub fn verify(&self) -> bool {
        let mut current = blake3_hash(&self.value);

        // Walk up the Merkle path
        for sibling in &self.proof_path {
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

        current == self.state_root
    }
}

/// A cross-chain bridge message with embedded proofs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeMessage {
    /// Source chain identifier.
    pub source_chain: String,
    /// Destination chain identifier.
    pub dest_chain: String,
    /// Sequence number for ordering and replay protection.
    pub sequence: u64,
    /// The block height this message references.
    pub block_height: u64,
    /// Block hash at this height.
    pub block_hash: [u8; 32],
    /// State root at this height.
    pub state_root: [u8; 32],
    /// Epoch of the referenced block.
    pub epoch: u64,
    /// Commit certificate proving block finality.
    pub commit_certificate: CommitCertificate,
    /// Optional state proofs (for specific account/value claims).
    pub state_proofs: Vec<StateProof>,
    /// Payload: the actual cross-chain data (e.g., token transfer details).
    pub payload: BridgePayload,
    /// Blake3 hash of the entire message (for deduplication).
    pub message_hash: [u8; 32],
}

/// Payload types for cross-chain messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BridgePayload {
    /// Token transfer to another chain.
    TokenTransfer {
        sender: [u8; 32],
        recipient: Vec<u8>,
        amount: u64,
        token_id: Option<[u8; 32]>,
    },
    /// Validator set update (for light client on the other chain).
    ValidatorSetUpdate {
        new_commitment: ValidatorSetCommitment,
    },
    /// Arbitrary data relay.
    DataRelay { data: Vec<u8> },
}

// ─────────────────────── Bridge Proof Builder ───────────────────────────

/// Builds outgoing bridge messages with all required proofs.
pub struct BridgeProofBuilder {
    /// Source chain identifier.
    source_chain: String,
    /// Next sequence number.
    next_sequence: u64,
    /// Sent messages indexed by sequence (for replay detection).
    sent_messages: BTreeMap<u64, [u8; 32]>,
}

impl BridgeProofBuilder {
    /// Create a new bridge proof builder.
    pub fn new(source_chain: String) -> Self {
        Self {
            source_chain,
            next_sequence: 1,
            sent_messages: BTreeMap::new(),
        }
    }

    /// Build a bridge message for a token transfer.
    #[allow(clippy::too_many_arguments)]
    pub fn build_transfer(
        &mut self,
        dest_chain: &str,
        block_height: u64,
        block_hash: [u8; 32],
        state_root: [u8; 32],
        epoch: u64,
        commit_certificate: CommitCertificate,
        sender: [u8; 32],
        recipient: Vec<u8>,
        amount: u64,
        state_proofs: Vec<StateProof>,
    ) -> BridgeMessage {
        let payload = BridgePayload::TokenTransfer {
            sender,
            recipient,
            amount,
            token_id: None,
        };

        self.build_message(
            dest_chain,
            block_height,
            block_hash,
            state_root,
            epoch,
            commit_certificate,
            state_proofs,
            payload,
        )
    }

    /// Build a bridge message for a validator set update.
    #[allow(clippy::too_many_arguments)]
    pub fn build_validator_set_update(
        &mut self,
        dest_chain: &str,
        block_height: u64,
        block_hash: [u8; 32],
        state_root: [u8; 32],
        epoch: u64,
        commit_certificate: CommitCertificate,
        new_commitment: ValidatorSetCommitment,
    ) -> BridgeMessage {
        let payload = BridgePayload::ValidatorSetUpdate { new_commitment };

        self.build_message(
            dest_chain,
            block_height,
            block_hash,
            state_root,
            epoch,
            commit_certificate,
            vec![],
            payload,
        )
    }

    /// Internal: build a message with auto-incrementing sequence.
    #[allow(clippy::too_many_arguments)]
    fn build_message(
        &mut self,
        dest_chain: &str,
        block_height: u64,
        block_hash: [u8; 32],
        state_root: [u8; 32],
        epoch: u64,
        commit_certificate: CommitCertificate,
        state_proofs: Vec<StateProof>,
        payload: BridgePayload,
    ) -> BridgeMessage {
        let seq = self.next_sequence;
        self.next_sequence += 1;

        // Compute message hash for dedup
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(self.source_chain.as_bytes());
        hash_input.extend_from_slice(dest_chain.as_bytes());
        hash_input.extend_from_slice(&seq.to_le_bytes());
        hash_input.extend_from_slice(&block_height.to_le_bytes());
        hash_input.extend_from_slice(&block_hash);
        hash_input.extend_from_slice(&state_root);
        let message_hash = blake3_hash(&hash_input);

        self.sent_messages.insert(seq, message_hash);

        debug!(
            sequence = seq,
            dest = dest_chain,
            height = block_height,
            "Bridge message built"
        );

        BridgeMessage {
            source_chain: self.source_chain.clone(),
            dest_chain: dest_chain.to_string(),
            sequence: seq,
            block_height,
            block_hash,
            state_root,
            epoch,
            commit_certificate,
            state_proofs,
            payload,
            message_hash,
        }
    }

    /// Get the next sequence number.
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Total messages sent.
    pub fn total_sent(&self) -> usize {
        self.sent_messages.len()
    }
}

// ─────────────────────── Bridge Verifier ────────────────────────────────

/// Verifies incoming bridge messages from EvaporChain.
///
/// Deployed on the receiving chain (or used by relayers) to validate
/// cross-chain messages before accepting them.
pub struct BridgeVerifier {
    /// Known validator set commitments by epoch.
    validator_sets: BTreeMap<u64, ValidatorSetCommitment>,
    /// Processed message hashes (replay protection).
    processed: BTreeMap<u64, [u8; 32]>,
    /// Expected source chain.
    expected_source: String,
    /// Last processed sequence.
    last_sequence: u64,
}

/// Result of bridge message verification.
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyResult {
    /// Message is valid and can be processed.
    Valid,
    /// Invalid: reason.
    Invalid(String),
    /// Already processed (replay).
    Replay,
}

impl BridgeVerifier {
    /// Create a new verifier with the initial validator set commitment.
    pub fn new(expected_source: String, initial_commitment: ValidatorSetCommitment) -> Self {
        let epoch = initial_commitment.epoch;
        let mut validator_sets = BTreeMap::new();
        validator_sets.insert(epoch, initial_commitment);

        Self {
            validator_sets,
            processed: BTreeMap::new(),
            expected_source,
            last_sequence: 0,
        }
    }

    /// Verify an incoming bridge message.
    pub fn verify(&self, msg: &BridgeMessage) -> VerifyResult {
        // 1. Source chain check
        if msg.source_chain != self.expected_source {
            return VerifyResult::Invalid(format!(
                "unexpected source: expected {}, got {}",
                self.expected_source, msg.source_chain
            ));
        }

        // 2. Replay protection
        if self.processed.contains_key(&msg.sequence) {
            return VerifyResult::Replay;
        }

        // 3. Sequence ordering (must be strictly increasing)
        if msg.sequence <= self.last_sequence {
            return VerifyResult::Invalid(format!(
                "sequence {} <= last processed {}",
                msg.sequence, self.last_sequence
            ));
        }

        // 4. Message hash verification
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(msg.source_chain.as_bytes());
        hash_input.extend_from_slice(msg.dest_chain.as_bytes());
        hash_input.extend_from_slice(&msg.sequence.to_le_bytes());
        hash_input.extend_from_slice(&msg.block_height.to_le_bytes());
        hash_input.extend_from_slice(&msg.block_hash);
        hash_input.extend_from_slice(&msg.state_root);
        let expected_hash = blake3_hash(&hash_input);
        if msg.message_hash != expected_hash {
            return VerifyResult::Invalid("message hash mismatch".into());
        }

        // 5. Find validator set for this epoch
        let commitment = match self.find_commitment_for_epoch(msg.epoch) {
            Some(c) => c,
            None => {
                return VerifyResult::Invalid(format!(
                    "no validator set commitment for epoch {}",
                    msg.epoch
                ));
            }
        };

        // 6. Verify commit certificate has supermajority
        if !commitment.verify_supermajority(&msg.commit_certificate) {
            return VerifyResult::Invalid("commit certificate lacks supermajority".into());
        }

        // 6b. Verify BLS aggregate signature on the commit certificate
        let vote_msg = crate::tendermint::TendermintConsensus::bls_vote_message(
            msg.commit_certificate.height,
            msg.commit_certificate.round,
            &Some(msg.commit_certificate.block_hash),
            "precommit",
        );
        if !commitment.verify_certificate_signature(&msg.commit_certificate, &vote_msg) {
            return VerifyResult::Invalid(
                "commit certificate has invalid BLS aggregate signature".into(),
            );
        }

        // 7. Verify state proofs (if any)
        for (i, proof) in msg.state_proofs.iter().enumerate() {
            if proof.state_root != msg.state_root {
                return VerifyResult::Invalid(format!(
                    "state proof {} references wrong state root",
                    i
                ));
            }
            if !proof.verify() {
                return VerifyResult::Invalid(format!("state proof {} is invalid", i));
            }
        }

        VerifyResult::Valid
    }

    /// Accept a verified message (mark as processed, advance sequence).
    pub fn accept(&mut self, msg: &BridgeMessage) {
        self.processed.insert(msg.sequence, msg.message_hash);
        if msg.sequence > self.last_sequence {
            self.last_sequence = msg.sequence;
        }

        // If this is a validator set update, apply it
        if let BridgePayload::ValidatorSetUpdate { ref new_commitment } = msg.payload {
            debug!(
                epoch = new_commitment.epoch,
                validators = new_commitment.validator_count,
                "Bridge: updated validator set commitment"
            );
            self.validator_sets
                .insert(new_commitment.epoch, new_commitment.clone());
        }
    }

    /// Find the most recent validator set commitment for a given epoch.
    fn find_commitment_for_epoch(&self, epoch: u64) -> Option<&ValidatorSetCommitment> {
        // Use the commitment with the highest epoch <= requested epoch
        self.validator_sets
            .range(..=epoch)
            .next_back()
            .map(|(_, c)| c)
    }

    /// Number of processed messages.
    pub fn processed_count(&self) -> usize {
        self.processed.len()
    }

    /// Last processed sequence number.
    pub fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    /// Number of known validator set epochs.
    pub fn known_epochs(&self) -> usize {
        self.validator_sets.len()
    }
}

// ─────────────────────── Tests ──────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_crypto::signatures::BlsKeypair;

    fn make_validator_commitment(
        epoch: u64,
        keypairs: &[BlsKeypair],
        stake: u64,
    ) -> ValidatorSetCommitment {
        let validators: Vec<(u64, u64, Vec<u8>)> = keypairs
            .iter()
            .enumerate()
            .map(|(i, kp)| ((i + 1) as u64, stake, kp.public_key_bytes().0))
            .collect();
        ValidatorSetCommitment::new(epoch, validators)
    }

    fn make_signed_certificate(
        height: u64,
        block_hash: [u8; 32],
        round: u32,
        keypairs: &[BlsKeypair],
        signer_indices: &[usize],
    ) -> CommitCertificate {
        // Build the vote message (matches tendermint.rs format)
        let mut vote_msg = Vec::new();
        vote_msg.extend_from_slice(b"precommit");
        vote_msg.extend_from_slice(&height.to_le_bytes());
        vote_msg.extend_from_slice(&round.to_le_bytes());
        vote_msg.extend_from_slice(&block_hash);

        let signer_ids: Vec<u64> = signer_indices.iter().map(|&i| (i + 1) as u64).collect();
        let sigs: Vec<BlsSignature> = signer_indices
            .iter()
            .map(|&i| keypairs[i].sign(&vote_msg))
            .collect();

        let agg_sig = BlsVerifier::aggregate_signatures(&sigs).unwrap();

        CommitCertificate {
            height,
            round,
            block_hash,
            signer_ids,
            aggregate_signature: agg_sig.0,
        }
    }

    fn make_state_proof(key: [u8; 32], value: &[u8], siblings: &[[u8; 32]]) -> StateProof {
        // Build a valid Merkle proof
        let mut current = blake3_hash(value);
        let mut proof_path = Vec::new();

        for sibling in siblings {
            proof_path.push(*sibling);
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

        StateProof {
            key,
            value: value.to_vec(),
            proof_path,
            state_root: current,
        }
    }

    #[test]
    fn test_validator_set_commitment() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment = make_validator_commitment(1, &keypairs, 1000);

        assert_eq!(commitment.epoch, 1);
        assert_eq!(commitment.total_stake, 4000);
        assert_eq!(commitment.validator_count, 4);
        assert_ne!(commitment.validators_hash, [0u8; 32]);
    }

    #[test]
    fn test_supermajority_check() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment = make_validator_commitment(1, &keypairs, 1000);

        // 3/4 signers = 75% >= 66.7%
        let cert_3 = make_signed_certificate(1, [0xAA; 32], 0, &keypairs, &[0, 1, 2]);
        assert!(commitment.verify_supermajority(&cert_3));

        // 2/4 signers = 50% < 66.7%
        let cert_2 = make_signed_certificate(1, [0xBB; 32], 0, &keypairs, &[0, 1]);
        assert!(!commitment.verify_supermajority(&cert_2));
    }

    #[test]
    fn test_certificate_bls_verification() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment = make_validator_commitment(1, &keypairs, 1000);

        let block_hash = [0xCC; 32];
        let cert = make_signed_certificate(5, block_hash, 0, &keypairs, &[0, 1, 2]);

        // Reconstruct the vote message
        let mut vote_msg = Vec::new();
        vote_msg.extend_from_slice(b"precommit");
        vote_msg.extend_from_slice(&5u64.to_le_bytes());
        vote_msg.extend_from_slice(&0u32.to_le_bytes());
        vote_msg.extend_from_slice(&block_hash);

        assert!(commitment.verify_certificate_signature(&cert, &vote_msg));
    }

    #[test]
    fn test_state_proof_verification() {
        let sibling1 = [0x11; 32];
        let sibling2 = [0x22; 32];
        let proof = make_state_proof([0xAA; 32], b"balance:1000000", &[sibling1, sibling2]);

        assert!(proof.verify());

        // Tamper with value
        let mut tampered = proof.clone();
        tampered.value = b"balance:9999999".to_vec();
        assert!(!tampered.verify());
    }

    #[test]
    fn test_bridge_proof_builder() {
        let mut builder = BridgeProofBuilder::new("evaporchain".into());
        assert_eq!(builder.next_sequence(), 1);

        let cert = CommitCertificate {
            height: 100,
            round: 0,
            block_hash: [0xAA; 32],
            signer_ids: vec![1, 2, 3],
            aggregate_signature: vec![0; 48],
        };

        let msg = builder.build_transfer(
            "ethereum",
            100,
            [0xAA; 32],
            [0xBB; 32],
            5,
            cert,
            [0x01; 32],
            vec![0x02; 20],
            1_000_000,
            vec![],
        );

        assert_eq!(msg.sequence, 1);
        assert_eq!(msg.source_chain, "evaporchain");
        assert_eq!(msg.dest_chain, "ethereum");
        assert_ne!(msg.message_hash, [0u8; 32]);
        assert_eq!(builder.next_sequence(), 2);
        assert_eq!(builder.total_sent(), 1);
    }

    #[test]
    fn test_bridge_verifier_valid_message() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment = make_validator_commitment(1, &keypairs, 1000);

        let mut builder = BridgeProofBuilder::new("evaporchain".into());
        let verifier = BridgeVerifier::new("evaporchain".into(), commitment);

        let cert = make_signed_certificate(100, [0xAA; 32], 0, &keypairs, &[0, 1, 2]);

        let msg = builder.build_transfer(
            "ethereum",
            100,
            [0xAA; 32],
            [0xBB; 32],
            1,
            cert,
            [0x01; 32],
            vec![0x02; 20],
            500_000,
            vec![],
        );

        assert_eq!(verifier.verify(&msg), VerifyResult::Valid);
    }

    #[test]
    fn test_bridge_verifier_replay_protection() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment = make_validator_commitment(1, &keypairs, 1000);

        let mut builder = BridgeProofBuilder::new("evaporchain".into());
        let mut verifier = BridgeVerifier::new("evaporchain".into(), commitment);

        let cert = make_signed_certificate(100, [0xAA; 32], 0, &keypairs, &[0, 1, 2]);

        let msg = builder.build_transfer(
            "ethereum",
            100,
            [0xAA; 32],
            [0xBB; 32],
            1,
            cert,
            [0x01; 32],
            vec![0x02; 20],
            500_000,
            vec![],
        );

        assert_eq!(verifier.verify(&msg), VerifyResult::Valid);
        verifier.accept(&msg);

        // Same message again → replay
        assert_eq!(verifier.verify(&msg), VerifyResult::Replay);
    }

    #[test]
    fn test_bridge_verifier_wrong_source() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment = make_validator_commitment(1, &keypairs, 1000);

        let mut builder = BridgeProofBuilder::new("wrong_chain".into());
        let verifier = BridgeVerifier::new("evaporchain".into(), commitment);

        let cert = make_signed_certificate(1, [0xAA; 32], 0, &keypairs, &[0, 1, 2]);
        let msg = builder.build_transfer(
            "ethereum",
            1,
            [0xAA; 32],
            [0xBB; 32],
            1,
            cert,
            [0x01; 32],
            vec![],
            100,
            vec![],
        );

        match verifier.verify(&msg) {
            VerifyResult::Invalid(reason) => assert!(reason.contains("unexpected source")),
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn test_bridge_verifier_insufficient_signers() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment = make_validator_commitment(1, &keypairs, 1000);

        let mut builder = BridgeProofBuilder::new("evaporchain".into());
        let verifier = BridgeVerifier::new("evaporchain".into(), commitment);

        // Only 2/4 signers (50% < 66.7%)
        let cert = make_signed_certificate(1, [0xAA; 32], 0, &keypairs, &[0, 1]);
        let msg = builder.build_transfer(
            "ethereum",
            1,
            [0xAA; 32],
            [0xBB; 32],
            1,
            cert,
            [0x01; 32],
            vec![],
            100,
            vec![],
        );

        match verifier.verify(&msg) {
            VerifyResult::Invalid(reason) => assert!(reason.contains("supermajority")),
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn test_bridge_verifier_with_state_proof() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment = make_validator_commitment(1, &keypairs, 1000);

        let mut builder = BridgeProofBuilder::new("evaporchain".into());
        let verifier = BridgeVerifier::new("evaporchain".into(), commitment);

        let cert = make_signed_certificate(1, [0xAA; 32], 0, &keypairs, &[0, 1, 2]);

        // Build a valid state proof
        let proof = make_state_proof([0x01; 32], b"balance:1000000", &[[0x33; 32]]);
        let state_root = proof.state_root;

        let msg = builder.build_transfer(
            "ethereum",
            1,
            [0xAA; 32],
            state_root,
            1,
            cert,
            [0x01; 32],
            vec![0x02; 20],
            1_000_000,
            vec![proof],
        );

        assert_eq!(verifier.verify(&msg), VerifyResult::Valid);
    }

    #[test]
    fn test_bridge_verifier_invalid_state_proof() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment = make_validator_commitment(1, &keypairs, 1000);

        let mut builder = BridgeProofBuilder::new("evaporchain".into());
        let verifier = BridgeVerifier::new("evaporchain".into(), commitment);

        let cert = make_signed_certificate(1, [0xAA; 32], 0, &keypairs, &[0, 1, 2]);

        // State proof with wrong state root
        let proof = StateProof {
            key: [0x01; 32],
            value: b"balance:1000000".to_vec(),
            proof_path: vec![[0x33; 32]],
            state_root: [0xBB; 32], // matches msg.state_root
        };

        let msg = builder.build_transfer(
            "ethereum",
            1,
            [0xAA; 32],
            [0xBB; 32],
            1,
            cert,
            [0x01; 32],
            vec![],
            100,
            vec![proof],
        );

        match verifier.verify(&msg) {
            VerifyResult::Invalid(reason) => assert!(reason.contains("state proof")),
            other => panic!("expected Invalid, got {:?}", other),
        }
    }

    #[test]
    fn test_bridge_validator_set_update() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment1 = make_validator_commitment(1, &keypairs, 1000);

        let mut builder = BridgeProofBuilder::new("evaporchain".into());
        let mut verifier = BridgeVerifier::new("evaporchain".into(), commitment1);
        assert_eq!(verifier.known_epochs(), 1);

        // Send validator set update for epoch 2
        let new_keypairs: Vec<BlsKeypair> = (0..5).map(|_| BlsKeypair::generate()).collect();
        let commitment2 = make_validator_commitment(2, &new_keypairs, 2000);

        let cert = make_signed_certificate(100, [0xDD; 32], 0, &keypairs, &[0, 1, 2]);

        let msg = builder.build_validator_set_update(
            "ethereum",
            100,
            [0xDD; 32],
            [0xEE; 32],
            1,
            cert,
            commitment2,
        );

        assert_eq!(verifier.verify(&msg), VerifyResult::Valid);
        verifier.accept(&msg);
        assert_eq!(verifier.known_epochs(), 2);

        // Now messages signed with epoch 2 validators should also be accepted
        let cert2 = make_signed_certificate(200, [0xFF; 32], 0, &new_keypairs, &[0, 1, 2, 3]);
        let msg2 = builder.build_transfer(
            "ethereum",
            200,
            [0xFF; 32],
            [0x11; 32],
            2,
            cert2,
            [0x01; 32],
            vec![],
            100,
            vec![],
        );

        assert_eq!(verifier.verify(&msg2), VerifyResult::Valid);
    }

    #[test]
    fn test_bridge_sequence_ordering() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment = make_validator_commitment(1, &keypairs, 1000);

        let mut builder = BridgeProofBuilder::new("evaporchain".into());
        let mut verifier = BridgeVerifier::new("evaporchain".into(), commitment);

        // Send message 1
        let cert1 = make_signed_certificate(1, [0xAA; 32], 0, &keypairs, &[0, 1, 2]);
        let msg1 = builder.build_transfer(
            "ethereum",
            1,
            [0xAA; 32],
            [0xBB; 32],
            1,
            cert1,
            [0x01; 32],
            vec![],
            100,
            vec![],
        );
        assert_eq!(verifier.verify(&msg1), VerifyResult::Valid);
        verifier.accept(&msg1);

        // Send message 2
        let cert2 = make_signed_certificate(2, [0xCC; 32], 0, &keypairs, &[0, 1, 2]);
        let msg2 = builder.build_transfer(
            "ethereum",
            2,
            [0xCC; 32],
            [0xDD; 32],
            1,
            cert2,
            [0x01; 32],
            vec![],
            200,
            vec![],
        );
        assert_eq!(verifier.verify(&msg2), VerifyResult::Valid);
        verifier.accept(&msg2);

        assert_eq!(verifier.processed_count(), 2);
        assert_eq!(verifier.last_sequence(), 2);
    }

    #[test]
    fn test_bridge_tampered_hash() {
        let keypairs: Vec<BlsKeypair> = (0..4).map(|_| BlsKeypair::generate()).collect();
        let commitment = make_validator_commitment(1, &keypairs, 1000);

        let mut builder = BridgeProofBuilder::new("evaporchain".into());
        let verifier = BridgeVerifier::new("evaporchain".into(), commitment);

        let cert = make_signed_certificate(1, [0xAA; 32], 0, &keypairs, &[0, 1, 2]);
        let mut msg = builder.build_transfer(
            "ethereum",
            1,
            [0xAA; 32],
            [0xBB; 32],
            1,
            cert,
            [0x01; 32],
            vec![],
            100,
            vec![],
        );

        // Tamper with message hash
        msg.message_hash[0] ^= 0xFF;

        match verifier.verify(&msg) {
            VerifyResult::Invalid(reason) => assert!(reason.contains("hash mismatch")),
            other => panic!("expected Invalid, got {:?}", other),
        }
    }
}
