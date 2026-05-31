//! Decay Access Pass typed init.
//!
//! On-chain decaying credential. The pass's "strength" is the
//! contract's own energy; valid only while strength stays at or
//! above `validity_floor`. Issuer-gated issue/revoke; structural
//! revocation arrives when the contract evaporates (no exercise()
//! call can succeed past that point).
//!
//! Params:
//!   - `energy`         — initial credential strength
//!   - `half_life`      — decay rate (epochs to halve)
//!   - `validity_floor` — minimum strength for the pass to remain
//!                        valid (below this, gates fail-closed)
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Decay Access Pass init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub energy: u64,
    pub half_life: u64,
    pub validity_floor: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
