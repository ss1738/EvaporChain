//! `Successor` — one heir on a lineage.
//!
//! A successor declares an account address that will accrue
//! authority via the lineage's `Ladder`. Multiple successors can be
//! attached to one lineage, and each gets a `weight_bp` (basis
//! points) — *the fraction of the ladder's authority share* that
//! goes to this successor at any tier.
//!
//! Doctrine concrete shape: a parent leaves a wallet to two children.
//! Each child gets `weight_bp = 5_000` (50%). The lineage's ladder
//! tier at full dormancy reads `authority_share_bp = 10_000`
//! (100%); each successor's effective authority is
//! `share_bp × weight_bp / 10_000` = `5_000 bp` each, summing to
//! `10_000` (full).

use evaporchain_types::AccountAddress;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::tier::FULL_AUTHORITY_BP;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SuccessorError {
    #[error("weight_bp must be > 0")]
    ZeroWeight,
    #[error("weight_bp {bp} exceeds FULL_AUTHORITY_BP ({FULL_AUTHORITY_BP})")]
    WeightAboveOne { bp: u32 },
    #[error("successor address must not equal the issuer's")]
    SuccessorIsIssuer,
}

/// One designated heir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Successor {
    pub address: AccountAddress,
    /// Fraction of the ladder's tier authority that flows to this
    /// successor, in basis points (10_000 = 1.00).
    pub weight_bp: u32,
    /// Optional human-readable label ("daughter", "trustee",
    /// "secondary"). Stored on chain so the wallet can render it.
    pub label: String,
}

impl Successor {
    pub fn new(
        address: AccountAddress,
        weight_bp: u32,
        label: impl Into<String>,
        issuer: AccountAddress,
    ) -> Result<Self, SuccessorError> {
        if weight_bp == 0 {
            return Err(SuccessorError::ZeroWeight);
        }
        if weight_bp > FULL_AUTHORITY_BP {
            return Err(SuccessorError::WeightAboveOne { bp: weight_bp });
        }
        if address == issuer {
            return Err(SuccessorError::SuccessorIsIssuer);
        }
        Ok(Self {
            address,
            weight_bp,
            label: label.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(b: u8) -> AccountAddress {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    #[test]
    fn rejects_zero_weight() {
        let err = Successor::new(addr(2), 0, "daughter", addr(1)).unwrap_err();
        assert_eq!(err, SuccessorError::ZeroWeight);
    }

    #[test]
    fn rejects_weight_above_full() {
        let err = Successor::new(addr(2), 20_000, "daughter", addr(1)).unwrap_err();
        assert!(matches!(err, SuccessorError::WeightAboveOne { .. }));
    }

    #[test]
    fn rejects_self_succession() {
        let err = Successor::new(addr(1), 5_000, "self", addr(1)).unwrap_err();
        assert_eq!(err, SuccessorError::SuccessorIsIssuer);
    }

    #[test]
    fn valid_successor_constructs() {
        let s = Successor::new(addr(2), 5_000, "daughter", addr(1)).unwrap();
        assert_eq!(s.weight_bp, 5_000);
        assert_eq!(s.label, "daughter");
    }

    #[test]
    fn round_trip_serde() {
        let s = Successor::new(addr(2), 7_500, "trustee", addr(1)).unwrap();
        let j = serde_json::to_string(&s).unwrap();
        let back: Successor = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
