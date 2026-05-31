//! Subscription typed init.
//!
//! Recurring-payment primitive whose lapse detector is the chain
//! itself. `pay()` refreshes the contract's energy via the runtime
//! hook; missing payments lets the contract evaporate; on_evaporate
//! flips lapsed=true. Same chain-as-keeper claim as DEADMAN_SWITCH
//! in a different surface.
//!
//! Params:
//!   - `initial_energy`  — contract lifetime budget
//!   - `half_life`       — energy decay rate
//!   - `period_amount`   — payment per period (carried as the default
//!                          off-chain coordinator credit value; the
//!                          on-chain set_terms() runtime call locks
//!                          the actual amount per instance)
//!   - `period_length`   — epochs per billing period
//!
//! Why the period args are in the typed init rather than purely
//! runtime: the wallet's deploy form pre-populates them from this
//! struct, and the deploy-fee oracle benefits from knowing the
//! intended cadence. The actual on-chain commitment happens in the
//! subsequent `set_terms(provider, amount, period)` call.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Subscription init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub period_amount: u64,
    pub period_length: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
