//! V2 certificate type.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaporatedForkCertV2 {
    pub fork_root: [u8; 32],
    pub evaluated_at_epoch: u64,
    pub total_seed_energy: u128,
    pub decayed_energy: u128,
    pub threshold: u128,
    /// 32-byte anchor — typically `BellCertificate.seed`.
    pub bell_seed_anchor: [u8; 32],
    /// Epoch at which the seed anchor was issued. Must satisfy
    /// `seed_anchor_epoch ≤ evaluated_at_epoch`.
    pub seed_anchor_epoch: u64,
    pub witness: [u8; 32],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CertV2Error {
    #[error("fork's decayed energy ({decayed}) is not below threshold ({threshold}) — fork has not evaporated")]
    NotEvaporated { decayed: u128, threshold: u128 },
    #[error("witness mismatch: re-derived {derived:?}, certificate carries {claimed:?}")]
    WitnessMismatch {
        derived: [u8; 32],
        claimed: [u8; 32],
    },
    #[error("causality violation: seed_anchor_epoch ({anchor}) > evaluated_at_epoch ({eval})")]
    CausalityViolation { anchor: u64, eval: u64 },
}
