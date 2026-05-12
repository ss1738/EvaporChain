//! Erasure coding and data availability sampling for EvaporChain.
//!
//! This crate provides:
//! - Reed-Solomon erasure coding for block data
//! - Data availability sampling (DAS) with proof generation/verification
//! - Block-level DA encoding and reconstruction
//! - Evaporation-specific DA proofs (unique to EvaporChain)

pub mod block_da;
pub mod block_da_2d;
pub mod certificate;
pub mod commitments;
pub mod erasure;
pub mod erasure2d;
pub mod evaporation_da;
pub mod light_client;
pub mod namespace;
pub mod poha;
pub mod pruning;
pub mod sampling;

pub use block_da::BlockDA;
pub use block_da_2d::{
    AvailabilityMetrics, BlockDA2D, BlockDA2DHeader, BlockDA2DPackage, CellSampleResult,
};
pub use erasure::ErasureEncoder;
pub use evaporation_da::EvaporationDAProof;
pub use namespace::{NamespaceId, NamespaceMerkleTree, NamespaceProof, NamespacedBlob};
pub use poha::{CertTemperature, PoHACertificate, PoHASampler, PoHAStore, TemperatureDistribution};
pub use sampling::{
    batch_verify_proofs, BatchVerifyResult, DAProof, DASampler, SampleQuery, SampleResponse,
};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use erasure::{ErasureConfig, ErasureEncoder};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "EvaporChain DA encodes block bytes into N data
    /// plus M parity Reed-Solomon shards. Any subset of N shards
    /// reconstructs the original payload exactly. Each shard carries
    /// a domain-tagged BLAKE3 hash that detects single-byte tampering.
    /// Empty input fails closed; insufficient-shard reconstruction
    /// fails closed."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let cfg = ErasureConfig {
            data_shards: 4,
            parity_shards: 4,
        };
        let enc = ErasureEncoder::new(cfg).unwrap();
        let payload: Vec<u8> = (0..1024u32).map(|i| i as u8).collect();

        // Encode → 8 shards (4 data + 4 parity).
        let encoded = enc.encode(&payload).unwrap();
        assert_eq!(encoded.shards.len(), 8);

        // Each shard's hash matches its content.
        for s in &encoded.shards {
            assert!(ErasureEncoder::verify_shard(s));
        }

        // Drop 4 shards (the parity ones); the remaining 4 data
        // shards reconstruct the original payload byte-for-byte.
        let mut subset: Vec<Option<Vec<u8>>> = (0..8).map(|_| None).collect();
        for (i, slot) in subset.iter_mut().take(4).enumerate() {
            *slot = Some(encoded.shards[i].data.clone());
        }
        let reconstructed = enc.reconstruct(subset).unwrap();
        assert_eq!(&reconstructed[..payload.len()], &payload[..]);

        // Tampered shard: flip one byte → hash check fails.
        let mut tampered = encoded.shards[0].clone();
        tampered.data[0] ^= 0xFF;
        assert!(!ErasureEncoder::verify_shard(&tampered));

        // Insufficient shards (only 3 of 4 needed) → fail closed.
        let mut too_few: Vec<Option<Vec<u8>>> = (0..8).map(|_| None).collect();
        for (i, slot) in too_few.iter_mut().take(3).enumerate() {
            *slot = Some(encoded.shards[i].data.clone());
        }
        assert!(enc.reconstruct(too_few).is_err());

        // Empty payload → typed error, not silent zero-shard pass.
        assert!(enc.encode(&[]).is_err());
    }
}
