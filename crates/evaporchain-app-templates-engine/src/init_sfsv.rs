//! SFSV (Future-Self Vault) typed init.
//!
//! Mirrors the `.es` `set_terms(future_self_addr, predicate, release_param,
//! deposit_amount)` signature (EvaporScript-first — `future_self_vault.es`
//! is source of truth). Previously this only carried `release_epoch`, so
//! `EnergyDecaysBelow` (predicate type 1) vaults were undeployable via the
//! pipeline. `release_param` now generalises: it is the target epoch when
//! `predicate_type == 0` (EpochReached) and the energy threshold when
//! `predicate_type == 1` (EnergyDecaysBelow) — exactly as the `.es`
//! `set_terms` routes `release_param` into `release_epoch` vs `threshold`.
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid SFSV init JSON: {0}")]
    Json(String),
    #[error("unknown predicate_type {0} (0=EpochReached, 1=EnergyDecaysBelow)")]
    UnknownPredicate(u64),
    #[error("deposit_amount must be > 0")]
    ZeroDeposit,
    #[error("release_param must be > 0")]
    ZeroReleaseParam,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitConfig {
    /// Beneficiary (creator's future self) — 32-byte address, hex or
    /// base-encoded per the calldata convention.
    pub future_self: String,
    /// 0 = EpochReached, 1 = EnergyDecaysBelow. Matches `.es` predicate_type.
    pub predicate_type: u64,
    /// EpochReached → target epoch; EnergyDecaysBelow → energy threshold.
    /// Same field the `.es` `set_terms` demultiplexes into
    /// `release_epoch` / `threshold`.
    pub release_param: u64,
    /// Snapshot of the committed energy deposit (the contract's own
    /// energy field is authoritative on-chain; this is for accounting).
    pub deposit_amount: u64,
}

pub fn parse(calldata: &[u8]) -> Result<InitConfig, ParseError> {
    let cfg: InitConfig =
        serde_json::from_slice(calldata).map_err(|e| ParseError::Json(e.to_string()))?;
    // Mirror the `.es` set_terms require()s so a malformed deploy fails
    // here rather than reverting on-chain.
    if cfg.predicate_type > 1 {
        return Err(ParseError::UnknownPredicate(cfg.predicate_type));
    }
    if cfg.deposit_amount == 0 {
        return Err(ParseError::ZeroDeposit);
    }
    if cfg.release_param == 0 {
        return Err(ParseError::ZeroReleaseParam);
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_epoch_reached_vault() {
        let j = br#"{"future_self":"0xab","predicate_type":0,"release_param":1000,"deposit_amount":500}"#;
        let c = parse(j).unwrap();
        assert_eq!(c.predicate_type, 0);
        assert_eq!(c.release_param, 1000);
        assert_eq!(c.deposit_amount, 500);
    }

    #[test]
    fn parses_energy_decays_below_vault() {
        // Previously impossible — predicate_type 1 + threshold via release_param.
        let j = br#"{"future_self":"0xab","predicate_type":1,"release_param":250,"deposit_amount":1000}"#;
        let c = parse(j).unwrap();
        assert_eq!(c.predicate_type, 1);
        assert_eq!(c.release_param, 250); // = threshold
    }

    #[test]
    fn rejects_unknown_predicate() {
        let j = br#"{"future_self":"0xab","predicate_type":2,"release_param":1,"deposit_amount":1}"#;
        assert_eq!(parse(j).unwrap_err(), ParseError::UnknownPredicate(2));
    }

    #[test]
    fn rejects_zero_deposit_and_zero_release_param() {
        let j0 = br#"{"future_self":"0xab","predicate_type":0,"release_param":10,"deposit_amount":0}"#;
        assert_eq!(parse(j0).unwrap_err(), ParseError::ZeroDeposit);
        let j1 = br#"{"future_self":"0xab","predicate_type":0,"release_param":0,"deposit_amount":10}"#;
        assert_eq!(parse(j1).unwrap_err(), ParseError::ZeroReleaseParam);
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(matches!(parse(b"not json"), Err(ParseError::Json(_))));
    }
}
