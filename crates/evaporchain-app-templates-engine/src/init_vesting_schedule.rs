//! Vesting Schedule (linear vest with cliff) typed init.
//!
//! Classic financial primitive every VC-backed startup needs, with
//! the doctrine twist that the post-vest claim window is bounded by
//! the contract's own energy. on_evaporate stamps vested_at_evaporate
//! and flips forfeit_signaled so the off-chain coordinator returns
//! the unclaimed remainder to the grantor.
//!
//! Params:
//!   - `initial_energy`     — contract lifetime budget (size for grant + claim window)
//!   - `half_life`          — energy decay rate
//!   - `default_grant`      — wallet form pre-populates this; actual grant locked at set_terms()
//!   - `default_cliff`      — wallet form pre-populates this as the default cliff in epochs
//!   - `default_duration`   — wallet form pre-populates this as the default vest window
//!
//! Runtime args (beneficiary address, exact grant/cliff/duration)
//! belong to `set_terms(beneficiary, grant, cliff, duration)` — not
//! deploy.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Vesting Schedule init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub default_grant: u64,
    pub default_cliff: u64,
    pub default_duration: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
