//! Data availability sampling (DAS).
//!
//! Light clients can verify data availability by requesting random samples
//! from block producers. Each sample includes a Merkle proof against the
//! shard commitment root.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::erasure::Shard;

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
pub fn batch_verify_proofs(proofs: &[&MerkleProof], shard_hashes: &[[u8; 32]]) -> BatchVerifyResult {
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

    for (_root, indices) in &groups {
        for &idx in indices {
            verified_count += 1;

            // Reconstruct root from leaf + siblings.
            let mut current = shard_hashes[idx];
            let mut leaf_idx = proofs[idx].leaf_index;
            for sibling in &proofs[idx].siblings {
                current = if leaf_idx % 2 == 0 {
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
            current = if index % 2 == 0 {
                Self::hash_pair(&current, sibling)
            } else {
                Self::hash_pair(sibling, &current)
            };
            index /= 2;
        }

        current == proof.root
    }

    /// Generate random sample queries for a block.
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
            let sibling_index = if current_index % 2 == 0 {
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
        let da_proof =
            DASampler::compute_commitment_with_config(&shards, 4).unwrap();

        // Generate sample queries
        let queries = DASampler::generate_queries(1, 8, 4, b"light-client-seed");

        // Respond to queries
        let responses: Vec<SampleResponse> = queries
            .iter()
            .map(|q| {
                let shard = shards[q.shard_index].clone();
                let proof = DASampler::generate_proof(&shards, q.shard_index).unwrap();
                SampleResponse { shard, proof }
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
            })
            .collect();

        let batch_result =
            DASampler::verify_samples_batch(&da_proof, &responses, 1).unwrap();
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
            })
            .collect();
        // Tamper the data of response[3] so hash check fails.
        responses[3].shard.data[0] ^= 0xFF;

        let batch_result =
            DASampler::verify_samples_batch(&da_proof, &responses, 1).unwrap();
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
            encoder.encode(b"Different data for second tree").unwrap().shards
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
            },
            SampleResponse {
                shard: shards_b[0].clone(),
                proof: proof_b0.clone(), // root != da_proof_a.commitment_root
            },
        ];
        let batch_result =
            DASampler::verify_samples_batch(&da_proof_a, &responses, 1).unwrap();
        assert!(!batch_result.all_valid);
        // Index 1 should be invalid (wrong root).
        assert!(batch_result.invalid_indices.contains(&1));
        // Index 0 should be valid.
        assert!(!batch_result.invalid_indices.contains(&0));
    }
}
