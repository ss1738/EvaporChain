//! Bell-Oracle typed init.
//!
//! On-chain consumer of EvaporChain's per-block CHSH S-value beacon.
//! Structurally rejects readings at or below the local-realism floor
//! (default 2000 milli-units = S = 2.0). Downstream contracts gate
//! quantum-randomness-requiring actions on `is_certified_now()`.
//!
//! Params:
//!   - `energy`          — contract lifetime budget
//!   - `half_life`       — decay rate
//!   - `threshold_milli` — minimum acceptable S-value (in milli-units;
//!                         2000 = S=2.0, the Bell inequality floor)
//!   - `max_age_epochs`  — how stale a certified reading may be before
//!                         `is_certified_now` returns false
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Bell-Oracle init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub energy: u64,
    pub half_life: u64,
    pub threshold_milli: u64,
    pub max_age_epochs: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
