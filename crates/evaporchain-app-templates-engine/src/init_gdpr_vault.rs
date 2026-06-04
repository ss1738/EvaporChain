//! GdprVault (Erasure-as-a-Service / crypto-shred, Privacy lane)
//! typed init.
//!
//! ONE retained record = ONE contract instance. The chain holds NO
//! personal data — only a 32-byte ciphertext commitment + the
//! consent/retention lifecycle (Dead Drop §9 founding constraint).
//! The contract's own energy IS the retention clock; on_evaporate
//! emits the natural-deadline shred trigger that off-chain key-
//! custody/HSM subscribes to.
//!
//! Deploy-time params:
//!   - `initial_energy`         — retention budget (sized to the
//!                                retention period)
//!   - `half_life`              — decay rate (controller picks; sets
//!                                the effective deadline)
//!   - `default_lawful_basis`   — wallet form pre-populates this as
//!                                the default Art. 6 lawful-basis
//!                                code for the one-shot
//!                                `seal(ct_commit, subject, basis)`
//!                                runtime call (1=consent,
//!                                2=contract, 3=legal-obligation,
//!                                6=legitimate-interest)
//!
//! Runtime args (ct_commitment + subject + actual basis) belong to
//! `seal()` — not deploy.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid GdprVault init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub default_lawful_basis: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
