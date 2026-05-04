//! Skill credential — issued by an attestor to a holder.
//!
//! Credentials are *level scores at a point in time*, scoped to a
//! `SkillClass`. The class supplies the half-life; the credential
//! supplies (level, attested_at_epoch). Freshness derives mechanically
//! and changes every epoch — no on-chain "expiry" tx is needed.
//!
//! Refresh is the only state-changing operation: the attestor (or a
//! delegated assessment platform) bumps `attested_at_epoch` forward
//! and optionally raises/lowers `level`. The act of refreshing is
//! gated to the original attestor by default, but the protocol does
//! not enforce who can refresh — that's a higher-layer policy.

use evaporchain_types::{AccountAddress, Energy, Epoch};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::skill_class::SkillClassId;

/// 32-byte opaque credential handle.
pub type CredentialId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CredentialError {
    #[error("level must be > 0 to bind a credential")]
    ZeroLevel,
    #[error("refresh epoch {refresh_at} must be ≥ attested_at {attested_at}")]
    RefreshGoingBackwards {
        refresh_at: Epoch,
        attested_at: Epoch,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    pub id: CredentialId,
    pub class: SkillClassId,
    /// Who issued the credential (exam board, prior employer, peer).
    pub attestor: AccountAddress,
    /// Who currently holds the credential. Holder doesn't change
    /// after issuance — credentials are bound to the original subject;
    /// this is not a transferable asset (anti-Sybil property).
    pub holder: AccountAddress,
    /// Skill score, semantically interpreted by the class. Higher =
    /// more skilled; the meaning of the units is class-specific (the
    /// class registry is the source of truth).
    pub level: Energy,
    /// Epoch the credential was last attested. Used as the freshness
    /// origin: the class's half-life applied to (now - attested_at)
    /// gives the current decay-adjusted level.
    pub attested_at_epoch: Epoch,
}

impl Credential {
    pub fn issue(
        id: CredentialId,
        class: SkillClassId,
        attestor: AccountAddress,
        holder: AccountAddress,
        level: Energy,
        attested_at_epoch: Epoch,
    ) -> Result<Self, CredentialError> {
        if level == 0 {
            return Err(CredentialError::ZeroLevel);
        }
        Ok(Self {
            id,
            class,
            attestor,
            holder,
            level,
            attested_at_epoch,
        })
    }

    /// Refresh by bumping `attested_at_epoch` forward and updating
    /// `level`. The new epoch must be ≥ the previous attestation
    /// epoch (refreshing into the past is rejected — it would be
    /// equivalent to back-dating a credential).
    pub fn refresh(
        &mut self,
        new_level: Energy,
        refresh_at: Epoch,
    ) -> Result<(), CredentialError> {
        if refresh_at < self.attested_at_epoch {
            return Err(CredentialError::RefreshGoingBackwards {
                refresh_at,
                attested_at: self.attested_at_epoch,
            });
        }
        if new_level == 0 {
            return Err(CredentialError::ZeroLevel);
        }
        self.attested_at_epoch = refresh_at;
        self.level = new_level;
        Ok(())
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

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    #[test]
    fn issue_rejects_zero_level() {
        let err = Credential::issue(id(1), id(2), addr(0xAA), addr(0xBB), 0, 10)
            .unwrap_err();
        assert_eq!(err, CredentialError::ZeroLevel);
    }

    #[test]
    fn issue_records_attestation_inputs() {
        let c = Credential::issue(id(1), id(2), addr(0xAA), addr(0xBB), 80, 100).unwrap();
        assert_eq!(c.attestor, addr(0xAA));
        assert_eq!(c.holder, addr(0xBB));
        assert_eq!(c.level, 80);
        assert_eq!(c.attested_at_epoch, 100);
    }

    #[test]
    fn refresh_advances_epoch_and_updates_level() {
        let mut c =
            Credential::issue(id(1), id(2), addr(0xAA), addr(0xBB), 80, 100).unwrap();
        c.refresh(90, 200).unwrap();
        assert_eq!(c.attested_at_epoch, 200);
        assert_eq!(c.level, 90);
    }

    #[test]
    fn refresh_at_same_epoch_is_allowed() {
        let mut c =
            Credential::issue(id(1), id(2), addr(0xAA), addr(0xBB), 80, 100).unwrap();
        c.refresh(85, 100).unwrap();
        assert_eq!(c.level, 85);
    }

    #[test]
    fn refresh_into_past_rejected() {
        let mut c =
            Credential::issue(id(1), id(2), addr(0xAA), addr(0xBB), 80, 100).unwrap();
        let err = c.refresh(90, 50).unwrap_err();
        assert!(matches!(
            err,
            CredentialError::RefreshGoingBackwards { .. }
        ));
        // Original credential unchanged on error.
        assert_eq!(c.level, 80);
        assert_eq!(c.attested_at_epoch, 100);
    }

    #[test]
    fn refresh_with_zero_level_rejected() {
        let mut c =
            Credential::issue(id(1), id(2), addr(0xAA), addr(0xBB), 80, 100).unwrap();
        let err = c.refresh(0, 200).unwrap_err();
        assert_eq!(err, CredentialError::ZeroLevel);
    }

    #[test]
    fn round_trip_serde() {
        let c = Credential::issue(id(1), id(2), addr(0xAA), addr(0xBB), 99, 50).unwrap();
        let s = serde_json::to_string(&c).unwrap();
        let back: Credential = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }
}
