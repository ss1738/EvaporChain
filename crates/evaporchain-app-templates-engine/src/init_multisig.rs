//! Multisig (one-decision-per-contract) typed init.
//!
//! Doctrine inversion of Gnosis-Safe-style proposal-map architectures:
//! the contract IS the proposal. The signer set, threshold, and
//! proposal action are all locked to a single instance; multiple
//! decisions = multiple contracts deployed independently and
//! evaporating independently.
//!
//! Params:
//!   - `initial_energy`     — contract lifetime budget (the decision
//!                            window)
//!   - `half_life`          — energy decay rate
//!   - `default_threshold`  — the wallet form pre-populates this as the
//!                            default required signature count; the
//!                            actual on-chain threshold is locked at
//!                            runtime via `set_threshold(t)` before
//!                            `propose()` seals the configuration
//!
//! Runtime args (signer addresses, threshold value, proposal action
//! string) are set after deploy via add_signer/set_threshold/propose.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Multisig init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub default_threshold: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
