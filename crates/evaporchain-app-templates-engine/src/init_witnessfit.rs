//! WitnessFit (Singh-Streak) typed init.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid WitnessFit init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub half_life: u64,
    pub boost_bp: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t1_20_parse_success() {
        let cfg = parse(br#"{"half_life":100,"boost_bp":1000}"#).unwrap();
        assert_eq!(cfg.half_life, 100);
        assert_eq!(cfg.boost_bp, 1000);
    }

    #[test]
    fn t1_20_parse_malformed_returns_json_error() {
        assert!(matches!(parse(b"not json"), Err(ParseError::Json(_))));
    }
}
