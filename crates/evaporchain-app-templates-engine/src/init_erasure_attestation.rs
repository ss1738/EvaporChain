//! ErasureAttestation (NIST 800-88 Certificate of Disposition,
//! Privacy lane) typed init.
//!
//! Proof-of-Erasure-as-a-Service. ONE attestation = ONE contract
//! instance. The chain holds NO personal data — only the disposition
//! metadata (data ref + sanitization method + verification result +
//! who/when), per NIST SP 800-88 Certificate of Media Disposition.
//! The contract's own energy IS the obligation/retention window;
//! on_evaporate without a recorded attestation emits the regulator-
//! grade NEGATIVE proof that the deadline lapsed un-attested.
//!
//! Pair with `gdpr_vault.es`: GdprVault destroys the key (shred
//! trigger); ErasureAttestation immutably proves the destruction was
//! performed and verified.
//!
//! Deploy-time params:
//!   - `initial_energy`             — obligation window budget
//!   - `half_life`                   — decay rate (controller picks)
//!   - `default_obligation_basis`   — wallet form default (1=GDPR-Art17,
//!                                     2=CCPA/AB1008, 3=NIST-program)
//!   - `default_method`             — wallet form default sanitization
//!                                     method (1=crypto-shred, 2=clear,
//!                                     3=purge, 4=destroy, 5=ML-unlearn)
//!
//! Runtime args (data_commitment + subject + actual basis + actual
//! method) belong to the one-shot `seal()` mutator.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid ErasureAttestation init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub default_obligation_basis: u64,
    pub default_method: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
