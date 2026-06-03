//! Lottery (single-draw, chain-VRF) typed init.
//!
//! The operator deploys, then locks `prize` + `stake` once via the
//! contract's one-shot `set_event(prize, stake)` mutator. Enrolment
//! opens; addresses enter exactly once; operator triggers `draw()`;
//! `random_range(entry_count)` picks the winner from the chain's VRF
//! beacon (LOTTERY-1, audit 2026-05-17 — operator can influence WHEN,
//! never WHO). Winner pulls the prize once. Unresolved at evaporation
//! = `voided = true` so the coordinator refunds entries off-chain.
//!
//! Deploy-time params:
//!   - `initial_energy`  — contract lifetime budget (sized to cover
//!                         enrolment + draw + claim window)
//!   - `half_life`       — energy decay rate
//!   - `default_prize`   — wallet form pre-populates this as the
//!                         default prize for `set_event(prize, stake)`;
//!                         the actual prize is locked at runtime
//!   - `default_stake`   — wallet form pre-populates this as the
//!                         default per-entry stake; the actual stake
//!                         is locked at runtime
//!
//! Runtime args (exact prize amount, exact stake amount) belong to
//! the one-shot `set_event()` mutator — not deploy.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Lottery init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub default_prize: u64,
    pub default_stake: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
