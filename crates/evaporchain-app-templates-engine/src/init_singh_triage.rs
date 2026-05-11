//! Singh-Triage (Wallet Inbox) typed init.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid Singh-Triage init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub horizon_today: u64,
    pub horizon_tomorrow: u64,
    pub horizon_week: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t1_20_parse_success() {
        let json = br#"{"horizon_today":24,"horizon_tomorrow":48,"horizon_week":168}"#;
        let cfg = parse(json).unwrap();
        assert_eq!(cfg.horizon_today, 24);
    }

    #[test]
    fn t1_20_parse_malformed_returns_json_error() {
        assert!(matches!(parse(b"not json"), Err(ParseError::Json(_))));
    }
}
