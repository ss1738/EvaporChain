//! Sealed-Bid Auction (commit/reveal/settle) typed init.
//!
//! Classic commit/reveal/settle auction with a doctrine twist:
//! `effective` (decay-adjusted) bid strength is the comparator, not
//! nominal. The auction lives in the chain runtime's auction-clerk
//! role — phases advance under seller direction; on_evaporate
//! without settlement = void.
//!
//! Params:
//!   - `initial_energy`         — contract lifetime budget (sized for
//!                                full COMMIT + REVEAL + SETTLE window)
//!   - `half_life`              — energy decay rate
//!   - `default_reserve_price`  — wallet form pre-populates this as
//!                                the default reserve; actual reserve
//!                                is locked at runtime via set_metadata()
//!
//! Runtime args (item label, exact reserve_price) belong to
//! `set_metadata()` — not deploy.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Sealed-Bid Auction init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub default_reserve_price: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
