//! `EvaporatedForkCert` + `ForkBlock` (the per-block input the prover
//! aggregates).

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkBlock {
    pub seed_energy: Energy,
    pub observed_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaporatedForkCert {
    pub fork_root: [u8; 32],
    pub evaluated_at_epoch: u64,
    pub total_seed_energy: u128,
    pub decayed_energy: u128,
    pub threshold: u128,
    pub witness: [u8; 32],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CertError {
    #[error(
        "fork's decayed energy ({decayed}) is not below threshold ({threshold}) — fork has not evaporated"
    )]
    NotEvaporated { decayed: u128, threshold: u128 },
    #[error(
        "witness mismatch: re-derived {derived:?}, certificate carries {claimed:?}"
    )]
    WitnessMismatch {
        derived: [u8; 32],
        claimed: [u8; 32],
    },
}
