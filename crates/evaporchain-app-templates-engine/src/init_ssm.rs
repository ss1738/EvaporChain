//! Singh Strategy Machines typed init.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid SSM init JSON: {0}")]
    Json(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    pub fragment: String,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t1_20_parse_success() {
        let cfg = parse(br#"{"fragment":"my-strategy"}"#).unwrap();
        assert_eq!(cfg.fragment, "my-strategy");
    }

    #[test]
    fn t1_20_parse_malformed_returns_json_error() {
        assert!(matches!(parse(b"not json"), Err(ParseError::Json(_))));
    }
}
