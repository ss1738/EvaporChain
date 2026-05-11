//! Singh-Resonance (Vital-Sign NFT) typed init.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Singh-Resonance init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub initial_energy: u64,
    pub base_half_life: u64,
    pub saturation: u64,
    pub max_scale_bp: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t1_20_parse_success() {
        let json = br#"{"initial_energy":1000,"base_half_life":100,"saturation":500,"max_scale_bp":10000}"#;
        let cfg = parse(json).unwrap();
        assert_eq!(cfg.initial_energy, 1000);
        assert_eq!(cfg.max_scale_bp, 10_000);
    }

    #[test]
    fn t1_20_parse_malformed_returns_json_error() {
        let r = parse(b"not json");
        assert!(matches!(r, Err(ParseError::Json(_))));
    }
}
