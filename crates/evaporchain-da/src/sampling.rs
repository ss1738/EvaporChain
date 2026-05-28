//! Data availability sampling (DAS).
//!
//! Light clients can verify data availability by requesting random samples
//! from block producers. Each sample includes a Merkle proof against the
//! shard commitment root.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::erasure::Shard;

/// Domain-separation tag for the DA-sample seed (AUDIT_2026_05_13 H7).
/// Versioned so a future Stage-B per-validator-VRF binding can ship
/// with `:v2\0` and run alongside v1 during the cutover.
pub const DA_SAMPLE_SEED_DST: &[u8] = b"evaporchain:da-sample-seed:v1\0";

#[derive(Error, Debug)]
pub enum SamplingError {
    #[error("shard index {index} out of range (total: {total})")]
    IndexOutOfRange { index: usize, total: usize },
    #[error("proof verification failed for shard {0}")]
    ProofVerificationFailed(usize),
    #[error("no shards available")]
    NoShards,
    #[error("insufficient samples: got {got}, need {need}")]
    InsufficientSamples { got: usize, need: usize },
}

/// A query for a specific shard sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleQuery {
    /// Block number being sampled.
    pub block_number: u64,
    /// Shard index to retrieve.
    pub shard_index: usize,
}

/// Response to a sample query, including the shard and its proof.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleResponse {
    /// The requested shard.
    pub shard: Shard,
    /// Merkle proof for this shard against the commitment root.
    pub proof: MerkleProof,
    /// Optional BLS/hybrid attestation signature from the responding peer.
    /// Signs BLAKE3(commitment_root || shard_index_le_bytes || shard_hash).
    #[serde(default)]
    pub attestation_signature: Option<Vec<u8>>,
    /// Public key of the attesting peer (for signature verification).
    #[serde(default)]
    pub attester_public_key: Option<Vec<u8>>,
}

/// Merkle proof for a shard in the commitment tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    /// Sibling hashes from leaf to root.
    pub siblings: Vec<[u8; 32]>,
    /// Index of the leaf in the tree.
    pub leaf_index: usize,
    /// Root hash this proof verifies against.
    pub root: [u8; 32],
}

/// A complete DA proof: commitment root + shard proofs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DAProof {
    /// Merkle root over all shard hashes.
    pub commitment_root: [u8; 32],
    /// Number of total shards.
    pub total_shards: usize,
    /// Number of data shards.
    pub data_shards: usize,
}

/// Result of batch-verifying multiple Merkle proofs.
#[derive(Debug, Clone)]
pub struct BatchVerifyResult {
    /// True if every proof verified successfully.
    pub all_valid: bool,
    /// Number of proofs that were verified (including invalid ones).
    pub verified_count: usize,
    /// Indices (into the original slice) of proofs that failed verification.
    pub invalid_indices: Vec<usize>,
}

/// Batch-verify a slice of Merkle proofs.
///
/// Proofs are grouped by root hash so that proofs against the same tree are
/// checked together. Verification short-circuits on the first invalid proof
/// within each group: once a failure is found the remaining proofs in that
/// group are skipped, but other groups are still checked so the caller gets
/// a complete picture of which indices failed.
pub fn batch_verify_proofs(
    proofs: &[&MerkleProof],
    shard_hashes: &[[u8; 32]],
) -> BatchVerifyResult {
    if proofs.len() != shard_hashes.len() {
        return BatchVerifyResult {
            all_valid: false,
            verified_count: 0,
            invalid_indices: (0..proofs.len()).collect(),
        };
    }

    // Group proof indices by root hash.
    let mut groups: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
    for (i, proof) in proofs.iter().enumerate() {
        groups.entry(proof.root).or_default().push(i);
    }

    let mut invalid_indices: Vec<usize> = Vec::new();
    let mut verified_count: usize = 0;

    for indices in groups.values() {
        for &idx in indices {
            verified_count += 1;

            // Reconstruct root from leaf + siblings.
            let mut current = shard_hashes[idx];
            let mut leaf_idx = proofs[idx].leaf_index;
            for sibling in &proofs[idx].siblings {
                current = if leaf_idx.is_multiple_of(2) {
                    DASampler::hash_pair(&current, sibling)
                } else {
                    DASampler::hash_pair(sibling, &current)
                };
                leaf_idx /= 2;
            }

            if current != proofs[idx].root {
                invalid_indices.push(idx);
                // Short-circuit: skip remaining proofs in this root group.
                break;
            }
        }
    }

    invalid_indices.sort_unstable();

    BatchVerifyResult {
        all_valid: invalid_indices.is_empty(),
        verified_count,
        invalid_indices,
    }
}

/// Data availability sampler — generates commitments and proofs,
/// verifies sample responses.
pub struct DASampler;

impl DASampler {
    /// Build a Merkle tree over shard hashes and return the commitment root.
    pub fn compute_commitment(shards: &[Shard]) -> Result<DAProof, SamplingError> {
        if shards.is_empty() {
            return Err(SamplingError::NoShards);
        }

        let root = Self::merkle_root(&shards.iter().map(|s| s.hash).collect::<Vec<_>>());

        Ok(DAProof {
            commitment_root: root,
            total_shards: shards.len(),
            data_shards: shards.len() / 2, // Assume 50% redundancy by default
        })
    }

    /// Build commitment with explicit data_shards count.
    pub fn compute_commitment_with_config(
        shards: &[Shard],
        data_shards: usize,
    ) -> Result<DAProof, SamplingError> {
        if shards.is_empty() {
            return Err(SamplingError::NoShards);
        }

        let root = Self::merkle_root(&shards.iter().map(|s| s.hash).collect::<Vec<_>>());

        Ok(DAProof {
            commitment_root: root,
            total_shards: shards.len(),
            data_shards,
        })
    }

    /// Generate a Merkle proof for a specific shard.
    pub fn generate_proof(
        shards: &[Shard],
        shard_index: usize,
    ) -> Result<MerkleProof, SamplingError> {
        if shard_index >= shards.len() {
            return Err(SamplingError::IndexOutOfRange {
                index: shard_index,
                total: shards.len(),
            });
        }

        let hashes: Vec<[u8; 32]> = shards.iter().map(|s| s.hash).collect();
        let (root, siblings) = Self::merkle_proof(&hashes, shard_index);

        Ok(MerkleProof {
            siblings,
            leaf_index: shard_index,
            root,
        })
    }

    /// Verify a Merkle proof for a shard.
    pub fn verify_proof(shard: &Shard, proof: &MerkleProof) -> bool {
        let mut current = shard.hash;
        let mut index = proof.leaf_index;

        for sibling in &proof.siblings {
            current = if index.is_multiple_of(2) {
                Self::hash_pair(&current, sibling)
            } else {
                Self::hash_pair(sibling, &current)
            };
            index /= 2;
        }

        current == proof.root
    }

    /// Generate random sample queries for a block.
    ///
    /// **AUDIT_2026_05_13 H7 Stage A**: callers MUST construct `seed`
    /// via [`build_da_sample_seed_v1`], which binds the seed to
    /// `data_root` (the producer's commitment to the entire shard
    /// matrix). Pre-Stage-A callers used `b"da-sample" || block.number
    /// || validator_id`, both publicly known before block production —
    /// a Byzantine producer could pre-compute every validator's queries
    /// and ensure only the desired subset of shards was available.
    ///
    /// Binding to `data_root` makes that grinding cost exponential in
    /// `(available_shards / total_shards)^(samples × validators)`. For
    /// modest withhold rates against typical validator sets, the cost
    /// is computationally infeasible.
    ///
    /// **Stage B follow-up**: per-validator VRF where each validator
    /// signs a VRF over `(block.data_root, block.number)` and reveals
    /// the output in their `DAAttestation`. Removes the grinding window
    /// entirely (producer cannot precompute without the validator's
    /// private key). Requires `DAAttestation` wire-format extension.
    pub fn generate_queries(
        block_number: u64,
        total_shards: usize,
        num_samples: usize,
        seed: &[u8],
    ) -> Vec<SampleQuery> {
        if total_shards == 0 {
            return Vec::new();
        }
        let mut queries = Vec::with_capacity(num_samples);
        for i in 0..num_samples {
            let mut hasher = blake3::Hasher::new();
            hasher.update(seed);
            hasher.update(&block_number.to_le_bytes());
            hasher.update(&(i as u64).to_le_bytes());
            let hash = hasher.finalize();
            let bytes = hash.as_bytes();
            let shard_index =
                (u64::from_le_bytes(bytes[0..8].try_into().unwrap()) as usize) % total_shards;

            queries.push(SampleQuery {
                block_number,
                shard_index,
            });
        }
        queries
    }

    /// Verify a set of sample responses against a DA proof.
    /// Returns Ok(true) if all samples are valid.
    pub fn verify_samples(
        proof: &DAProof,
        responses: &[SampleResponse],
        min_samples: usize,
    ) -> Result<bool, SamplingError> {
        if responses.len() < min_samples {
            return Err(SamplingError::InsufficientSamples {
                got: responses.len(),
                need: min_samples,
            });
        }

        for response in responses {
            // Verify shard hash
            let computed_hash: [u8; 32] = blake3::hash(&response.shard.data).into();
            if computed_hash != response.shard.hash {
                return Ok(false);
            }

            // Verify Merkle proof
            if response.proof.root != proof.commitment_root {
                return Ok(false);
            }

            if !Self::verify_proof(&response.shard, &response.proof) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Batch-verify a set of sample responses against a DA proof.
    ///
    /// Unlike `verify_samples` which returns a simple bool, this returns a
    /// `BatchVerifyResult` with per-index failure information — useful for
    /// light clients verifying 100+ samples and wanting to know *which*
    /// responses are bad rather than just "something failed".
    pub fn verify_samples_batch(
        proof: &DAProof,
        responses: &[SampleResponse],
        min_samples: usize,
    ) -> Result<BatchVerifyResult, SamplingError> {
        if responses.len() < min_samples {
            return Err(SamplingError::InsufficientSamples {
                got: responses.len(),
                need: min_samples,
            });
        }

        let mut invalid_indices: Vec<usize> = Vec::new();
        let mut verified_count: usize = 0;

        // Pre-check: all responses must reference the correct commitment root,
        // and shard hashes must match their data.
        for (i, response) in responses.iter().enumerate() {
            verified_count += 1;

            // Check shard data integrity.
            let computed_hash: [u8; 32] = blake3::hash(&response.shard.data).into();
            if computed_hash != response.shard.hash {
                invalid_indices.push(i);
                continue;
            }

            // Check root matches expected commitment.
            if response.proof.root != proof.commitment_root {
                invalid_indices.push(i);
                continue;
            }

            // Verify Merkle proof.
            if !Self::verify_proof(&response.shard, &response.proof) {
                invalid_indices.push(i);
                continue;
            }
        }

        Ok(BatchVerifyResult {
            all_valid: invalid_indices.is_empty(),
            verified_count,
            invalid_indices,
        })
    }

    /// Build the canonical DA-sample seed for a `(block, validator)` pair.
    /// AUDIT_2026_05_13 H7 Stage A closure.
    ///
    /// The seed binds to the producer's `data_root` commitment, which
    /// commits to the entire shard matrix. The producer must choose
    /// `data_root` BEFORE knowing which shards each validator will
    /// sample — they cannot grind it to fit a withheld subset without
    /// exponential work in (available_shards / total_shards)^(samples ×
    /// validators).
    ///
    /// Stage B will additionally bind to a per-validator VRF output
    /// (revealed in `DAAttestation`) to remove the grinding window
    /// entirely. See `generate_queries` docstring.
    ///
    /// All callers — the production tendermint node, the consensus
    /// light client, and tests — should go through this builder so the
    /// seed shape is grep-checkable and future entropy additions are
    /// one-edit migrations.
    pub fn build_da_sample_seed_v1(
        block_number: u64,
        data_root: &[u8; 32],
        validator_id: u64,
    ) -> Vec<u8> {
        let mut seed = Vec::with_capacity(DA_SAMPLE_SEED_DST.len() + 8 + 32 + 8);
        seed.extend_from_slice(DA_SAMPLE_SEED_DST);
        seed.extend_from_slice(&block_number.to_le_bytes());
        seed.extend_from_slice(data_root);
        seed.extend_from_slice(&validator_id.to_le_bytes());
        seed
    }

    /// Build the canonical attestation message for a sample response.
    /// The attester signs this to prove they hold the shard data.
    pub fn attestation_message(
        commitment_root: &[u8; 32],
        shard_index: usize,
        shard_hash: &[u8; 32],
    ) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"evaporchain-da-attestation-v1");
        hasher.update(commitment_root);
        hasher.update(&(shard_index as u64).to_le_bytes());
        hasher.update(shard_hash);
        hasher.finalize().into()
    }

    /// Verify attestation signatures on sample responses.
    /// Returns indices of responses with invalid or missing attestations.
    pub fn verify_attestations(proof: &DAProof, responses: &[SampleResponse]) -> Vec<usize> {
        let mut invalid = Vec::new();
        for (i, resp) in responses.iter().enumerate() {
            match (&resp.attestation_signature, &resp.attester_public_key) {
                (Some(sig), Some(pk)) => {
                    let msg = Self::attestation_message(
                        &proof.commitment_root,
                        resp.proof.leaf_index,
                        &resp.shard.hash,
                    );
                    if !<evaporchain_crypto::signatures::HybridVerifier as evaporchain_crypto::signatures::Verifier>::verify(&msg, sig, pk) {
                        invalid.push(i);
                    }
                }
                (None, _) | (_, None) => {
                    invalid.push(i);
                }
            }
        }
        invalid
    }

    // ── Internal Merkle helpers ──

    fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(left);
        hasher.update(right);
        hasher.finalize().into()
    }

    fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
        if leaves.is_empty() {
            return [0u8; 32];
        }
        if leaves.len() == 1 {
            return leaves[0];
        }

        // Pad to power of 2
        let mut layer: Vec<[u8; 32]> = leaves.to_vec();
        while layer.len().count_ones() != 1 {
            layer.push([0u8; 32]);
        }

        while layer.len() > 1 {
            let mut next = Vec::with_capacity(layer.len() / 2);
            for pair in layer.chunks(2) {
                next.push(Self::hash_pair(&pair[0], &pair[1]));
            }
            layer = next;
        }

        layer[0]
    }

    fn merkle_proof(leaves: &[[u8; 32]], index: usize) -> ([u8; 32], Vec<[u8; 32]>) {
        if leaves.is_empty() {
            return ([0u8; 32], vec![]);
        }

        // Pad to power of 2
        let mut layer: Vec<[u8; 32]> = leaves.to_vec();
        while layer.len().count_ones() != 1 {
            layer.push([0u8; 32]);
        }

        let mut siblings = Vec::new();
        let mut current_index = index;

        while layer.len() > 1 {
            // Record sibling
            let sibling_index = if current_index.is_multiple_of(2) {
                current_index + 1
            } else {
                current_index - 1
            };
            if sibling_index < layer.len() {
                siblings.push(layer[sibling_index]);
            } else {
                siblings.push([0u8; 32]);
            }

            // Go up one level
            let mut next = Vec::with_capacity(layer.len() / 2);
            for pair in layer.chunks(2) {
                next.push(Self::hash_pair(&pair[0], &pair[1]));
            }
            layer = next;
            current_index /= 2;
        }

        (layer[0], siblings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::erasure::{ErasureConfig, ErasureEncoder};

    fn make_test_shards() -> Vec<Shard> {
        let encoder = ErasureEncoder::new(ErasureConfig {
            data_shards: 4,
            parity_shards: 4,
        })
        .unwrap();
        let data = b"Test data for DA sampling verification";
        encoder.encode(data).unwrap().shards
    }

    #[test]
    fn test_compute_commitment() {
        let shards = make_test_shards();
        let proof = DASampler::compute_commitment(&shards).unwrap();
        assert_eq!(proof.total_shards, 8);
        assert_ne!(proof.commitment_root, [0u8; 32]);
    }

    #[test]
    fn test_generate_and_verify_proof() {
        let shards = make_test_shards();

        for i in 0..shards.len() {
            let proof = DASampler::generate_proof(&shards, i).unwrap();
            assert!(DASampler::verify_proof(&shards[i], &proof));
        }
    }

    #[test]
    fn test_proof_fails_for_wrong_shard() {
        let shards = make_test_shards();
        let proof = DASampler::generate_proof(&shards, 0).unwrap();

        // Verify with shard 1 instead of shard 0 — should fail
        assert!(!DASampler::verify_proof(&shards[1], &proof));
    }

    #[test]
    fn test_generate_queries_deterministic() {
        let seed = b"test-seed";
        let q1 = DASampler::generate_queries(42, 8, 4, seed);
        let q2 = DASampler::generate_queries(42, 8, 4, seed);

        for (a, b) in q1.iter().zip(q2.iter()) {
            assert_eq!(a.shard_index, b.shard_index);
            assert_eq!(a.block_number, b.block_number);
        }
    }

    #[test]
    fn test_generate_queries_bounded() {
        let queries = DASampler::generate_queries(1, 8, 100, b"seed");
        for q in &queries {
            assert!(q.shard_index < 8);
        }
    }

    #[test]
    fn test_full_sampling_flow() {
        let shards = make_test_shards();
        let da_proof = DASampler::compute_commitment_with_config(&shards, 4).unwrap();

        // Generate sample queries
        let queries = DASampler::generate_queries(1, 8, 4, b"light-client-seed");

        // Respond to queries
        let responses: Vec<SampleResponse> = queries
            .iter()
            .map(|q| {
                let shard = shards[q.shard_index].clone();
                let proof = DASampler::generate_proof(&shards, q.shard_index).unwrap();
                SampleResponse {
                    shard,
                    proof,
                    attestation_signature: None,
                    attester_public_key: None,
                }
            })
            .collect();

        // Verify all samples
        let result = DASampler::verify_samples(&da_proof, &responses, 4).unwrap();
        assert!(result);
    }

    #[test]
    fn test_insufficient_samples() {
        let shards = make_test_shards();
        let da_proof = DASampler::compute_commitment(&shards).unwrap();

        let result = DASampler::verify_samples(&da_proof, &[], 4);
        assert!(result.is_err());
    }

    #[test]
    fn test_tampered_shard_fails_verification() {
        let shards = make_test_shards();
        let da_proof = DASampler::compute_commitment(&shards).unwrap();

        let mut bad_shard = shards[0].clone();
        bad_shard.data[0] ^= 0xFF;
        // Keep original hash so it mismatches
        let proof = DASampler::generate_proof(&shards, 0).unwrap();

        let responses = vec![SampleResponse {
            shard: bad_shard,
            proof,
            attestation_signature: None,
            attester_public_key: None,
        }];
        let result = DASampler::verify_samples(&da_proof, &responses, 1).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_out_of_range_proof() {
        let shards = make_test_shards();
        let result = DASampler::generate_proof(&shards, 100);
        assert!(result.is_err());
    }

    // ── Batch verification tests ──

    #[test]
    fn test_batch_verify_all_valid() {
        let shards = make_test_shards();

        // Generate proofs for every shard.
        let proofs: Vec<MerkleProof> = (0..shards.len())
            .map(|i| DASampler::generate_proof(&shards, i).unwrap())
            .collect();
        let proof_refs: Vec<&MerkleProof> = proofs.iter().collect();
        let shard_hashes: Vec<[u8; 32]> = shards.iter().map(|s| s.hash).collect();

        let result = batch_verify_proofs(&proof_refs, &shard_hashes);
        assert!(result.all_valid);
        assert_eq!(result.verified_count, shards.len());
        assert!(result.invalid_indices.is_empty());

        // Also verify via verify_samples_batch.
        let da_proof = DASampler::compute_commitment(&shards).unwrap();
        let responses: Vec<SampleResponse> = shards
            .iter()
            .enumerate()
            .map(|(i, s)| SampleResponse {
                shard: s.clone(),
                proof: proofs[i].clone(),
                attestation_signature: None,
                attester_public_key: None,
            })
            .collect();

        let batch_result = DASampler::verify_samples_batch(&da_proof, &responses, 1).unwrap();
        assert!(batch_result.all_valid);
        assert_eq!(batch_result.verified_count, shards.len());
        assert!(batch_result.invalid_indices.is_empty());
    }

    #[test]
    fn test_batch_verify_one_invalid() {
        let shards = make_test_shards();

        let proofs: Vec<MerkleProof> = (0..shards.len())
            .map(|i| DASampler::generate_proof(&shards, i).unwrap())
            .collect();

        // Tamper with one shard hash so its proof fails.
        let mut shard_hashes: Vec<[u8; 32]> = shards.iter().map(|s| s.hash).collect();
        shard_hashes[2] = [0xFFu8; 32]; // corrupt index 2

        let proof_refs: Vec<&MerkleProof> = proofs.iter().collect();
        let result = batch_verify_proofs(&proof_refs, &shard_hashes);
        assert!(!result.all_valid);
        assert!(result.invalid_indices.contains(&2));

        // Also verify via verify_samples_batch with a tampered shard.
        let da_proof = DASampler::compute_commitment(&shards).unwrap();
        let mut responses: Vec<SampleResponse> = shards
            .iter()
            .enumerate()
            .map(|(i, s)| SampleResponse {
                shard: s.clone(),
                proof: proofs[i].clone(),
                attestation_signature: None,
                attester_public_key: None,
            })
            .collect();
        // Tamper the data of response[3] so hash check fails.
        responses[3].shard.data[0] ^= 0xFF;

        let batch_result = DASampler::verify_samples_batch(&da_proof, &responses, 1).unwrap();
        assert!(!batch_result.all_valid);
        assert!(batch_result.invalid_indices.contains(&3));
    }

    #[test]
    fn test_batch_verify_mixed_roots() {
        // Build two independent shard sets with different Merkle roots.
        let shards_a = make_test_shards();
        let shards_b = {
            let encoder = ErasureEncoder::new(ErasureConfig {
                data_shards: 4,
                parity_shards: 4,
            })
            .unwrap();
            encoder
                .encode(b"Different data for second tree")
                .unwrap()
                .shards
        };

        let proof_a0 = DASampler::generate_proof(&shards_a, 0).unwrap();
        let proof_b0 = DASampler::generate_proof(&shards_b, 0).unwrap();

        // Sanity: different roots.
        assert_ne!(proof_a0.root, proof_b0.root);

        // batch_verify_proofs with proofs from two trees and correct hashes.
        let proof_refs: Vec<&MerkleProof> = vec![&proof_a0, &proof_b0];
        let shard_hashes = vec![shards_a[0].hash, shards_b[0].hash];
        let result = batch_verify_proofs(&proof_refs, &shard_hashes);
        assert!(result.all_valid);
        assert_eq!(result.verified_count, 2);

        // Now swap the hashes so each proof gets the wrong leaf — both should fail.
        let shard_hashes_swapped = vec![shards_b[0].hash, shards_a[0].hash];
        let result_bad = batch_verify_proofs(&proof_refs, &shard_hashes_swapped);
        assert!(!result_bad.all_valid);
        assert_eq!(result_bad.invalid_indices.len(), 2);

        // verify_samples_batch: mix a response whose proof.root differs from
        // the DA commitment root — should be caught as invalid.
        let da_proof_a = DASampler::compute_commitment(&shards_a).unwrap();
        let responses = vec![
            SampleResponse {
                shard: shards_a[0].clone(),
                proof: proof_a0.clone(),
                attestation_signature: None,
                attester_public_key: None,
            },
            SampleResponse {
                shard: shards_b[0].clone(),
                proof: proof_b0.clone(),
                attestation_signature: None,
                attester_public_key: None,
            },
        ];
        let batch_result = DASampler::verify_samples_batch(&da_proof_a, &responses, 1).unwrap();
        assert!(!batch_result.all_valid);
        // Index 1 should be invalid (wrong root).
        assert!(batch_result.invalid_indices.contains(&1));
        // Index 0 should be valid.
        assert!(!batch_result.invalid_indices.contains(&0));
    }

    #[test]
    fn test_per_validator_da_seeds_produce_different_queries() {
        let block_number: u64 = 42;
        let total_shards = 64;
        let num_samples = 8;

        let make_seed = |validator_id: u64| -> Vec<u8> {
            let mut seed = Vec::with_capacity(40);
            seed.extend_from_slice(b"da-sample");
            seed.extend_from_slice(&block_number.to_le_bytes());
            seed.extend_from_slice(&validator_id.to_le_bytes());
            seed
        };

        let queries_v0 =
            DASampler::generate_queries(block_number, total_shards, num_samples, &make_seed(0));
        let queries_v1 =
            DASampler::generate_queries(block_number, total_shards, num_samples, &make_seed(1));
        let queries_v2 =
            DASampler::generate_queries(block_number, total_shards, num_samples, &make_seed(2));

        let indices =
            |qs: &[SampleQuery]| -> Vec<usize> { qs.iter().map(|q| q.shard_index).collect() };

        assert_ne!(indices(&queries_v0), indices(&queries_v1));
        assert_ne!(indices(&queries_v1), indices(&queries_v2));
        assert_ne!(indices(&queries_v0), indices(&queries_v2));
    }

    #[test]
    fn test_same_validator_same_block_is_deterministic() {
        let seed = {
            let mut s = Vec::new();
            s.extend_from_slice(b"da-sample");
            s.extend_from_slice(&100u64.to_le_bytes());
            s.extend_from_slice(&5u64.to_le_bytes());
            s
        };
        let q1 = DASampler::generate_queries(100, 32, 4, &seed);
        let q2 = DASampler::generate_queries(100, 32, 4, &seed);
        let indices =
            |qs: &[SampleQuery]| -> Vec<usize> { qs.iter().map(|q| q.shard_index).collect() };
        assert_eq!(indices(&q1), indices(&q2));
    }

    /// T1.20 — `compute_commitment` rejects empty shards with the
    /// `NoShards` variant (lines 152-154). Previously the path was
    /// unreached. Also pins the same gate on the _with_config variant.
    #[test]
    fn t1_20_compute_commitment_empty_shards_returns_noshards() {
        let err = DASampler::compute_commitment(&[]).unwrap_err();
        assert!(
            matches!(err, SamplingError::NoShards),
            "empty shards must yield NoShards; got {err:?}"
        );
        let err2 = DASampler::compute_commitment_with_config(&[], 4).unwrap_err();
        assert!(matches!(err2, SamplingError::NoShards));
    }

    /// T1.20 — `generate_queries` short-circuits to empty when
    /// `total_shards == 0` (lines 232-234). Without this gate the
    /// modulo would panic; the test pins the guard.
    #[test]
    fn t1_20_generate_queries_zero_shards_returns_empty() {
        let queries = DASampler::generate_queries(42, 0, 10, b"any-seed");
        assert!(
            queries.is_empty(),
            "zero shards must short-circuit to empty"
        );
    }

    /// T1.20 — `SamplingError` variants carry the diagnostic values
    /// their format strings reference. Existing tests only check
    /// `is_err()`; operators triaging a failed verification need
    /// *which* index was out of range and *how many* samples fell
    /// short.
    #[test]
    fn t1_20_error_variants_carry_diagnostic_values() {
        // IndexOutOfRange { index, total }
        let shards = make_test_shards();
        let total = shards.len();
        let err = DASampler::generate_proof(&shards, 99).unwrap_err();
        match err {
            SamplingError::IndexOutOfRange { index, total: t } => {
                assert_eq!(index, 99);
                assert_eq!(t, total);
            }
            other => panic!("expected IndexOutOfRange, got {other:?}"),
        }

        // InsufficientSamples { got, need }
        let da_proof = DASampler::compute_commitment(&shards).unwrap();
        let err = DASampler::verify_samples(&da_proof, &[], 4).unwrap_err();
        match err {
            SamplingError::InsufficientSamples { got, need } => {
                assert_eq!(got, 0);
                assert_eq!(need, 4);
            }
            other => panic!("expected InsufficientSamples, got {other:?}"),
        }
    }

    /// T1.20 — `attestation_message` is (a) domain-separated under
    /// the `evaporchain-da-attestation-v1` tag and (b) field-sensitive
    /// across `commitment_root`, `shard_index`, `shard_hash`. Pinning
    /// these three components prevents a future refactor that dropped
    /// one of them from silently collapsing signatures across
    /// different shards or different blocks.
    #[test]
    fn t1_20_attestation_message_domain_separated_and_field_sensitive() {
        let root_a = [0x11u8; 32];
        let root_b = [0x22u8; 32];
        let hash_a = [0xAAu8; 32];
        let hash_b = [0xBBu8; 32];

        let base = DASampler::attestation_message(&root_a, 0, &hash_a);
        // Vary commitment_root.
        assert_ne!(
            base,
            DASampler::attestation_message(&root_b, 0, &hash_a),
            "commitment_root must be in the digest"
        );
        // Vary shard_index.
        assert_ne!(
            base,
            DASampler::attestation_message(&root_a, 1, &hash_a),
            "shard_index must be in the digest"
        );
        // Vary shard_hash.
        assert_ne!(
            base,
            DASampler::attestation_message(&root_a, 0, &hash_b),
            "shard_hash must be in the digest"
        );

        // Domain-separated: a raw concatenation without the v1 tag
        // would produce a different result. Pin by computing the
        // "no domain tag" baseline by hand and checking it differs.
        let no_tag = {
            let mut h = blake3::Hasher::new();
            h.update(&root_a);
            h.update(&0u64.to_le_bytes());
            h.update(&hash_a);
            let out: [u8; 32] = h.finalize().into();
            out
        };
        assert_ne!(
            base, no_tag,
            "attestation_message must include the domain tag"
        );
    }

    // ─── AUDIT_2026_05_13 H7 regression suite ─────────────────────────

    #[test]
    fn audit_h7_seed_differs_by_data_root() {
        // The pre-fix seed depended only on (block.number, validator_id)
        // — both publicly known before the producer commits. Post-fix
        // the seed is bound to `data_root`. Two different data_roots
        // for the same (block_number, validator_id) MUST produce
        // different seeds → different queries.
        let block_number = 42;
        let validator_id = 7;
        let root_a = [0xAAu8; 32];
        let root_b = [0xBBu8; 32];

        let seed_a = DASampler::build_da_sample_seed_v1(block_number, &root_a, validator_id);
        let seed_b = DASampler::build_da_sample_seed_v1(block_number, &root_b, validator_id);
        assert_ne!(seed_a, seed_b);

        let q_a = DASampler::generate_queries(block_number, 256, 4, &seed_a);
        let q_b = DASampler::generate_queries(block_number, 256, 4, &seed_b);
        // With 256 shards and 4 samples, the probability of identical
        // shard_index sets across two different seeds is < 2^-32.
        assert_ne!(
            q_a.iter().map(|q| q.shard_index).collect::<Vec<_>>(),
            q_b.iter().map(|q| q.shard_index).collect::<Vec<_>>(),
            "seeds bound to different data_roots must produce different queries"
        );
    }

    #[test]
    fn audit_h7_seed_deterministic_across_calls() {
        // Same inputs → identical seed → identical queries. Required
        // so the retry path (PendingSample.data_root) can reconstruct
        // the same shard indices.
        let seed_1 = DASampler::build_da_sample_seed_v1(100, &[0xCD; 32], 5);
        let seed_2 = DASampler::build_da_sample_seed_v1(100, &[0xCD; 32], 5);
        assert_eq!(seed_1, seed_2);
        let q_1 = DASampler::generate_queries(100, 64, 4, &seed_1);
        let q_2 = DASampler::generate_queries(100, 64, 4, &seed_2);
        assert_eq!(q_1.len(), q_2.len());
        for (a, b) in q_1.iter().zip(q_2.iter()) {
            assert_eq!(a.shard_index, b.shard_index);
        }
    }

    #[test]
    fn audit_h7_seed_carries_dst_prefix() {
        // The DST prefix `evaporchain:da-sample-seed:v1\0` is what
        // makes a future v2 (per-validator-VRF) cleanly distinguishable
        // during cutover. Lock it via byte-level assertion.
        let seed = DASampler::build_da_sample_seed_v1(0, &[0u8; 32], 0);
        assert!(seed.starts_with(DA_SAMPLE_SEED_DST));
        // Length is fixed: DST + 8 (u64 block) + 32 (data_root) + 8 (u64 vid).
        assert_eq!(seed.len(), DA_SAMPLE_SEED_DST.len() + 8 + 32 + 8);
    }

    #[test]
    fn audit_h7_seed_differs_by_validator_id() {
        // Defense-in-depth: each validator's seed is unique even
        // sharing block_number + data_root. Pre-fix this was already
        // true; locked by test to prevent regression.
        let seed_a = DASampler::build_da_sample_seed_v1(1, &[0xFF; 32], 1);
        let seed_b = DASampler::build_da_sample_seed_v1(1, &[0xFF; 32], 2);
        assert_ne!(seed_a, seed_b);
    }
}
