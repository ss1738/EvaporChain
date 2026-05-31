//! DeadMan Switch typed init.
//!
//! Two params: `initial_energy` (the contract's lifetime budget in
//! the chain's decay schedule) and `refresh_window` (how many epochs
//! the holder may go silent before `release_dead` becomes callable
//! by anyone). Both must be positive; bind-layer validation enforces
//! this in `evaporchain-app-templates-bind`.
//!
//! The runtime `arm(holder, payload_hash, window)` args are NOT part
//! of this typed init — they're set by the deployer in a subsequent
//! call after the contract instance exists. The catalogue's
//! `default_params` exposes `refresh_window` here so the wallet UI
//! can pre-populate that downstream arg from a single deploy form.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid DeadMan Switch init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub refresh_window: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
