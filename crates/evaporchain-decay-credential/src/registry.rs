//! [`CredentialRegistry`] — a keyed collection of [`DecayCredential`]s
//! with issue / refresh / revoke / verify and subject lookup.

use std::collections::HashMap;

use crate::credential::{CredError, CredentialId, DecayCredential};

/// In-memory registry of decay-credentials keyed by id.
#[derive(Debug, Clone, Default)]
pub struct CredentialRegistry {
    creds: HashMap<CredentialId, DecayCredential>,
}

impl CredentialRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issue and store a new credential. Rejects a duplicate id and any
    /// degenerate parameter (see [`DecayCredential::issue`]).
    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        &mut self,
        id: CredentialId,
        issuer: [u8; 32],
        subject: [u8; 32],
        claim: String,
        initial_strength: u64,
        half_life: u64,
        validity_floor: u64,
        issued_at: u64,
    ) -> Result<CredentialId, CredError> {
        if self.creds.contains_key(&id) {
            return Err(CredError::DuplicateId);
        }
        let cred = DecayCredential::issue(
            id,
            issuer,
            subject,
            claim,
            initial_strength,
            half_life,
            validity_floor,
            issued_at,
        )?;
        self.creds.insert(id, cred);
        Ok(id)
    }

    pub fn get(&self, id: &CredentialId) -> Option<&DecayCredential> {
        self.creds.get(id)
    }

    /// Refresh a stored credential. Issuer-gated; `NotFound` if absent.
    pub fn refresh(
        &mut self,
        id: &CredentialId,
        caller: [u8; 32],
        top_up: u64,
        now: u64,
    ) -> Result<(), CredError> {
        self.creds
            .get_mut(id)
            .ok_or(CredError::NotFound)?
            .refresh(caller, top_up, now)
    }

    /// Revoke a stored credential. Issuer-gated; `NotFound` if absent.
    pub fn revoke(
        &mut self,
        id: &CredentialId,
        caller: [u8; 32],
        now: u64,
    ) -> Result<(), CredError> {
        self.creds
            .get_mut(id)
            .ok_or(CredError::NotFound)?
            .revoke(caller, now)
    }

    /// Whether a stored credential is valid at `now`. A missing
    /// credential is not valid.
    pub fn is_valid(&self, id: &CredentialId, now: u64) -> bool {
        self.creds.get(id).is_some_and(|c| c.is_valid_at(now))
    }

    /// All currently-valid credentials attesting `subject` at `now`.
    pub fn valid_credentials_for_subject(
        &self,
        subject: &[u8; 32],
        now: u64,
    ) -> Vec<&DecayCredential> {
        let mut out: Vec<&DecayCredential> = self
            .creds
            .values()
            .filter(|c| &c.subject == subject && c.is_valid_at(now))
            .collect();
        out.sort_by_key(|c| c.id);
        out
    }

    pub fn len(&self) -> usize {
        self.creds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.creds.is_empty()
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

    #[test]
    fn issue_then_get_and_verify() {
        let mut reg = CredentialRegistry::new();
        let id = reg
            .issue(cid(1), issuer(), subject(), "kyc".into(), 1_000, 10, 250, 0)
            .unwrap();
        assert_eq!(reg.len(), 1);
        assert!(reg.get(&id).is_some());
        assert!(reg.is_valid(&id, 0));
    }

    #[test]
    fn duplicate_id_rejected() {
        let mut reg = CredentialRegistry::new();
        reg.issue(cid(1), issuer(), subject(), "a".into(), 1_000, 10, 1, 0)
            .unwrap();
        let e = reg
            .issue(cid(1), issuer(), subject(), "b".into(), 1_000, 10, 1, 0)
            .unwrap_err();
        assert_eq!(e, CredError::DuplicateId);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn missing_credential_is_not_valid() {
        let reg = CredentialRegistry::new();
        assert!(!reg.is_valid(&cid(9), 0));
    }

    #[test]
    fn refresh_revoke_on_missing_id_is_not_found() {
        let mut reg = CredentialRegistry::new();
        assert_eq!(
            reg.refresh(&cid(9), issuer(), 1, 1),
            Err(CredError::NotFound)
        );
        assert_eq!(reg.revoke(&cid(9), issuer(), 1), Err(CredError::NotFound));
    }

    #[test]
    fn registry_enforces_issuer_authority() {
        let mut reg = CredentialRegistry::new();
        let id = reg
            .issue(cid(1), issuer(), subject(), "kyc".into(), 1_000, 10, 250, 0)
            .unwrap();
        assert_eq!(
            reg.refresh(&id, stranger(), 1_000, 5),
            Err(CredError::NotIssuer)
        );
        assert_eq!(reg.revoke(&id, stranger(), 5), Err(CredError::NotIssuer));
    }

    #[test]
    fn subject_lookup_filters_by_validity_and_subject() {
        let mut reg = CredentialRegistry::new();
        let other_subject = [0x44u8; 32];
        // valid for subject
        reg.issue(cid(1), issuer(), subject(), "kyc".into(), 1_000, 10, 250, 0)
            .unwrap();
        // expired for subject (floor 250, decays below by t=30)
        reg.issue(cid(2), issuer(), subject(), "rep".into(), 1_000, 10, 250, 0)
            .unwrap();
        // valid but for a different subject
        reg.issue(cid(3), issuer(), other_subject, "kyc".into(), 1_000, 10, 250, 0)
            .unwrap();

        // At t=0 both subject() creds are valid.
        let at0 = reg.valid_credentials_for_subject(&subject(), 0);
        assert_eq!(at0.len(), 2);

        // At t=30 (3 half-lives → strength 125 < floor 250) both have
        // expired → none valid.
        let at30 = reg.valid_credentials_for_subject(&subject(), 30);
        assert!(at30.is_empty());

        // Refresh cid(1) → it returns to the valid set, cid(2) stays out.
        reg.refresh(&cid(1), issuer(), 1_000, 30).unwrap();
        let after = reg.valid_credentials_for_subject(&subject(), 30);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, cid(1));
    }

    #[test]
    fn e2e_lifecycle() {
        let mut reg = CredentialRegistry::new();
        let id = reg
            .issue(
                cid(1),
                issuer(),
                subject(),
                "kyc:verified".into(),
                1_000_000,
                100,
                250_000,
                0,
            )
            .unwrap();

        // Issued → valid.
        assert!(reg.is_valid(&id, 0));
        // Decays below floor → invalid.
        assert!(!reg.is_valid(&id, 260));
        // Issuer refreshes → valid again.
        reg.refresh(&id, issuer(), 1_000_000, 260).unwrap();
        assert!(reg.is_valid(&id, 260));
        // Issuer revokes → terminally invalid.
        reg.revoke(&id, issuer(), 300).unwrap();
        assert!(!reg.is_valid(&id, 300));
        assert_eq!(reg.get(&id).unwrap().revoked_at, Some(300));
    }

    #[test]
    fn serde_roundtrip_of_a_credential() {
        let mut reg = CredentialRegistry::new();
        let id = reg
            .issue(cid(7), issuer(), subject(), "kyc".into(), 1_000, 10, 250, 0)
            .unwrap();
        let c = reg.get(&id).unwrap();
        let json = serde_json::to_string(c).unwrap();
        let back: DecayCredential = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, c);
    }

    proptest::proptest! {
        /// Strength never increases as time advances between refreshes,
        /// and validity implies strength ≥ floor.
        #[test]
        fn property_strength_monotone_and_validity_consistent(
            initial in 1u64..u64::MAX/2,
            half_life in 1u64..10_000u64,
            t1 in 0u64..100_000u64,
            dt in 0u64..100_000u64,
        ) {
            let floor = (initial / 4).max(1);
            let c = DecayCredential::issue(
                cid(1), issuer(), subject(), "p".into(), initial, half_life, floor, 0,
            ).unwrap();
            let s1 = c.strength_at(t1);
            let s2 = c.strength_at(t1 + dt);
            proptest::prop_assert!(s2 <= s1);
            if c.is_valid_at(t1) {
                proptest::prop_assert!(s1 >= floor);
            }
        }
    }
}
