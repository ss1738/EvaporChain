//! Erasure coding and data availability sampling for EvaporChain.
//!
//! This crate provides:
//! - Reed-Solomon erasure coding for block data
//! - Data availability sampling (DAS) with proof generation/verification
//! - Block-level DA encoding and reconstruction
//! - Evaporation-specific DA proofs (unique to EvaporChain)

pub mod erasure;
pub mod erasure2d;
pub mod sampling;
pub mod block_da;
pub mod evaporation_da;
pub mod commitments;
pub mod certificate;
pub mod namespace;
pub mod poha;

pub use erasure::ErasureEncoder;
pub use sampling::{DASampler, DAProof, SampleQuery, SampleResponse};
pub use block_da::BlockDA;
pub use evaporation_da::EvaporationDAProof;
pub use namespace::{NamespaceMerkleTree, NamespacedBlob, NamespaceId, NamespaceProof};
pub use poha::{PoHACertificate, PoHAStore, PoHASampler, CertTemperature, TemperatureDistribution};
