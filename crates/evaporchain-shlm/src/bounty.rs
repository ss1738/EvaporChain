//! Bounty — an employer's posted demand for a fresh skill credential.
//!
//! A `Bounty` declares: "I will pay up to `salary_cap` for a credential
//! in `class` whose freshness score is ≥ `min_freshness` and whose
//! level is ≥ `min_level`." Holders submit their credential as a "bid"
//! against the bounty; matching is filtered first (does the credential
//! pass thresholds at this epoch?) and then cleared via SDDC for the
//! salary axis.

use evaporchain_types::{AccountAddress, Energy, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::credential::Credential;
use crate::freshness::freshness_score;
use crate::skill_class::{SkillClass, SkillClassId};

/// 32-byte opaque bounty handle.
pub type BountyId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BountyError {
    #[error("salary_cap must be > 0")]
    ZeroSalary,
    #[error("min_level must be > 0")]
    ZeroMinLevel,
    #[error("class id does not match credential's class")]
    ClassMismatch,
}

/// Employer posting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounty {
    pub id: BountyId,
    pub employer: AccountAddress,
    pub class: SkillClassId,
    pub salary_cap: Energy,
    pub min_freshness: Energy,
    pub min_level: Energy,
    pub posted_at: Epoch,
}

impl Bounty {
    pub fn post(
        id: BountyId,
        employer: AccountAddress,
        class: SkillClassId,
        salary_cap: Energy,
        min_freshness: Energy,
        min_level: Energy,
        posted_at: Epoch,
    ) -> Result<Self, BountyError> {
        if salary_cap == 0 {
            return Err(BountyError::ZeroSalary);
        }
        if min_level == 0 {
            return Err(BountyError::ZeroMinLevel);
        }
        Ok(Self {
            id,
            employer,
            class,
            salary_cap,
            min_freshness,
            min_level,
            posted_at,
        })
    }

    /// True iff the credential, at `epoch_now`, satisfies the bounty's
    /// thresholds. This is the *eligibility filter* — clearing on the
    /// salary axis happens separately via SDDC.
    pub fn matches(
        &self,
        class: &SkillClass,
        cred: &Credential,
        epoch_now: Epoch,
    ) -> Result<bool, BountyError> {
        if class.id != self.class || cred.class != self.class {
            return Err(BountyError::ClassMismatch);
        }
        let score = freshness_score(class, cred, epoch_now);
        Ok(score >= self.min_freshness && cred.level >= self.min_level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> [u8; 32] {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn class() -> SkillClass {
        SkillClass::register(id(0xC1), "Python", 100, 0).unwrap()
    }

    fn cred_at(level: Energy, attested_at: Epoch) -> Credential {
        Credential::issue(id(0xCD), id(0xC1), id(0xAA), id(0xBB), level, attested_at).unwrap()
    }

    fn bounty_at(min_freshness: Energy, min_level: Energy) -> Bounty {
        Bounty::post(
            id(0xB0),
            id(0xEE),
            id(0xC1),
            100_000,
            min_freshness,
            min_level,
            500,
        )
        .unwrap()
    }

    #[test]
    fn rejects_zero_salary() {
        assert_eq!(
            Bounty::post(id(1), id(2), id(3), 0, 100, 50, 0).unwrap_err(),
            BountyError::ZeroSalary
        );
    }

    #[test]
    fn rejects_zero_min_level() {
        assert_eq!(
            Bounty::post(id(1), id(2), id(3), 1000, 100, 0, 0).unwrap_err(),
            BountyError::ZeroMinLevel
        );
    }

    #[test]
    fn matches_when_fresh_credential_meets_thresholds() {
        let c = class();
        let cr = cred_at(800, 0);
        let b = bounty_at(500, 100);
        // At epoch 0 (just-issued), score=800 ≥ 500, level=800 ≥ 100.
        assert!(b.matches(&c, &cr, 0).unwrap());
    }

    #[test]
    fn rejects_credential_with_too_decayed_score() {
        let c = class(); // half_life=100
        let cr = cred_at(1000, 0);
        let b = bounty_at(900, 100); // demands ≥ 900 freshness
                                     // At epoch 50 (half a half-life), score < 1000. After 1 half-life,
                                     // score ≈ 500 < 900 — definitely fails.
        assert!(!b.matches(&c, &cr, 100).unwrap());
    }

    #[test]
    fn rejects_credential_with_too_low_level() {
        let c = class();
        let cr = cred_at(50, 0); // level=50
        let b = bounty_at(10, 100); // demands level ≥ 100
        assert!(!b.matches(&c, &cr, 0).unwrap());
    }

    #[test]
    fn class_mismatch_errors() {
        let c = class(); // id = 0xC1
        let other_cred = Credential::issue(id(0xCD), id(0xC2), id(0xAA), id(0xBB), 800, 0).unwrap();
        let b = bounty_at(500, 100);
        let err = b.matches(&c, &other_cred, 0).unwrap_err();
        assert_eq!(err, BountyError::ClassMismatch);
    }

    #[test]
    fn round_trip_serde() {
        let b = bounty_at(500, 100);
        let s = serde_json::to_string(&b).unwrap();
        let back: Bounty = serde_json::from_str(&s).unwrap();
        assert_eq!(b, back);
    }
}
