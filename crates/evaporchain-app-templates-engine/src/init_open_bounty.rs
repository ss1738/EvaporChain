//! Open-Call Bounty typed init.
//!
//! Task bounty where the un-accepted-on-evaporation refund IS the
//! chain runtime; no off-chain liquidator required. Same chain-as-
//! keeper escrow doctrine as DEADMAN_SWITCH and SUBSCRIPTION_SERVICE,
//! in a task-bounty surface.
//!
//! Params:
//!   - `initial_energy`   — contract lifetime budget
//!   - `half_life`        — decay rate (epochs to halve energy)
//!   - `default_reward`   — default reward amount the wallet form
//!                          pre-populates; the actual reward is
//!                          locked at runtime via `set_bounty(task,
//!                          reward)` after the contract exists
//!
//! Runtime args (task spec string, recipient address for accept,
//! solution string for submit) are not part of the typed init.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Open Bounty init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub default_reward: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
