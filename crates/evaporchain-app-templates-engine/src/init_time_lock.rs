//! Time Lock (chain-as-keeper vault) typed init.
//!
//! Locks `default_amount` for a beneficiary until `default_lock_window`
//! epochs after deploy. The off-chain coordinator returns the locked
//! amount to the grantor if the contract evaporates with the lock
//! still active (never claimed, never revoked) — the runtime is the
//! deadline enforcer.
//!
//! Params:
//!   - `initial_energy`       — contract lifetime budget (sized for the full claim window)
//!   - `half_life`            — energy decay rate
//!   - `default_amount`       — wallet form pre-populates this; locked at runtime via set_terms()
//!   - `default_lock_window`  — wallet form pre-populates this as the
//!                              default epochs-until-unlock; the actual
//!                              `unlock_epoch` is computed at set_terms()
//!                              from the current chain epoch + window
//!
//! Runtime args (beneficiary address, exact amount, exact unlock_epoch)
//! belong to `set_terms(beneficiary, amount, unlock)` — not deploy.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Time Lock init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub default_amount: u64,
    pub default_lock_window: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
