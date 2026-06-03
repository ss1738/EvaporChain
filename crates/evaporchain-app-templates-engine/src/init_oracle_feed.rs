//! Oracle Feed (generic decaying data oracle) typed init.
//!
//! Standard oracles publish `(value, timestamp)` and force every
//! consumer to decide staleness themselves. OracleFeed inverts that:
//! the feed IS a decaying contract, `max_age` is a hard ceiling on
//! read-time freshness, and `is_fresh()` flips false structurally
//! rather than by consumer convention. on_evaporate ends the
//! publication surface; consumers who depended on the feed must
//! rebind to a fresh one — stale data being removed from chain is a
//! feature.
//!
//! Deploy-time params:
//!   - `initial_energy`   — contract lifetime budget (sized for the
//!                          operator's update cadence)
//!   - `half_life`        — energy decay rate
//!   - `default_max_age`  — wallet form pre-populates this as the
//!                          default freshness ceiling; the actual
//!                          `max_age` is locked at runtime via the
//!                          one-shot `set_feed(label, max_age)`
//!                          mutator
//!
//! Runtime args (the actual `label` string + the actual `max_age`
//! ceiling) belong to `set_feed()` — not deploy.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid OracleFeed init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub half_life: u64,
    pub default_max_age: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
