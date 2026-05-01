//! `RetentionProof` — proof artefact + error types.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_types::Energy;

pub type StateId = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionProof {
    pub state_id: StateId,
    /// Epoch at which the regulator (or any consumer) committed
    /// `committed_energy` to keep this state alive.
    pub activated_epoch: u64,
    /// Energy committed at activation. Decays under chain-global λ.
    pub committed_energy: Energy,
    /// The latest epoch at which the committed energy is still above
    /// the chain-set retention floor.
    pub retained_until_epoch: u64,
    /// Domain-separated blake3 over the binding fields.
    pub witness: [u8; 32],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RetentionProofError {
    #[error(
        "query epoch {query} exceeds retained_until {retained_until} — state \
         no longer provably retained"
    )]
    QueryAfterRetention { query: u64, retained_until: u64 },
    #[error("witness mismatch: re-derived {derived:?}, proof carries {claimed:?}")]
    WitnessMismatch {
        derived: [u8; 32],
        claimed: [u8; 32],
    },
}
