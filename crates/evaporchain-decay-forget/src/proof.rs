//! `DecayForgetProof` artefact + error types.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_types::Energy;

pub type RecordId = [u8; 32];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecayForgetProof {
    pub record_id: RecordId,
    /// Recoverability commitment at activation (the energy budget the
    /// chain originally committed to keeping the record recoverable).
    pub original_commitment: Energy,
    pub activated_epoch: u64,
    /// Epoch the proof claims the record is forgotten *at*.
    pub forgotten_at_epoch: u64,
    /// Threshold below which the commitment is considered forgotten.
    pub forget_threshold: Energy,
    /// The decayed commitment evaluated at `forgotten_at_epoch` —
    /// must be at or below `forget_threshold` for the proof to verify.
    pub decayed_commitment: Energy,
    /// blake3 binding over all fields.
    pub witness: [u8; 32],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ForgetProofError {
    #[error(
        "decayed commitment {decayed} is not at-or-below forget threshold {threshold} — record not yet forgotten"
    )]
    NotForgotten { decayed: Energy, threshold: Energy },
    #[error(
        "witness mismatch: re-derived {derived:?}, proof carries {claimed:?}"
    )]
    WitnessMismatch {
        derived: [u8; 32],
        claimed: [u8; 32],
    },
}
