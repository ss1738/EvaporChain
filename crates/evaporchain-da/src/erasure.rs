//! Reed-Solomon erasure coding wrapper.
//!
//! Encodes block data into data + parity shards so that the original
//! data can be reconstructed from any `data_shards` of `total_shards` pieces.

use reed_solomon_erasure::galois_8::ReedSolomon;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ErasureError {
    #[error("reed-solomon error: {0}")]
    ReedSolomon(String),
    #[error("not enough shards for reconstruction: have {have}, need {need}")]
    NotEnoughShards { have: usize, need: usize },
    #[error("shard size mismatch: expected {expected}, got {got}")]
    ShardSizeMismatch { expected: usize, got: usize },
    #[error("empty data")]
    EmptyData,
}

/// Configuration for erasure coding.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ErasureConfig {
    /// Number of original data shards.
    pub data_shards: usize,
    /// Number of parity (redundancy) shards.
    pub parity_shards: usize,
}

impl Default for ErasureConfig {
    fn default() -> Self {
        Self {
            data_shards: 4,
            parity_shards: 4,
        }
    }
}

impl ErasureConfig {
    pub fn total_shards(&self) -> usize {
        self.data_shards + self.parity_shards
    }
}

/// A single shard with its index and content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shard {
    /// Index in the shard array (0..total_shards).
    pub index: usize,
    /// Shard data bytes.
    pub data: Vec<u8>,
    /// Blake3 hash of this shard's data.
    pub hash: [u8; 32],
}

/// Result of encoding data into shards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodedData {
    /// All shards (data + parity).
    pub shards: Vec<Shard>,
    /// Erasure coding configuration used.
    pub config: ErasureConfig,
    /// Original data length before padding.
    pub original_len: usize,
    /// Size of each shard in bytes.
    pub shard_size: usize,
}

/// Reed-Solomon erasure encoder/decoder.
pub struct ErasureEncoder {
    config: ErasureConfig,
    rs: ReedSolomon,
}

impl ErasureEncoder {
    /// Create a new encoder with the given configuration.
    pub fn new(config: ErasureConfig) -> Result<Self, ErasureError> {
        let rs = ReedSolomon::new(config.data_shards, config.parity_shards)
            .map_err(|e| ErasureError::ReedSolomon(e.to_string()))?;
        Ok(Self { config, rs })
    }

    /// Create an encoder with default 4+4 configuration.
    pub fn default_config() -> Result<Self, ErasureError> {
        Self::new(ErasureConfig::default())
    }

    /// Encode data into data + parity shards.
    pub fn encode(&self, data: &[u8]) -> Result<EncodedData, ErasureError> {
        if data.is_empty() {
            return Err(ErasureError::EmptyData);
        }

        let shard_size = (data.len() + self.config.data_shards - 1) / self.config.data_shards;
        let padded_len = shard_size * self.config.data_shards;

        // Create padded data
        let mut padded = data.to_vec();
        padded.resize(padded_len, 0u8);

        // Split into data shards
        let mut shard_vecs: Vec<Vec<u8>> = padded
            .chunks(shard_size)
            .map(|c| c.to_vec())
            .collect();

        // Add empty parity shards
        for _ in 0..self.config.parity_shards {
            shard_vecs.push(vec![0u8; shard_size]);
        }

        // Encode parity
        self.rs
            .encode(&mut shard_vecs)
            .map_err(|e| ErasureError::ReedSolomon(e.to_string()))?;

        // Build Shard structs with hashes
        let shards: Vec<Shard> = shard_vecs
            .into_iter()
            .enumerate()
            .map(|(i, data)| {
                let hash = blake3::hash(&data).into();
                Shard { index: i, data, hash }
            })
            .collect();

        Ok(EncodedData {
            shards,
            config: self.config,
            original_len: data.len(),
            shard_size,
        })
    }

    /// Reconstruct original data from a subset of shards.
    ///
    /// `shards` is a Vec of `Option<Vec<u8>>` — present shards have `Some`,
    /// missing shards have `None`. At least `data_shards` must be present.
    pub fn reconstruct(&self, shards: Vec<Option<Vec<u8>>>) -> Result<Vec<u8>, ErasureError> {
        let present_count = shards.iter().filter(|s| s.is_some()).count();
        if present_count < self.config.data_shards {
            return Err(ErasureError::NotEnoughShards {
                have: present_count,
                need: self.config.data_shards,
            });
        }

        let mut shard_refs: Vec<Option<Vec<u8>>> = shards;

        self.rs
            .reconstruct(&mut shard_refs)
            .map_err(|e| ErasureError::ReedSolomon(e.to_string()))?;

        // Concatenate data shards
        let mut result = Vec::new();
        for shard in shard_refs.iter().take(self.config.data_shards) {
            if let Some(data) = shard {
                result.extend_from_slice(data);
            }
        }

        Ok(result)
    }

    /// Verify that a shard's hash matches its content.
    pub fn verify_shard(shard: &Shard) -> bool {
        let computed: [u8; 32] = blake3::hash(&shard.data).into();
        computed == shard.hash
    }

    /// Get the encoder configuration.
    pub fn config(&self) -> &ErasureConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let encoder = ErasureEncoder::default_config().unwrap();
        let data = b"Hello, EvaporChain data availability layer!";
        let encoded = encoder.encode(data).unwrap();

        assert_eq!(encoded.shards.len(), 8); // 4 data + 4 parity
        assert_eq!(encoded.original_len, data.len());

        // Reconstruct from all shards
        let all_shards: Vec<Option<Vec<u8>>> = encoded
            .shards
            .iter()
            .map(|s| Some(s.data.clone()))
            .collect();
        let recovered = encoder.reconstruct(all_shards).unwrap();
        assert_eq!(&recovered[..data.len()], data.as_slice());
    }

    #[test]
    fn test_reconstruct_with_missing_parity() {
        let encoder = ErasureEncoder::default_config().unwrap();
        let data = b"Reconstruct me from partial shards please!";
        let encoded = encoder.encode(data).unwrap();

        // Drop all 4 parity shards — still recoverable from 4 data shards
        let mut partial: Vec<Option<Vec<u8>>> = encoded
            .shards
            .iter()
            .map(|s| Some(s.data.clone()))
            .collect();
        for i in 4..8 {
            partial[i] = None;
        }

        let recovered = encoder.reconstruct(partial).unwrap();
        assert_eq!(&recovered[..data.len()], data.as_slice());
    }

    #[test]
    fn test_reconstruct_with_missing_data_shards() {
        let encoder = ErasureEncoder::default_config().unwrap();
        let data = b"Even with data shards missing, parity saves us!";
        let encoded = encoder.encode(data).unwrap();

        // Drop data shards 0 and 2, keep shards 1,3,4,5,6,7
        let mut partial: Vec<Option<Vec<u8>>> = encoded
            .shards
            .iter()
            .map(|s| Some(s.data.clone()))
            .collect();
        partial[0] = None;
        partial[2] = None;

        let recovered = encoder.reconstruct(partial).unwrap();
        assert_eq!(&recovered[..data.len()], data.as_slice());
    }

    #[test]
    fn test_too_many_missing_shards() {
        let encoder = ErasureEncoder::default_config().unwrap();
        let data = b"This won't survive 5 missing shards";
        let encoded = encoder.encode(data).unwrap();

        // Drop 5 shards — only 3 remain, need 4
        let mut partial: Vec<Option<Vec<u8>>> = encoded
            .shards
            .iter()
            .map(|s| Some(s.data.clone()))
            .collect();
        for i in 0..5 {
            partial[i] = None;
        }

        let result = encoder.reconstruct(partial);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_shard_integrity() {
        let encoder = ErasureEncoder::default_config().unwrap();
        let data = b"Verify my shard hashes";
        let encoded = encoder.encode(data).unwrap();

        for shard in &encoded.shards {
            assert!(ErasureEncoder::verify_shard(shard));
        }

        // Tamper with a shard
        let mut tampered = encoded.shards[0].clone();
        tampered.data[0] ^= 0xFF;
        assert!(!ErasureEncoder::verify_shard(&tampered));
    }

    #[test]
    fn test_empty_data_error() {
        let encoder = ErasureEncoder::default_config().unwrap();
        assert!(encoder.encode(b"").is_err());
    }

    #[test]
    fn test_custom_config() {
        let config = ErasureConfig {
            data_shards: 8,
            parity_shards: 4,
        };
        let encoder = ErasureEncoder::new(config).unwrap();
        let data = vec![42u8; 1024];
        let encoded = encoder.encode(&data).unwrap();

        assert_eq!(encoded.shards.len(), 12);
        assert_eq!(encoded.config.data_shards, 8);
        assert_eq!(encoded.config.parity_shards, 4);
    }

    #[test]
    fn test_large_data() {
        let encoder = ErasureEncoder::default_config().unwrap();
        let data = vec![0xAB; 10_000];
        let encoded = encoder.encode(&data).unwrap();

        let all_shards: Vec<Option<Vec<u8>>> = encoded
            .shards
            .iter()
            .map(|s| Some(s.data.clone()))
            .collect();
        let recovered = encoder.reconstruct(all_shards).unwrap();
        assert_eq!(&recovered[..data.len()], data.as_slice());
    }
}
