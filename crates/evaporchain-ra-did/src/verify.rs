//! `verify_presentation` — the verifier-side gate.

use std::collections::HashMap;

use thiserror::Error;

use crate::attr::AttributeKey;
use crate::did::Presentation;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("attribute {key:?} energy {energy} below verifier floor {floor} — non-presentable")]
    EnergyBelowFloor {
        key: AttributeKey,
        energy: u64,
        floor: u64,
    },
    #[error("required attribute {0:?} missing from presentation")]
    MissingRequiredAttribute(AttributeKey),
    #[error("disallowed issuer {0:?} for attribute {1:?}")]
    DisallowedIssuer([u8; 32], AttributeKey),
}

/// Verifier policy: per-attribute energy floor + accepted issuer set.
///
/// Two structural decisions:
/// - **Per-attribute floor**: different verifiers may require
///   different freshness for different attributes (e.g., a bank
///   demands a 2-month-old KYC but accepts a 5-year-old age
///   verification).
/// - **Per-attribute issuer set**: a verifier may trust some
///   issuers and not others. `None` = accept any issuer.
#[derive(Debug, Clone, Default)]
pub struct VerifierPolicy {
    pub min_energy: HashMap<AttributeKey, u64>,
    pub allowed_issuers: HashMap<AttributeKey, Vec<[u8; 32]>>,
    /// Required attributes — verification fails if any is missing
    /// from the presentation.
    pub required: Vec<AttributeKey>,
}

impl VerifierPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_min_energy(mut self, key: AttributeKey, floor: u64) -> Self {
        self.min_energy.insert(key, floor);
        self
    }

    pub fn with_allowed_issuers(mut self, key: AttributeKey, issuers: Vec<[u8; 32]>) -> Self {
        self.allowed_issuers.insert(key, issuers);
        self
    }

    pub fn requiring(mut self, key: AttributeKey) -> Self {
        self.required.push(key);
        self
    }
}

/// Check a presentation against a verifier policy. Returns
/// `Ok(())` iff every disclosed attribute clears its floor and
/// allowed-issuer set, and every required attribute is disclosed.
pub fn verify_presentation(
    pres: &Presentation,
    policy: &VerifierPolicy,
) -> Result<(), VerifyError> {
    // Energy floor + issuer checks per disclosed attribute.
    for a in &pres.attributes {
        if let Some(&floor) = policy.min_energy.get(&a.key) {
            if a.energy < floor {
                return Err(VerifyError::EnergyBelowFloor {
                    key: a.key,
                    energy: a.energy,
                    floor,
                });
            }
        }
        if let Some(issuers) = policy.allowed_issuers.get(&a.key) {
            if !issuers.contains(&a.issuer) {
                return Err(VerifyError::DisallowedIssuer(a.issuer, a.key));
            }
        }
    }

    // Required-attribute presence check.
    for req in &policy.required {
        if !pres.attributes.iter().any(|a| a.key == *req) {
            return Err(VerifyError::MissingRequiredAttribute(*req));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attr::Attribute;
    use crate::did::{Did, DidId};

    fn did(b: u8) -> DidId {
        DidId([b; 32])
    }
    fn k(b: u8) -> AttributeKey {
        AttributeKey([b; 32])
    }

    fn attr_at(key: AttributeKey, energy: u64, issuer_byte: u8) -> Attribute {
        Attribute::new(key, b"value".to_vec(), [issuer_byte; 32], energy, 100)
    }

    fn build_did(id: DidId, attrs: Vec<Attribute>) -> Did {
        let mut d = Did::new(id);
        for a in attrs {
            d.add_attribute(a).unwrap();
        }
        d
    }

    // ── basic gate ────────────────────────────────────────────────

    #[test]
    fn empty_policy_accepts_any_presentation() {
        let d = build_did(did(1), vec![attr_at(k(1), 1000, 0xAB)]);
        let p = d.present(&[k(1)]).unwrap();
        verify_presentation(&p, &VerifierPolicy::new()).unwrap();
    }

    #[test]
    fn empty_presentation_passes_empty_policy() {
        let d = Did::new(did(1));
        let p = d.present(&[]).unwrap();
        verify_presentation(&p, &VerifierPolicy::new()).unwrap();
    }

    // ── energy floor — the load-bearing claim ─────────────────────

    #[test]
    fn fresh_attribute_clears_floor() {
        let d = build_did(did(1), vec![attr_at(k(1), 1000, 0xAB)]);
        let p = d.present(&[k(1)]).unwrap();
        let policy = VerifierPolicy::new().with_min_energy(k(1), 100);
        verify_presentation(&p, &policy).unwrap();
    }

    #[test]
    fn decayed_attribute_rejected() {
        let mut d = build_did(did(1), vec![attr_at(k(1), 1000, 0xAB)]);
        d.decay_attribute(k(1), 50).unwrap();
        let p = d.present(&[k(1)]).unwrap();
        let policy = VerifierPolicy::new().with_min_energy(k(1), 100);
        let err = verify_presentation(&p, &policy).unwrap_err();
        assert!(matches!(err, VerifyError::EnergyBelowFloor { .. }));
    }

    #[test]
    fn re_attestation_restores_presentability() {
        let mut d = build_did(did(1), vec![attr_at(k(1), 1000, 0xAB)]);
        d.decay_attribute(k(1), 50).unwrap();
        // Re-attest with fresh energy.
        d.re_attest(attr_at(k(1), 1000, 0xAB)).unwrap();
        let p = d.present(&[k(1)]).unwrap();
        let policy = VerifierPolicy::new().with_min_energy(k(1), 100);
        verify_presentation(&p, &policy).unwrap();
    }

    // ── per-attribute heterogeneous floors ────────────────────────

    #[test]
    fn different_attributes_have_independent_floors() {
        // Bank verifier: KYC must be very fresh, age can be older.
        let mut d = build_did(
            did(1),
            vec![attr_at(k(1), 50, 0xAB), attr_at(k(2), 1000, 0xAB)],
        );
        // KYC has decayed but is above its low floor; age is fresh.
        let _ = &mut d;
        let p = d.present(&[k(1), k(2)]).unwrap();
        let policy = VerifierPolicy::new()
            .with_min_energy(k(1), 10) // permissive floor
            .with_min_energy(k(2), 500); // strict floor
        verify_presentation(&p, &policy).unwrap();
    }

    #[test]
    fn one_attribute_decayed_but_not_required_passes() {
        let mut d = build_did(
            did(1),
            vec![attr_at(k(1), 1000, 0xAB), attr_at(k(2), 1000, 0xAB)],
        );
        d.decay_attribute(k(2), 10).unwrap();
        // Verifier doesn't ask about k(2) — only discloses k(1).
        let p = d.present(&[k(1)]).unwrap();
        let policy = VerifierPolicy::new().with_min_energy(k(1), 100);
        verify_presentation(&p, &policy).unwrap();
    }

    // ── allowed issuers ───────────────────────────────────────────

    #[test]
    fn issuer_in_allow_list_passes() {
        let d = build_did(did(1), vec![attr_at(k(1), 1000, 0xAB)]);
        let p = d.present(&[k(1)]).unwrap();
        let policy = VerifierPolicy::new().with_allowed_issuers(k(1), vec![[0xAB; 32]]);
        verify_presentation(&p, &policy).unwrap();
    }

    #[test]
    fn issuer_not_in_allow_list_rejected() {
        let d = build_did(did(1), vec![attr_at(k(1), 1000, 0xAB)]);
        let p = d.present(&[k(1)]).unwrap();
        let policy = VerifierPolicy::new().with_allowed_issuers(k(1), vec![[0xCD; 32]]);
        let err = verify_presentation(&p, &policy).unwrap_err();
        assert!(matches!(err, VerifyError::DisallowedIssuer(_, _)));
    }

    // ── required attributes ───────────────────────────────────────

    #[test]
    fn missing_required_attribute_rejected() {
        let d = build_did(did(1), vec![attr_at(k(1), 1000, 0xAB)]);
        let p = d.present(&[k(1)]).unwrap(); // discloses k(1) only
        let policy = VerifierPolicy::new().requiring(k(2));
        let err = verify_presentation(&p, &policy).unwrap_err();
        assert_eq!(err, VerifyError::MissingRequiredAttribute(k(2)));
    }

    #[test]
    fn disclosed_required_attribute_passes() {
        let d = build_did(did(1), vec![attr_at(k(1), 1000, 0xAB)]);
        let p = d.present(&[k(1)]).unwrap();
        let policy = VerifierPolicy::new().requiring(k(1));
        verify_presentation(&p, &policy).unwrap();
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "RA-DID is the first DID where Sybil resistance
        // is structural. A fresh DID has no attributes; each
        // attribute requires energy at attestation that decays
        // under the chain's λ. Spinning up N synthetic identities
        // requires earning N · |attribute_set| attestations at
        // full cost — there is no cheap clone path."
        //
        // Demonstrate: a DID with a fresh KYC attestation
        // satisfies a bank verifier; a stale-KYC DID does not;
        // a fresh-but-DIFFERENT-issuer attestation also doesn't
        // (the bank trusts only specific KYC issuers).

        let bank_kyc_issuer: [u8; 32] = [0xBB; 32];
        let other_issuer: [u8; 32] = [0xFF; 32];

        // Verifier: bank requires fresh KYC from one of two trusted issuers.
        let policy = VerifierPolicy::new()
            .requiring(k(1))
            .with_min_energy(k(1), 500)
            .with_allowed_issuers(k(1), vec![bank_kyc_issuer]);

        // Honest DID: fresh KYC from the trusted issuer. Passes.
        let d_honest = build_did(
            did(1),
            vec![Attribute::new(
                k(1),
                b"alice".to_vec(),
                bank_kyc_issuer,
                1000,
                100,
            )],
        );
        verify_presentation(&d_honest.present(&[k(1)]).unwrap(), &policy).unwrap();

        // Stale KYC: same DID but energy decayed. Rejected.
        let mut d_stale = d_honest.clone();
        d_stale.decay_attribute(k(1), 100).unwrap();
        let err = verify_presentation(&d_stale.present(&[k(1)]).unwrap(), &policy).unwrap_err();
        assert!(matches!(err, VerifyError::EnergyBelowFloor { .. }));

        // Cheap-clone Sybil: spawn a fresh DID with a fresh KYC
        // from an UN-TRUSTED issuer. Even though the energy is
        // high, the issuer is rejected.
        let d_sybil = build_did(
            did(2),
            vec![Attribute::new(
                k(1),
                b"bob".to_vec(),
                other_issuer,
                1000,
                100,
            )],
        );
        let err = verify_presentation(&d_sybil.present(&[k(1)]).unwrap(), &policy).unwrap_err();
        assert!(matches!(err, VerifyError::DisallowedIssuer(_, _)));
    }
}
