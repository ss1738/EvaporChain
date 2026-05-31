//! Mortal NFT (general transferable decaying NFT) typed init.
//!
//! Each NFT is its own contract instance; the contract's own energy
//! IS the NFT's lifespan. Distinct from Mayfly (the doctrine-purest
//! short-life version) — Mortal NFT carries holder lifecycle,
//! transfer count, collection identity, and metadata URI.
//!
//! Params:
//!   - `initial_energy` — the NFT's lifetime budget
//!   - `half_life`      — decay rate (epochs to halve energy)
//!
//! The runtime args (name, collection, metadata, recipient) are
//! NOT part of init — they're set by the deployer's subsequent
//! `set_metadata(...)` call after the contract instance exists.
//! Catalogue default_params therefore exposes the two energy
//! settings; the dApp form prompts for name / collection / metadata /
//! recipient at the call layer.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Mortal NFT init JSON: {0}")]
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
