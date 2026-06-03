//! EvaporCashNote (demurrage bearer-note, Money lane) typed init.
//!
//! Native demurrage money. ONE note = ONE contract instance; the
//! note's own `energy` builtin IS its spendable value, so a hoarded
//! note loses value by chain physics (the evaporation engine) with
//! no keeper bot, no in-contract decay formula, and no off-chain
//! timer. The Wörgl / Gesell incentive native.
//!
//! Deploy-time params:
//!   - `initial_energy`  — the note's value at issue (the deployer
//!                         funds the note with this energy budget)
//!   - `half_life`       — demurrage rate (smaller = rots faster)
//!   - `default_face`    — wallet form pre-populates this as the
//!                         default face value for the one-shot
//!                         `issue(to, face_value)` runtime call;
//!                         the actual `face` snapshot is locked at
//!                         issue time
//!
//! Runtime args (the bearer `to` address + the actual `face_value`
//! snapshot) belong to `issue()` — not deploy.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid EvaporCashNote init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub default_face: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
