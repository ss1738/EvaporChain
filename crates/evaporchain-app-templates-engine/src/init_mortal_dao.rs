//! Mortal-DAO typed init.
//!
//! Single-instance governance contract whose lifecycle rides the
//! contract's own energy. Composes all four decay primitives:
//!   - members refresh to stay active (decay-credential)
//!   - per-member proposal cap resets on refresh (decay-rate-limit)
//!   - vote weight = participations + 1 (decay-reputation)
//!   - quorum threshold tracks running peak engagement (decay-quorum)
//!
//! Params:
//!   - `energy`            — contract lifetime budget
//!   - `half_life`         — decay rate
//!   - `freshness_window`  — epochs a member may go silent before
//!                           losing active-member status
//!   - `proposal_cap`      — max proposals per active member per
//!                           refresh window
//!   - `voting_window`     — epochs a proposal stays open for votes
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Mortal-DAO init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub energy: u64,
    pub half_life: u64,
    pub freshness_window: u64,
    pub proposal_cap: u64,
    pub voting_window: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
