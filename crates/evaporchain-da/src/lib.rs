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
