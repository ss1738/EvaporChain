//! Refresh-Market (AMM-priced namespace rent) typed init.
//!
//! Per `crates/evaporchain-refresh-market`: each namespace declares a
//! `capacity` (max concurrent active slots) at registration, and the
//! per-epoch rent rate is an AMM curve over `(used, capacity)`:
//!     `rent_rate(u, c) = base × (u + 1)² / c²`
//!
//! Deploying a refresh-market template registers a new namespace.
//! The `id_hex` is the namespace identifier (hex-encoded byte string),
//! `capacity` is the max-slots declaration, `base_rent` is the AMM
//! base coefficient.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid RefreshMarket init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    /// Hex-encoded namespace id bytes.
    pub id_hex: String,
    /// Max concurrent active slots in this namespace.
    pub capacity: u64,
    /// AMM base coefficient — per-epoch rent at zero utilisation
    /// equals `base_rent / capacity²`.
    pub base_rent: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}
