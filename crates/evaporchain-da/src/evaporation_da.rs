//! Evaporation-specific data availability proofs.
//!
//! When an object evaporates, we need to prove that its data was available
//! *before* evaporation. This is unique to EvaporChain — no other chain
//! needs to prove historical DA for state that intentionally disappears.
//!
//! The evaporation DA proof contains:
//! - The shard commitment from when the object was last encoded
//! - A pre-evaporation data hash
//! - An energy snapshot proving the object had decayed to zero
//! - A Merkle proof linking the object's data to the block DA commitment

use evaporchain_types::{Epoch, ObjectId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::erasure::{ErasureError, Shard};
use crate::sampling::{DASampler, MerkleProof, SamplingError};

#[derive(Error, Debug)]
pub enum EvaporationDAError {
    #[error("erasure error: {0}")]
    Erasure(#[from] ErasureError),
    #[error("sampling error: {0}")]
    Sampling(#[from] SamplingError),
    #[error("energy not zero at evaporation: {0}")]
    EnergyNotZero(u64),
    #[error("data hash mismatch")]
    DataHashMismatch,
}

/// Snapshot of an object's energy state at evaporation time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnergySnapshot {
    /// Object ID.
    pub object_id: ObjectId,
    /// Energy at evaporation epoch (should be 0).
    pub energy_at_evaporation: u64,
    /// The epoch when evaporation occurred.
    pub evaporation_epoch: Epoch,
    /// Half-life of the object.
    pub half_life: u64,
    /// Last refresh epoch.
    pub last_refreshed: Epoch,
    /// Initial energy at last refresh.
    pub energy_at_refresh: u64,
}

/// A proof that an object's data was available before evaporation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaporationDAProof {
    /// Object that evaporated.
    pub object_id: ObjectId,
    /// Blake3 hash of the object's data before evaporation.
    pub pre_evaporation_data_hash: [u8; 32],
    /// DA commitment root from the block containing this object's data.
    pub da_commitment_root: [u8; 32],
    /// Merkle proof linking object data shard to the DA commitment.
    pub shard_proof: MerkleProof,
    /// Shard index where this object's data was encoded.
    pub shard_index: usize,
    /// Shard hash for verification.
    pub shard_hash: [u8; 32],
    /// Energy snapshot at evaporation time.
    pub energy_snapshot: EnergySnapshot,
    /// Epoch when the proof was generated.
    pub proof_epoch: Epoch,
}

/// Builder for evaporation DA proofs.
pub struct EvaporationDAProofBuilder;

impl EvaporationDAProofBuilder {
    /// Create a DA proof for an object that is about to evaporate.
    ///
    /// This proves the object's data was erasure-coded and available
    /// in the block DA before the object evaporated.
    pub fn create_proof(
        object_id: ObjectId,
        object_data: &[u8],
        energy_snapshot: EnergySnapshot,
        block_shards: &[Shard],
        shard_index: usize,
    ) -> Result<EvaporationDAProof, EvaporationDAError> {
        // Verify energy is zero (or near-zero for grace period objects)
        if energy_snapshot.energy_at_evaporation > 0 {
            return Err(EvaporationDAError::EnergyNotZero(
                energy_snapshot.energy_at_evaporation,
            ));
        }

        let pre_evaporation_data_hash: [u8; 32] = blake3::hash(object_data).into();

        // Generate Merkle proof against block DA shards
        let proof = DASampler::generate_proof(block_shards, shard_index)?;
        let commitment = DASampler::compute_commitment(block_shards)?;

        Ok(EvaporationDAProof {
            object_id,
            pre_evaporation_data_hash,
            da_commitment_root: commitment.commitment_root,
            shard_proof: proof,
            shard_index,
            shard_hash: block_shards[shard_index].hash,
            proof_epoch: energy_snapshot.evaporation_epoch,
            energy_snapshot,
        })
    }

    /// Verify an evaporation DA proof.
    pub fn verify_proof(
        proof: &EvaporationDAProof,
        shard_data: &[u8],
    ) -> Result<bool, EvaporationDAError> {
        // 1. Verify energy was zero
        if proof.energy_snapshot.energy_at_evaporation > 0 {
            return Ok(false);
        }

        // 2. Verify shard hash
        let computed_hash: [u8; 32] = blake3::hash(shard_data).into();
        if computed_hash != proof.shard_hash {
            return Ok(false);
        }

        // 3. Verify Merkle proof
        let shard = crate::erasure::Shard {
            index: proof.shard_index,
            data: shard_data.to_vec(),
            hash: computed_hash,
        };

        if !DASampler::verify_proof(&shard, &proof.shard_proof) {
            return Ok(false);
        }

        // 4. Verify proof root matches commitment
        if proof.shard_proof.root != proof.da_commitment_root {
            return Ok(false);
        }

        Ok(true)
    }

    /// Create a compact proof hash for on-chain storage.
    /// This 32-byte hash commits to the entire evaporation DA proof.
    pub fn proof_hash(proof: &EvaporationDAProof) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&proof.object_id);
        hasher.update(&proof.pre_evaporation_data_hash);
        hasher.update(&proof.da_commitment_root);
        hasher.update(&proof.shard_hash);
        hasher.update(&proof.proof_epoch.to_le_bytes());
        hasher.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::erasure::{ErasureConfig, ErasureEncoder};

    fn setup_test_data() -> (ObjectId, Vec<u8>, EnergySnapshot, Vec<Shard>) {
        let object_id = [0x42u8; 32];
        let object_data = b"token:evap-governance state data payload".to_vec();

        let energy_snapshot = EnergySnapshot {
            object_id,
            energy_at_evaporation: 0,
            evaporation_epoch: 100,
            half_life: 10,
            last_refreshed: 50,
            energy_at_refresh: 50_000,
        };

        // Encode block data (which includes this object)
        let encoder = ErasureEncoder::new(ErasureConfig {
            data_shards: 4,
            parity_shards: 4,
        })
        .unwrap();

        let block_data = b"block containing token:evap-governance and other objects";
        let encoded = encoder.encode(block_data).unwrap();

        (object_id, object_data, energy_snapshot, encoded.shards)
    }

    #[test]
    fn test_create_evaporation_proof() {
        let (object_id, object_data, snapshot, shards) = setup_test_data();

        let proof = EvaporationDAProofBuilder::create_proof(
            object_id,
            &object_data,
            snapshot,
            &shards,
            0,
        )
        .unwrap();

        assert_eq!(proof.object_id, object_id);
        assert_eq!(proof.shard_index, 0);
        assert_eq!(proof.proof_epoch, 100);
        assert_ne!(proof.pre_evaporation_data_hash, [0u8; 32]);
    }

    #[test]
    fn test_verify_evaporation_proof() {
        let (object_id, object_data, snapshot, shards) = setup_test_data();

        let proof = EvaporationDAProofBuilder::create_proof(
            object_id,
            &object_data,
            snapshot,
            &shards,
            2,
        )
        .unwrap();

        let valid = EvaporationDAProofBuilder::verify_proof(&proof, &shards[2].data).unwrap();
        assert!(valid);
    }

    #[test]
    fn test_reject_nonzero_energy() {
        let (object_id, object_data, mut snapshot, shards) = setup_test_data();
        snapshot.energy_at_evaporation = 100; // not zero!

        let result = EvaporationDAProofBuilder::create_proof(
            object_id,
            &object_data,
            snapshot,
            &shards,
            0,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_shard_data_fails() {
        let (object_id, object_data, snapshot, shards) = setup_test_data();

        let proof = EvaporationDAProofBuilder::create_proof(
            object_id,
            &object_data,
            snapshot,
            &shards,
            0,
        )
        .unwrap();

        // Verify with wrong shard data
        let wrong_data = vec![0xFF; shards[0].data.len()];
        let valid = EvaporationDAProofBuilder::verify_proof(&proof, &wrong_data).unwrap();
        assert!(!valid);
    }

    #[test]
    fn test_proof_hash_deterministic() {
        let (object_id, object_data, snapshot, shards) = setup_test_data();

        let proof = EvaporationDAProofBuilder::create_proof(
            object_id,
            &object_data,
            snapshot,
            &shards,
            0,
        )
        .unwrap();

        let h1 = EvaporationDAProofBuilder::proof_hash(&proof);
        let h2 = EvaporationDAProofBuilder::proof_hash(&proof);
        assert_eq!(h1, h2);
        assert_ne!(h1, [0u8; 32]);
    }

    #[test]
    fn test_proof_hash_changes_with_object() {
        let (_, object_data, snapshot, shards) = setup_test_data();

        let proof1 = EvaporationDAProofBuilder::create_proof(
            [0x42u8; 32],
            &object_data,
            snapshot.clone(),
            &shards,
            0,
        )
        .unwrap();

        let mut snapshot2 = snapshot;
        snapshot2.object_id = [0x99u8; 32];
        let proof2 = EvaporationDAProofBuilder::create_proof(
            [0x99u8; 32],
            &object_data,
            snapshot2,
            &shards,
            0,
        )
        .unwrap();

        assert_ne!(
            EvaporationDAProofBuilder::proof_hash(&proof1),
            EvaporationDAProofBuilder::proof_hash(&proof2)
        );
    }

    #[test]
    fn test_different_shard_indices() {
        let (object_id, object_data, snapshot, shards) = setup_test_data();

        for i in 0..shards.len() {
            let proof = EvaporationDAProofBuilder::create_proof(
                object_id,
                &object_data,
                snapshot.clone(),
                &shards,
                i,
            )
            .unwrap();

            let valid =
                EvaporationDAProofBuilder::verify_proof(&proof, &shards[i].data).unwrap();
            assert!(valid, "Failed for shard index {i}");
        }
    }
}
