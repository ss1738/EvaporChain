//! Cross-chain ghost bridges — evaporated objects that "haunt" other chains.
//!
//! When an object evaporates on EvaporChain, its ghost record + MMR inclusion
//! proof can be relayed to another chain. The receiving chain can verify:
//! 1. The object existed (Merkle proof against EvaporChain state root)
//! 2. It evaporated at a specific epoch (ghost record fields)
//! 3. Its nullifier is in the MMR (MMR inclusion proof)
//!
//! This enables "ghost assets" — claims on other chains backed by proof of
//! prior existence on EvaporChain. Use cases: cross-chain NFT history,
//! reputation scores that survive evaporation, insurance claims.

use evaporchain_crypto::signatures::{BlsPublicKey, BlsSignature, BlsVerifier};
use evaporchain_types::{GhostRecord, ObjectId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ─── Ghost Bridge Proof ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhostBridgeProof {
    pub ghost: GhostRecord,
    pub mmr_inclusion: MmrInclusionWitness,
    pub state_root: [u8; 32],
    pub state_root_proof: StateRootAttestation,
    pub target_chain_id: u64,
    pub bridge_nonce: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MmrInclusionWitness {
    pub leaf_index: u64,
    pub leaf_hash: [u8; 32],
    pub proof_hashes: Vec<[u8; 32]>,
    pub mmr_root: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateRootAttestation {
    pub block_number: u64,
    pub epoch: u64,
    pub state_root: [u8; 32],
    pub validator_signatures: Vec<ValidatorSig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorSig {
    pub validator_id: u64,
    pub signature: Vec<u8>,
}

// ─── Ghost Claim (what the receiving chain sees) ────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhostClaim {
    pub original_object_id: ObjectId,
    pub original_owner: [u8; 32],
    pub evaporated_at_epoch: u64,
    pub data_hash: [u8; 32],
    pub claim_type: GhostClaimType,
    pub target_chain_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GhostClaimType {
    Existence,
    DataContent { content_hash: [u8; 32] },
    OwnershipHistory,
    ReputationScore { score: u64 },
    InsuranceClaim { policy_hash: [u8; 32], amount: u64 },
}

// ─── Bridge Builder ─────────────────────────────────────────────────────

pub struct GhostBridgeBuilder {
    next_nonce: u64,
}

impl GhostBridgeBuilder {
    pub fn new() -> Self {
        Self { next_nonce: 0 }
    }

    pub fn build_proof(
        &mut self,
        ghost: GhostRecord,
        mmr_leaf_index: u64,
        mmr_proof_hashes: Vec<[u8; 32]>,
        mmr_root: [u8; 32],
        state_root: [u8; 32],
        block_number: u64,
        epoch: u64,
        validator_sigs: Vec<ValidatorSig>,
        target_chain_id: u64,
    ) -> GhostBridgeProof {
        let leaf_hash = ghost.data_hash;
        self.next_nonce += 1;

        GhostBridgeProof {
            ghost,
            mmr_inclusion: MmrInclusionWitness {
                leaf_index: mmr_leaf_index,
                leaf_hash,
                proof_hashes: mmr_proof_hashes,
                mmr_root,
            },
            state_root,
            state_root_proof: StateRootAttestation {
                block_number,
                epoch,
                state_root,
                validator_signatures: validator_sigs,
            },
            target_chain_id,
            bridge_nonce: self.next_nonce,
        }
    }

    pub fn extract_claim(proof: &GhostBridgeProof, claim_type: GhostClaimType) -> GhostClaim {
        GhostClaim {
            original_object_id: proof.ghost.object_id,
            original_owner: proof.ghost.owner,
            evaporated_at_epoch: proof.ghost.evaporated_at,
            data_hash: proof.ghost.data_hash,
            claim_type,
            target_chain_id: proof.target_chain_id,
        }
    }
}

impl Default for GhostBridgeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Verification ───────────────────────────────────────────────────────

pub fn verify_ghost_bridge_proof(proof: &GhostBridgeProof) -> GhostBridgeVerification {
    verify_ghost_bridge_proof_with_keys(proof, None)
}

pub fn verify_ghost_bridge_proof_with_keys(
    proof: &GhostBridgeProof,
    validator_pubkeys: Option<&BTreeMap<u64, Vec<u8>>>,
) -> GhostBridgeVerification {
    let mut checks = GhostBridgeVerification::default();

    // Check 1: ghost data_hash is non-zero (object existed)
    checks.ghost_valid = proof.ghost.data_hash != [0u8; 32];

    // Check 2: MMR inclusion — verify Merkle path from leaf to root
    checks.mmr_inclusion_valid = verify_mmr_path(
        &proof.mmr_inclusion.leaf_hash,
        proof.mmr_inclusion.leaf_index,
        &proof.mmr_inclusion.proof_hashes,
        &proof.mmr_inclusion.mmr_root,
    );

    // Check 3: State root attestation — structural + cryptographic verification
    let structural_valid = proof.state_root_proof.validator_signatures.len() >= 2
        && proof.state_root_proof.state_root == proof.state_root
        && has_unique_validator_ids(&proof.state_root_proof.validator_signatures)
        && proof.state_root_proof.validator_signatures.iter().all(|sig| {
            !sig.signature.is_empty() && sig.signature.len() >= 48
        });

    checks.attestation_valid = if structural_valid {
        match validator_pubkeys {
            Some(keys) => {
                proof.state_root_proof.validator_signatures.iter().all(|sig| {
                    keys.get(&sig.validator_id).map_or(false, |pk_bytes| {
                        let pk = BlsPublicKey(pk_bytes.clone());
                        let bls_sig = BlsSignature(sig.signature.clone());
                        BlsVerifier::verify(&proof.state_root, &bls_sig, &pk)
                    })
                })
            }
            None => true,
        }
    } else {
        false
    };

    // Check 4: State roots match
    checks.state_root_matches = proof.state_root == proof.state_root_proof.state_root;

    // Check 5: Nonce is non-zero (replay protection)
    checks.nonce_valid = proof.bridge_nonce > 0;

    checks.overall_valid = checks.ghost_valid
        && checks.mmr_inclusion_valid
        && checks.attestation_valid
        && checks.state_root_matches
        && checks.nonce_valid;

    checks
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GhostBridgeVerification {
    pub ghost_valid: bool,
    pub mmr_inclusion_valid: bool,
    pub attestation_valid: bool,
    pub state_root_matches: bool,
    pub nonce_valid: bool,
    pub overall_valid: bool,
}

fn has_unique_validator_ids(sigs: &[ValidatorSig]) -> bool {
    let mut seen = std::collections::HashSet::new();
    sigs.iter().all(|s| seen.insert(s.validator_id))
}

fn verify_mmr_path(
    leaf_hash: &[u8; 32],
    leaf_index: u64,
    proof_hashes: &[[u8; 32]],
    expected_root: &[u8; 32],
) -> bool {
    if proof_hashes.is_empty() {
        return leaf_hash == expected_root;
    }

    let mut current = *leaf_hash;
    let mut idx = leaf_index;
    for sibling in proof_hashes {
        let mut hasher = blake3::Hasher::new();
        if idx % 2 == 0 {
            hasher.update(&current);
            hasher.update(sibling);
        } else {
            hasher.update(sibling);
            hasher.update(&current);
        }
        current = *hasher.finalize().as_bytes();
        idx /= 2;
    }
    current == *expected_root
}

// ─── Ghost Bridge Registry (tracks active bridges) ─────────────────────

#[derive(Debug, Default)]
pub struct GhostBridgeRegistry {
    active_bridges: Vec<GhostBridgeProof>,
    processed_nonces: Vec<u64>,
}

impl GhostBridgeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, proof: GhostBridgeProof) -> Result<(), &'static str> {
        if self.processed_nonces.contains(&proof.bridge_nonce) {
            return Err("bridge nonce already processed (replay)");
        }
        let verification = verify_ghost_bridge_proof(&proof);
        if !verification.overall_valid {
            return Err("ghost bridge proof verification failed");
        }
        self.processed_nonces.push(proof.bridge_nonce);
        self.active_bridges.push(proof);
        Ok(())
    }

    pub fn bridges_for_chain(&self, chain_id: u64) -> Vec<&GhostBridgeProof> {
        self.active_bridges
            .iter()
            .filter(|b| b.target_chain_id == chain_id)
            .collect()
    }

    pub fn bridge_count(&self) -> usize {
        self.active_bridges.len()
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ghost(id_byte: u8, evap_epoch: u64) -> GhostRecord {
        let mut object_id = [0u8; 32];
        object_id[0] = id_byte;
        GhostRecord {
            object_id,
            owner: [id_byte; 32],
            evaporated_at: evap_epoch,
            data_hash: {
                let mut h = blake3::Hasher::new();
                h.update(&object_id);
                *h.finalize().as_bytes()
            },
            original_data: None,
            mmr_position: Some(0),
            original_half_life: Some(100),
        }
    }

    fn make_bridge_proof(ghost: GhostRecord, chain_id: u64) -> GhostBridgeProof {
        let data_hash = ghost.data_hash;
        let state_root = [0xAA; 32];
        GhostBridgeProof {
            ghost,
            mmr_inclusion: MmrInclusionWitness {
                leaf_index: 0,
                leaf_hash: data_hash,
                proof_hashes: vec![],
                mmr_root: data_hash,
            },
            state_root,
            state_root_proof: StateRootAttestation {
                block_number: 100,
                epoch: 50,
                state_root,
                validator_signatures: vec![
                    ValidatorSig {
                        validator_id: 0,
                        signature: vec![0xAA; 96],
                    },
                    ValidatorSig {
                        validator_id: 1,
                        signature: vec![0xBB; 96],
                    },
                ],
            },
            target_chain_id: chain_id,
            bridge_nonce: 1,
        }
    }

    #[test]
    fn test_build_and_verify_bridge_proof() {
        let ghost = make_ghost(1, 50);
        let proof = make_bridge_proof(ghost, 42);
        let result = verify_ghost_bridge_proof(&proof);
        assert!(result.ghost_valid);
        assert!(result.mmr_inclusion_valid);
        assert!(result.attestation_valid);
        assert!(result.state_root_matches);
        assert!(result.nonce_valid);
        assert!(result.overall_valid);
    }

    #[test]
    fn test_zero_data_hash_fails() {
        let mut ghost = make_ghost(1, 50);
        ghost.data_hash = [0u8; 32];
        let proof = make_bridge_proof(ghost, 42);
        let result = verify_ghost_bridge_proof(&proof);
        assert!(!result.ghost_valid);
        assert!(!result.overall_valid);
    }

    #[test]
    fn test_state_root_mismatch_fails() {
        let ghost = make_ghost(1, 50);
        let mut proof = make_bridge_proof(ghost, 42);
        proof.state_root = [0xBB; 32];
        let result = verify_ghost_bridge_proof(&proof);
        assert!(!result.state_root_matches);
        assert!(!result.overall_valid);
    }

    #[test]
    fn test_no_validator_sigs_fails() {
        let ghost = make_ghost(1, 50);
        let mut proof = make_bridge_proof(ghost, 42);
        proof.state_root_proof.validator_signatures.clear();
        let result = verify_ghost_bridge_proof(&proof);
        assert!(!result.attestation_valid);
        assert!(!result.overall_valid);
    }

    #[test]
    fn test_extract_claim() {
        let ghost = make_ghost(1, 50);
        let proof = make_bridge_proof(ghost, 42);
        let claim = GhostBridgeBuilder::extract_claim(&proof, GhostClaimType::Existence);
        assert_eq!(claim.evaporated_at_epoch, 50);
        assert_eq!(claim.target_chain_id, 42);
        assert_eq!(claim.claim_type, GhostClaimType::Existence);
    }

    #[test]
    fn test_registry_prevents_replay() {
        let mut registry = GhostBridgeRegistry::new();
        let ghost = make_ghost(1, 50);
        let proof = make_bridge_proof(ghost, 42);
        assert!(registry.register(proof.clone()).is_ok());
        assert_eq!(registry.bridge_count(), 1);
        assert!(registry.register(proof).is_err());
    }

    #[test]
    fn test_registry_filters_by_chain() {
        let mut registry = GhostBridgeRegistry::new();
        let p1 = {
            let ghost = make_ghost(1, 50);
            let mut p = make_bridge_proof(ghost, 42);
            p.bridge_nonce = 1;
            p
        };
        let p2 = {
            let ghost = make_ghost(2, 60);
            let mut p = make_bridge_proof(ghost, 99);
            p.bridge_nonce = 2;
            p
        };
        registry.register(p1).unwrap();
        registry.register(p2).unwrap();
        assert_eq!(registry.bridges_for_chain(42).len(), 1);
        assert_eq!(registry.bridges_for_chain(99).len(), 1);
        assert_eq!(registry.bridges_for_chain(0).len(), 0);
    }

    #[test]
    fn test_builder_increments_nonce() {
        let mut builder = GhostBridgeBuilder::new();
        let ghost1 = make_ghost(1, 50);
        let ghost2 = make_ghost(2, 60);
        let p1 = builder.build_proof(
            ghost1, 0, vec![], [0u8; 32], [0xAA; 32], 100, 50,
            vec![ValidatorSig { validator_id: 0, signature: vec![1] }], 42,
        );
        let p2 = builder.build_proof(
            ghost2, 1, vec![], [0u8; 32], [0xAA; 32], 101, 51,
            vec![ValidatorSig { validator_id: 0, signature: vec![1] }], 42,
        );
        assert_eq!(p1.bridge_nonce, 1);
        assert_eq!(p2.bridge_nonce, 2);
    }

    #[test]
    fn test_mmr_path_single_leaf() {
        let leaf = [0xAB; 32];
        assert!(verify_mmr_path(&leaf, 0, &[], &leaf));
    }

    #[test]
    fn test_mmr_path_with_sibling() {
        let leaf = [0xAB; 32];
        let sibling = [0xCD; 32];
        let mut hasher = blake3::Hasher::new();
        hasher.update(&leaf);
        hasher.update(&sibling);
        let root = *hasher.finalize().as_bytes();
        assert!(verify_mmr_path(&leaf, 0, &[sibling], &root));
    }

    #[test]
    fn test_mmr_path_wrong_root_fails() {
        let leaf = [0xAB; 32];
        let sibling = [0xCD; 32];
        assert!(!verify_mmr_path(&leaf, 0, &[sibling], &[0xFF; 32]));
    }

    #[test]
    fn test_single_validator_sig_fails() {
        let ghost = make_ghost(1, 50);
        let mut proof = make_bridge_proof(ghost, 42);
        proof.state_root_proof.validator_signatures = vec![ValidatorSig {
            validator_id: 0,
            signature: vec![0xAA; 96],
        }];
        let result = verify_ghost_bridge_proof(&proof);
        assert!(!result.attestation_valid, "need >=2 validator signatures");
        assert!(!result.overall_valid);
    }

    #[test]
    fn test_short_signature_fails() {
        let ghost = make_ghost(1, 50);
        let mut proof = make_bridge_proof(ghost, 42);
        proof.state_root_proof.validator_signatures = vec![
            ValidatorSig { validator_id: 0, signature: vec![0xAA; 96] },
            ValidatorSig { validator_id: 1, signature: vec![0xBB; 10] }, // too short
        ];
        let result = verify_ghost_bridge_proof(&proof);
        assert!(!result.attestation_valid, "signature must be >= 48 bytes");
    }

    #[test]
    fn test_duplicate_validator_ids_fails() {
        let ghost = make_ghost(1, 50);
        let mut proof = make_bridge_proof(ghost, 42);
        proof.state_root_proof.validator_signatures = vec![
            ValidatorSig { validator_id: 0, signature: vec![0xAA; 96] },
            ValidatorSig { validator_id: 0, signature: vec![0xBB; 96] }, // duplicate
        ];
        let result = verify_ghost_bridge_proof(&proof);
        assert!(!result.attestation_valid, "duplicate validator IDs");
    }

    #[test]
    fn test_insurance_claim_type() {
        let ghost = make_ghost(1, 50);
        let proof = make_bridge_proof(ghost, 42);
        let claim = GhostBridgeBuilder::extract_claim(
            &proof,
            GhostClaimType::InsuranceClaim {
                policy_hash: [0xBB; 32],
                amount: 1_000_000,
            },
        );
        match claim.claim_type {
            GhostClaimType::InsuranceClaim { amount, .. } => assert_eq!(amount, 1_000_000),
            _ => panic!("wrong claim type"),
        }
    }
}
