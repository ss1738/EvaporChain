//! Decay-credential substrate primitive.
//!
//! A credential (KYC pass, reputation badge, validator-eligibility
//! grant, access token) is issued with an `energy` (strength) and a
//! `half_life`. Its strength decays through the chain's canonical
//! `evaporchain_types::energy_at_epoch` halving curve. The credential
//! is **valid only while its strength stays at or above a floor** — so
//! a credential is not a permanent stamp. It evaporates unless the
//! issuer actively refreshes it, mirroring how the rest of the chain
//! treats state: nothing persists for free.
//!
//! This is the credential analogue of the energy-decay doctrine:
//! "trust is a flow, not a stock." A bank that verified you two years
//! ago has not verified you today; an attestation should fade the same
//! way energy does.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Validity decays monotonically.** Between refreshes a
//!    credential's strength is non-increasing (it routes through
//!    `energy_at_epoch`). Once strength drops below the floor the
//!    credential reads invalid — there is no "valid forever" path.
//!
//! 2. **Only the issuer can refresh or revoke.** The subject (or any
//!    third party) cannot self-renew or forge validity. Authority is
//!    bound to the issuer address that minted the credential.
//!
//! 3. **Refresh tops up the *decayed* value, never the original.**
//!    Refresh reads the current decayed strength, adds the top-up, and
//!    resets the decay clock — it is a rate-of-decay reset, not a
//!    rebate of strength already lost. Revoke is terminal (the
//!    credential must be re-issued to come back).
//!
//! ## Module map
//!
//! - [`credential`] — [`DecayCredential`] state machine + [`CredError`].
//! - [`registry`] — [`CredentialRegistry`]: issue / refresh / revoke /
//!   verify, plus subject lookup.

pub mod credential;
pub mod registry;

pub use credential::{CredError, CredentialId, DecayCredential};
pub use registry::CredentialRegistry;

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// Doctrine claim asserted as a structural test.
    ///
    /// Press claim: "A decay-credential is trust-as-a-flow. An issuer
    /// attests a subject with a strength that decays at a half-life;
    /// the attestation is valid only while strong enough, so it
    /// evaporates unless refreshed. Only the issuer can refresh or
    /// revoke. A subject cannot keep a stale credential alive, and a
    /// revoked credential stays dead."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let issuer = [0x15u8; 32];
        let subject = [0x5Bu8; 32];
        let stranger = [0xDDu8; 32];

        let mut reg = CredentialRegistry::new();
        // strength 1_000_000, half-life 100, valid while strength ≥ 250_000.
        let id = reg
            .issue(
                CredentialId([1u8; 32]),
                issuer,
                subject,
                "kyc:verified".into(),
                1_000_000,
                100,
                250_000,
                0,
            )
            .unwrap();

        // Fresh: valid.
        assert!(reg.is_valid(&id, 0));

        // After 2 half-lives strength ≈ 250_000 (the floor) → still
        // valid; after a bit more it dips below the floor → invalid.
        assert!(reg.is_valid(&id, 200));
        assert!(!reg.is_valid(&id, 260));

        // The subject cannot self-renew — authority is the issuer's.
        assert!(matches!(
            reg.refresh(&id, subject, 1_000_000, 260),
            Err(CredError::NotIssuer)
        ));
        // A stranger cannot revoke it either.
        assert!(matches!(
            reg.revoke(&id, stranger, 260),
            Err(CredError::NotIssuer)
        ));

        // The issuer refreshes → valid again.
        reg.refresh(&id, issuer, 1_000_000, 260).unwrap();
        assert!(reg.is_valid(&id, 260));

        // The issuer revokes → terminally invalid, even though it had
        // strength left.
        reg.revoke(&id, issuer, 300).unwrap();
        assert!(!reg.is_valid(&id, 300));
        // Refreshing a revoked credential is rejected — must re-issue.
        assert!(matches!(
            reg.refresh(&id, issuer, 1_000_000, 300),
            Err(CredError::AlreadyRevoked)
        ));
    }
}
