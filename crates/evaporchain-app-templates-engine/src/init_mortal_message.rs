//! Mortal Message (canonical EvaporScript pilot) typed init.
//!
//! Self-destructing message where the contract's own energy IS the
//! message lifespan. Per project CLAUDE.md "Two unifying invariants"
//! #2, this is the reference pilot every other .es contract follows.
//!
//! Params:
//!   - `initial_energy` — the message's lifetime budget
//!   - `half_life`      — decay rate (epochs to halve energy)
//!
//! Runtime args (body, recipient) belong to the subsequent
//! `set_payload(body, recipient)` call — they are NOT part of the
//! typed init, both for variable-length reasons and because the
//! message can be deployed empty and sealed later (the canonical
//! mint-then-populate flow).
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Mortal Message init JSON: {0}")]
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
