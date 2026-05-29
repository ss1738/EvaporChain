//! [`DecayCredential`] — the credential state machine.
//!
//! Strength decays through `evaporchain_types::energy_at_epoch`; the
//! credential is valid only while strength ≥ `validity_floor`. Refresh
//! and revoke are gated to the issuer.

use evaporchain_types::energy_at_epoch;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 32-byte credential identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct CredentialId(pub [u8; 32]);

/// Errors for issue / refresh / revoke.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CredError {
    #[error("zero initial strength")]
    ZeroInitialStrength,
    #[error("zero half-life")]
    ZeroHalfLife,
    #[error("validity floor must be ≥ 1")]
    ZeroValidityFloor,
    #[error("validity floor {floor} exceeds initial strength {initial} — credential born invalid")]
    FloorExceedsInitial { floor: u64, initial: u64 },
    #[error("caller is not the issuer of this credential")]
    NotIssuer,
    #[error("credential is revoked — must be re-issued")]
    AlreadyRevoked,
    #[error("non-monotone time: incoming {incoming} < last_refreshed {last}")]
    NonMonotoneTime { incoming: u64, last: u64 },
    #[error("credential not found")]
    NotFound,
    #[error("credential id already exists")]
    DuplicateId,
}

/// An attestation whose strength decays over time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecayCredential {
    pub id: CredentialId,
    /// The address authorised to refresh or revoke.
    pub issuer: [u8; 32],
    /// The address the credential attests something about.
    pub subject: [u8; 32],
    /// The claim being attested, e.g. "kyc:verified" or "rep:gold".
    pub claim: String,
    /// Decay baseline: strength at `last_refreshed`. Decays from here.
    pub energy: u64,
    /// Half-life (epochs) of the strength decay.
    pub half_life: u64,
    /// Minimum strength for the credential to read as valid (≥ 1).
    pub validity_floor: u64,
    /// Epoch the credential was first issued.
    pub issued_at: u64,
    /// Epoch the decay clock was last reset (issue or refresh).
    pub last_refreshed: u64,
    /// Whether the issuer has revoked the credential (terminal).
    pub revoked: bool,
    /// Epoch of revocation, if revoked.
    pub revoked_at: Option<u64>,
}

impl DecayCredential {
    /// Issue a new credential. Rejects degenerate parameters that would
    /// make the credential never-valid or ill-defined.
    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        id: CredentialId,
        issuer: [u8; 32],
        subject: [u8; 32],
        claim: String,
        initial_strength: u64,
        half_life: u64,
        validity_floor: u64,
        issued_at: u64,
    ) -> Result<Self, CredError> {
        if initial_strength == 0 {
            return Err(CredError::ZeroInitialStrength);
        }
        if half_life == 0 {
            return Err(CredError::ZeroHalfLife);
        }
        if validity_floor == 0 {
            return Err(CredError::ZeroValidityFloor);
        }
        if validity_floor > initial_strength {
            return Err(CredError::FloorExceedsInitial {
                floor: validity_floor,
                initial: initial_strength,
            });
        }
        Ok(Self {
            id,
            issuer,
            subject,
            claim,
            energy: initial_strength,
            half_life,
            validity_floor,
            issued_at,
            last_refreshed: issued_at,
            revoked: false,
            revoked_at: None,
        })
    }

    /// Decay-adjusted strength at epoch `now`. A revoked credential has
    /// zero strength. Reads in the past (before `last_refreshed`) clamp
    /// to the baseline rather than growing.
    pub fn strength_at(&self, now: u64) -> u64 {
        if self.revoked {
            return 0;
        }
        let elapsed = now.saturating_sub(self.last_refreshed);
        energy_at_epoch(self.energy, self.half_life, elapsed)
    }

    /// Whether the credential is valid at epoch `now`: not revoked and
    /// strength still at or above the floor.
    pub fn is_valid_at(&self, now: u64) -> bool {
        !self.revoked && self.strength_at(now) >= self.validity_floor
    }

    /// Refresh: top up the *decayed* strength and reset the decay clock
    /// to `now`. Issuer-only; rejected on a revoked credential or a
    /// time that runs backwards.
    pub fn refresh(&mut self, caller: [u8; 32], top_up: u64, now: u64) -> Result<(), CredError> {
        if caller != self.issuer {
            return Err(CredError::NotIssuer);
        }
        if self.revoked {
            return Err(CredError::AlreadyRevoked);
        }
        if now < self.last_refreshed {
            return Err(CredError::NonMonotoneTime {
                incoming: now,
                last: self.last_refreshed,
            });
        }
        // Top up the value that survives decay to `now` — refresh is a
        // clock reset on the decayed strength, never a rebate of what
        // was already lost.
        self.energy = self.strength_at(now).saturating_add(top_up);
        self.last_refreshed = now;
        Ok(())
    }

    /// Revoke: terminally invalidate the credential. Issuer-only.
    pub fn revoke(&mut self, caller: [u8; 32], now: u64) -> Result<(), CredError> {
        if caller != self.issuer {
            return Err(CredError::NotIssuer);
        }
        if self.revoked {
            return Err(CredError::AlreadyRevoked);
        }
        self.revoked = true;
        self.revoked_at = Some(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(b: u8) -> CredentialId {
        CredentialId([b; 32])
    }
    fn issuer() -> [u8; 32] {
        [0x11; 32]
    }
    fn subject() -> [u8; 32] {
        [0x22; 32]
    }
    fn stranger() -> [u8; 32] {
        [0x33; 32]
    }

    fn fresh() -> DecayCredential {
        DecayCredential::issue(cid(1), issuer(), subject(), "kyc".into(), 1_000_000, 100, 250_000, 0)
            .unwrap()
    }

    // ── issue validation ─────────────────────────────────────────

    #[test]
    fn issue_rejects_zero_strength() {
        let e = DecayCredential::issue(cid(1), issuer(), subject(), "x".into(), 0, 100, 1, 0)
            .unwrap_err();
        assert_eq!(e, CredError::ZeroInitialStrength);
    }

    #[test]
    fn issue_rejects_zero_half_life() {
        let e = DecayCredential::issue(cid(1), issuer(), subject(), "x".into(), 100, 0, 1, 0)
            .unwrap_err();
        assert_eq!(e, CredError::ZeroHalfLife);
    }

    #[test]
    fn issue_rejects_zero_floor() {
        let e = DecayCredential::issue(cid(1), issuer(), subject(), "x".into(), 100, 10, 0, 0)
            .unwrap_err();
        assert_eq!(e, CredError::ZeroValidityFloor);
    }

    #[test]
    fn issue_rejects_floor_above_initial() {
        let e = DecayCredential::issue(cid(1), issuer(), subject(), "x".into(), 100, 10, 200, 0)
            .unwrap_err();
        assert!(matches!(e, CredError::FloorExceedsInitial { .. }));
    }

    #[test]
    fn fresh_credential_is_valid() {
        let c = fresh();
        assert!(c.is_valid_at(0));
        assert_eq!(c.strength_at(0), 1_000_000);
    }

    // ── decay + validity threshold ───────────────────────────────

    #[test]
    fn strength_decays_at_half_life() {
        let c = fresh();
        assert_eq!(c.strength_at(100), 500_000); // one half-life
        assert_eq!(c.strength_at(200), 250_000); // two half-lives → at floor
    }

    #[test]
    fn valid_while_at_or_above_floor_then_invalid_below() {
        let c = fresh();
        assert!(c.is_valid_at(200)); // strength == floor (250_000)
        assert!(!c.is_valid_at(260)); // dipped below floor
    }

    #[test]
    fn strength_is_monotone_non_increasing() {
        let c = fresh();
        let a = c.strength_at(10);
        let b = c.strength_at(50);
        let d = c.strength_at(150);
        assert!(a >= b && b >= d);
    }

    // ── refresh ──────────────────────────────────────────────────

    #[test]
    fn refresh_tops_up_decayed_value_and_resets_clock() {
        let mut c = fresh();
        // At t=100 strength is 500_000. Refresh +500_000 → 1_000_000.
        c.refresh(issuer(), 500_000, 100).unwrap();
        assert_eq!(c.last_refreshed, 100);
        assert_eq!(c.strength_at(100), 1_000_000);
        // Decay now restarts from 100.
        assert_eq!(c.strength_at(200), 500_000);
    }

    #[test]
    fn refresh_rebuilds_validity() {
        let mut c = fresh();
        assert!(!c.is_valid_at(260));
        c.refresh(issuer(), 1_000_000, 260).unwrap();
        assert!(c.is_valid_at(260));
    }

    #[test]
    fn refresh_by_non_issuer_rejected() {
        let mut c = fresh();
        assert_eq!(c.refresh(subject(), 1, 10), Err(CredError::NotIssuer));
        assert_eq!(c.refresh(stranger(), 1, 10), Err(CredError::NotIssuer));
    }

    #[test]
    fn refresh_in_the_past_rejected() {
        let mut c = fresh();
        c.refresh(issuer(), 1, 100).unwrap();
        assert!(matches!(
            c.refresh(issuer(), 1, 50),
            Err(CredError::NonMonotoneTime { .. })
        ));
    }

    // ── revoke ───────────────────────────────────────────────────

    #[test]
    fn revoke_makes_invalid_even_with_strength_left() {
        let mut c = fresh();
        assert!(c.is_valid_at(0));
        c.revoke(issuer(), 5).unwrap();
        assert!(!c.is_valid_at(5));
        assert_eq!(c.strength_at(5), 0);
        assert_eq!(c.revoked_at, Some(5));
    }

    #[test]
    fn revoke_by_non_issuer_rejected() {
        let mut c = fresh();
        assert_eq!(c.revoke(stranger(), 5), Err(CredError::NotIssuer));
        assert!(c.is_valid_at(0));
    }

    #[test]
    fn double_revoke_rejected() {
        let mut c = fresh();
        c.revoke(issuer(), 5).unwrap();
        assert_eq!(c.revoke(issuer(), 6), Err(CredError::AlreadyRevoked));
    }

    #[test]
    fn refresh_after_revoke_rejected() {
        let mut c = fresh();
        c.revoke(issuer(), 5).unwrap();
        assert_eq!(
            c.refresh(issuer(), 1_000_000, 6),
            Err(CredError::AlreadyRevoked)
        );
    }
}
