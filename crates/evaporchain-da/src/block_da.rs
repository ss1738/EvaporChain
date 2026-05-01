//! Block-level data availability encoding and reconstruction.
//!
//! Ties together erasure coding + DAS for complete blocks.

use serde::{Deserialize, Serialize};

use crate::erasure::{ErasureConfig, ErasureEncoder, ErasureError, Shard};
use crate::sampling::{DASampler, SampleResponse, SamplingError};

/// Errors from block DA operations.
#[derive(Debug, thiserror::Error)]
pub enum BlockDAError {
    #[error("erasure error: {0}")]
    Erasure(#[from] ErasureError),
    #[error("sampling error: {0}")]
    Sampling(#[from] SamplingError),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("block reconstruction failed: data hash mismatch")]
    ReconstructionHashMismatch,
}

/// DA header extension for a block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDAHeader {
    /// DA commitment root (Merkle root over shard hashes).
    pub commitment_root: [u8; 32],
    /// Blake3 hash of the original serialized block data.
    pub data_hash: [u8; 32],
    /// Erasure coding configuration.
    pub erasure_config: ErasureConfig,
    /// Total number of shards.
    pub total_shards: usize,
    /// Original data length in bytes.
    pub original_len: usize,
}

/// Complete block DA package: header + encoded shards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDAPackage {
    pub header: BlockDAHeader,
    pub shards: Vec<Shard>,
}

/// Block-level DA operations.
pub struct BlockDA {
    encoder: ErasureEncoder,
}

impl BlockDA {
    /// Create with default 4+4 erasure config.
    pub fn new() -> Result<Self, BlockDAError> {
        Ok(Self {
            encoder: ErasureEncoder::default_config()?,
        })
    }

    /// Create with custom erasure config.
    pub fn with_config(config: ErasureConfig) -> Result<Self, BlockDAError> {
        Ok(Self {
            encoder: ErasureEncoder::new(config)?,
        })
    }

    /// Encode a block's serialized data for DA.
    pub fn encode_block(&self, block_data: &[u8]) -> Result<BlockDAPackage, BlockDAError> {
        let data_hash: [u8; 32] = blake3::hash(block_data).into();
        let encoded = self.encoder.encode(block_data)?;
        let da_proof = DASampler::compute_commitment_with_config(
            &encoded.shards,
            self.encoder.config().data_shards,
        )?;

        Ok(BlockDAPackage {
            header: BlockDAHeader {
                commitment_root: da_proof.commitment_root,
                data_hash,
                erasure_config: *self.encoder.config(),
                total_shards: encoded.shards.len(),
                original_len: encoded.original_len,
            },
            shards: encoded.shards,
        })
    }

    /// Reconstruct block data from available shards.
    pub fn reconstruct_block(
        &self,
        header: &BlockDAHeader,
        shards: Vec<Option<Vec<u8>>>,
    ) -> Result<Vec<u8>, BlockDAError> {
        let recovered = self.encoder.reconstruct(shards)?;
        if header.original_len > recovered.len() {
            return Err(BlockDAError::ReconstructionHashMismatch);
        }
        let truncated = &recovered[..header.original_len];

        // Verify hash
        let hash: [u8; 32] = blake3::hash(truncated).into();
        if hash != header.data_hash {
            return Err(BlockDAError::ReconstructionHashMismatch);
        }

        Ok(truncated.to_vec())
    }

    /// Generate a DA proof for a specific shard in a block.
    pub fn prove_shard(
        &self,
        package: &BlockDAPackage,
        shard_index: usize,
    ) -> Result<SampleResponse, BlockDAError> {
        let proof = DASampler::generate_proof(&package.shards, shard_index)?;
        Ok(SampleResponse {
            shard: package.shards[shard_index].clone(),
            proof,
            attestation_signature: None,
            attester_public_key: None,
        })
    }

    /// Verify a shard sample against the block DA header.
    pub fn verify_shard_sample(header: &BlockDAHeader, response: &SampleResponse) -> bool {
        // Verify shard hash
        let computed_hash: [u8; 32] = blake3::hash(&response.shard.data).into();
        if computed_hash != response.shard.hash {
            return false;
        }

        // Verify proof root matches header commitment
        if response.proof.root != header.commitment_root {
            return false;
        }

        DASampler::verify_proof(&response.shard, &response.proof)
    }

    /// Generate random sample queries for a block.
    pub fn generate_sample_queries(
        block_number: u64,
        header: &BlockDAHeader,
        num_samples: usize,
        seed: &[u8],
    ) -> Vec<crate::sampling::SampleQuery> {
        DASampler::generate_queries(block_number, header.total_shards, num_samples, seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_reconstruct_block() {
        let da = BlockDA::new().unwrap();
        let block_data = br#"{"number":42,"epoch":100,"txs":["transfer","refresh"]}"#;

        let package = da.encode_block(block_data).unwrap();
        assert_eq!(package.header.original_len, block_data.len());
        assert_eq!(package.shards.len(), 8);

        // Reconstruct from all shards
        let all: Vec<Option<Vec<u8>>> = package
            .shards
            .iter()
            .map(|s| Some(s.data.clone()))
            .collect();
        let recovered = da.reconstruct_block(&package.header, all).unwrap();
        assert_eq!(recovered, block_data);
    }

    #[test]
    fn test_encode_reconstruct_with_missing() {
        let da = BlockDA::new().unwrap();
        let block_data = b"Block data with some shards missing during reconstruction";

        let package = da.encode_block(block_data).unwrap();

        // Drop 3 shards (max tolerable for 4+4)
        let mut partial: Vec<Option<Vec<u8>>> = package
            .shards
            .iter()
            .map(|s| Some(s.data.clone()))
            .collect();
        partial[1] = None;
        partial[3] = None;
        partial[5] = None;

        let recovered = da.reconstruct_block(&package.header, partial).unwrap();
        assert_eq!(recovered, block_data);
    }

    #[test]
    fn test_prove_and_verify_shard() {
        let da = BlockDA::new().unwrap();
        let block_data = b"Prove individual shard availability";
        let package = da.encode_block(block_data).unwrap();

        for i in 0..package.shards.len() {
            let response = da.prove_shard(&package, i).unwrap();
            assert!(BlockDA::verify_shard_sample(&package.header, &response));
        }
    }

    #[test]
    fn test_tampered_shard_fails_block_verify() {
        let da = BlockDA::new().unwrap();
        let block_data = b"Tamper detection test";
        let package = da.encode_block(block_data).unwrap();

        let mut response = da.prove_shard(&package, 0).unwrap();
        response.shard.data[0] ^= 0xFF; // tamper
        assert!(!BlockDA::verify_shard_sample(&package.header, &response));
    }

    #[test]
    fn test_hash_mismatch_on_corrupt_reconstruct() {
        let da = BlockDA::new().unwrap();
        let block_data = b"Hash mismatch if data corrupted";
        let package = da.encode_block(block_data).unwrap();

        // Provide all shards but tamper with one
        let mut shards: Vec<Option<Vec<u8>>> = package
            .shards
            .iter()
            .map(|s| Some(s.data.clone()))
            .collect();
        if let Some(ref mut s) = shards[0] {
            s[0] ^= 0xFF;
        }

        // Drop enough so reconstruction uses the tampered shard
        // With all shards present, RS may just use data shards directly
        // so tamper data shard 0 and drop parity to force using it
        let mut shards2: Vec<Option<Vec<u8>>> = package
            .shards
            .iter()
            .map(|s| Some(s.data.clone()))
            .collect();
        // Remove all parity
        for slot in shards2.iter_mut().take(8).skip(4) {
            *slot = None;
        }
        // Tamper data shard 0
        if let Some(ref mut s) = shards2[0] {
            s[0] ^= 0xFF;
        }

        let result = da.reconstruct_block(&package.header, shards2);
        assert!(result.is_err());
    }

    #[test]
    fn test_sample_queries_generation() {
        let da = BlockDA::new().unwrap();
        let block_data = b"Query generation test";
        let package = da.encode_block(block_data).unwrap();

        let queries = BlockDA::generate_sample_queries(1, &package.header, 10, b"seed");
        assert_eq!(queries.len(), 10);
        for q in &queries {
            assert!(q.shard_index < package.header.total_shards);
        }
    }
}
