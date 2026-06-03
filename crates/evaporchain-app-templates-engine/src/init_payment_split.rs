//! Payment Split (pull-payment revenue splitter) typed init.
//!
//! Pull-payment revenue splitter with basis-point shares (must sum to
//! exactly 10_000 — 100.00% — by seal-time). Any address deposits;
//! recipients pull on demand. on_evaporate stamps the unclaimed pool
//! and signals forfeit so the off-chain coordinator returns the
//! residue to the deployer.
//!
//! Params:
//!   - `initial_energy`  — contract lifetime budget (sized for the
//!                          full deposit + claim window)
//!   - `half_life`       — energy decay rate
//!
//! Recipients + their bps shares are set at runtime via
//! `add_recipient(target, bps)` + `seal()`; not part of the typed
//! init because the recipient set is variable-shape (1..=N) and would
//! need a JSON array. Keeping init lean.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Payment Split init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
