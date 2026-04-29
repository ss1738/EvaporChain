//! `Capsule` — opaque ciphertext + energy gate.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_types::Energy;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capsule {
    pub seal_epoch: u64,
    pub energy_threshold: Energy,
    pub ciphertext_blob: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CapsuleError {
    #[error("capsule ciphertext is empty")]
    EmptyCiphertext,
}

impl Capsule {
    pub fn new(
        seal_epoch: u64,
        energy_threshold: Energy,
        ciphertext_blob: Vec<u8>,
    ) -> Result<Self, CapsuleError> {
        if ciphertext_blob.is_empty() {
            return Err(CapsuleError::EmptyCiphertext);
        }
        Ok(Self {
            seal_epoch,
            energy_threshold,
            ciphertext_blob,
        })
    }
}
