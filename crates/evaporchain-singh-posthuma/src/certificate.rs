//! Threshold death certificate.
//!
//! V1 model: a `DeathCertificate` is the testament-id + death-epoch +
//! a committee-supplied nonce, plus a list of `Attestation`s — pairs
//! of `(validator, signature_blob)`. The signature blob is opaque
//! here; the chain's BLS / ML-DSA verifier (or whatever scheme the
//! committee uses off-chain) plugs in via a callback when verifying.
//!
//! V1 verifier: counts unique committee-member attestations with
//! valid signatures; certificate verifies iff count ≥ threshold. We
//! deliberately don't ship the signature primitive here — that's the
//! crypto crate's job. The crate exposes `verify_certificate` that
//! takes a `is_signature_valid` closure so production wiring plugs in
//! the real verifier and tests plug in a mock.
//!
//! This keeps the lifecycle math cleanly separated from the crypto
//! choice (BLS / FROST / ML-DSA / threshold-Ed25519) so it can move
//! when the chain's signing scheme moves.

use evaporchain_types::{AccountAddress, Epoch};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

use crate::testament::TestamentId;
use crate::vault::SealedVault;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CertificateError {
    #[error("certificate cites a different testament id")]
    WrongTestament,
    #[error("attestation list is empty")]
    NoAttestations,
    #[error("attestor {0:?} is not a committee member")]
    NotCommitteeMember(AccountAddress),
    #[error("attestor {0:?} appears more than once")]
    DuplicateAttestor(AccountAddress),
    #[error("signature for attestor {0:?} did not verify")]
    BadSignature(AccountAddress),
    #[error("only {valid} unique valid attestations; threshold is {threshold}")]
    BelowThreshold { valid: usize, threshold: usize },
}

/// One committee member's signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation {
    pub validator: AccountAddress,
    /// Opaque signature blob — verifier closure interprets it.
    pub signature: Vec<u8>,
}

/// Threshold death certificate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeathCertificate {
    pub testament_id: TestamentId,
    pub death_epoch: Epoch,
    /// Anti-replay nonce. Caller (the chain) supplies; should be a
    /// chain-derived value (block hash slice, beacon randomness) to
    /// stop committees pre-signing certificates.
    pub nonce: [u8; 32],
    pub attestations: Vec<Attestation>,
}

impl DeathCertificate {
    /// Canonical bytes that committee members signed. Deterministic;
    /// stable across validators.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32 + 8 + 32 + 4);
        bytes.extend_from_slice(&self.testament_id);
        bytes.extend_from_slice(&self.death_epoch.to_le_bytes());
        bytes.extend_from_slice(&self.nonce);
        // domain-separation tag — protects against cross-purpose
        // signatures being repurposed as death attestations.
        bytes.extend_from_slice(b"singh-posthuma:death:v1");
        bytes
    }
}

/// Verify the certificate against the vault's threshold + committee.
///
/// `is_signature_valid` is a caller-supplied closure that, given a
/// validator address, the canonical signing bytes, and the signature
/// blob, returns true iff the signature verifies under the
/// committee's threshold scheme. Tests pass a mock; production wires
/// in the chain's BLS / FROST verifier.
pub fn verify_certificate<F>(
    cert: &DeathCertificate,
    expected_testament: TestamentId,
    vault: &SealedVault,
    is_signature_valid: F,
) -> Result<(), CertificateError>
where
    F: Fn(&AccountAddress, &[u8], &[u8]) -> bool,
{
    if cert.testament_id != expected_testament {
        return Err(CertificateError::WrongTestament);
    }
    if cert.attestations.is_empty() {
        return Err(CertificateError::NoAttestations);
    }
    let signing_bytes = cert.signing_bytes();
    let mut seen: HashSet<AccountAddress> = HashSet::new();
    let mut valid: usize = 0;
    for att in &cert.attestations {
        if !vault.is_committee_member(&att.validator) {
            return Err(CertificateError::NotCommitteeMember(att.validator));
        }
        if !seen.insert(att.validator) {
            return Err(CertificateError::DuplicateAttestor(att.validator));
        }
        if !is_signature_valid(&att.validator, &signing_bytes, &att.signature) {
            return Err(CertificateError::BadSignature(att.validator));
        }
        valid += 1;
    }
    if valid < vault.m_threshold {
        return Err(CertificateError::BelowThreshold {
            valid,
            threshold: vault.m_threshold,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::SealedVault;

    fn validator(b: u8) -> AccountAddress {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    fn vault(threshold: usize) -> SealedVault {
        SealedVault::new(
            [0xAB; 32],
            1024,
            threshold,
            vec![
                validator(1),
                validator(2),
                validator(3),
                validator(4),
                validator(5),
            ],
            [0xCD; 32],
        )
        .unwrap()
    }

    fn cert_with(testament: TestamentId, signers: Vec<u8>) -> DeathCertificate {
        let attestations = signers
            .into_iter()
            .map(|b| Attestation {
                validator: validator(b),
                signature: vec![b; 8],
            })
            .collect();
        DeathCertificate {
            testament_id: testament,
            death_epoch: 100,
            nonce: [0xEE; 32],
            attestations,
        }
    }

    fn always_valid(_v: &AccountAddress, _b: &[u8], _s: &[u8]) -> bool {
        true
    }

    #[test]
    fn cert_with_threshold_signers_verifies() {
        let v = vault(3);
        let c = cert_with([0xAA; 32], vec![1, 2, 3]);
        verify_certificate(&c, [0xAA; 32], &v, always_valid).unwrap();
    }

    #[test]
    fn cert_below_threshold_rejected() {
        let v = vault(3);
        let c = cert_with([0xAA; 32], vec![1, 2]);
        let err = verify_certificate(&c, [0xAA; 32], &v, always_valid).unwrap_err();
        assert!(matches!(
            err,
            CertificateError::BelowThreshold {
                valid: 2,
                threshold: 3
            }
        ));
    }

    #[test]
    fn cert_for_wrong_testament_rejected() {
        let v = vault(2);
        let c = cert_with([0xAA; 32], vec![1, 2]);
        let err = verify_certificate(&c, [0xBB; 32], &v, always_valid).unwrap_err();
        assert_eq!(err, CertificateError::WrongTestament);
    }

    #[test]
    fn cert_with_non_member_rejected() {
        let v = vault(2);
        let c = cert_with([0xAA; 32], vec![1, 99]); // 99 not in committee
        let err = verify_certificate(&c, [0xAA; 32], &v, always_valid).unwrap_err();
        assert!(matches!(err, CertificateError::NotCommitteeMember(_)));
    }

    #[test]
    fn duplicate_attestor_rejected() {
        let v = vault(2);
        let c = cert_with([0xAA; 32], vec![1, 1, 2]);
        let err = verify_certificate(&c, [0xAA; 32], &v, always_valid).unwrap_err();
        assert!(matches!(err, CertificateError::DuplicateAttestor(_)));
    }

    #[test]
    fn empty_attestations_rejected() {
        let v = vault(1);
        let c = DeathCertificate {
            testament_id: [0xAA; 32],
            death_epoch: 1,
            nonce: [0; 32],
            attestations: vec![],
        };
        let err = verify_certificate(&c, [0xAA; 32], &v, always_valid).unwrap_err();
        assert_eq!(err, CertificateError::NoAttestations);
    }

    #[test]
    fn bad_signature_rejected() {
        let v = vault(2);
        let c = cert_with([0xAA; 32], vec![1, 2]);
        // Reject signatures from validator 2 specifically.
        let mock = |va: &AccountAddress, _b: &[u8], _s: &[u8]| va != &validator(2);
        let err = verify_certificate(&c, [0xAA; 32], &v, mock).unwrap_err();
        assert!(matches!(err, CertificateError::BadSignature(_)));
    }

    #[test]
    fn signing_bytes_include_domain_separation() {
        let c = cert_with([0xAA; 32], vec![1]);
        let s = c.signing_bytes();
        // Domain-separation tag must be in the bytes — protects
        // against cross-purpose signature replay.
        let dst = b"singh-posthuma:death:v1";
        assert!(
            s.windows(dst.len()).any(|w| w == dst),
            "signing bytes missing domain-separation tag"
        );
    }

    #[test]
    fn signing_bytes_change_with_inputs() {
        let mut a = cert_with([0xAA; 32], vec![1]);
        let mut b = cert_with([0xBB; 32], vec![1]);
        assert_ne!(a.signing_bytes(), b.signing_bytes());
        a.death_epoch = 1;
        b.death_epoch = 2;
        b.testament_id = [0xAA; 32];
        assert_ne!(a.signing_bytes(), b.signing_bytes());
    }

    #[test]
    fn round_trip_serde() {
        let c = cert_with([0xAA; 32], vec![1, 2, 3]);
        let s = serde_json::to_string(&c).unwrap();
        let back: DeathCertificate = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn the_pre_signing_attack_is_blocked_by_nonce() {
        // Doctrine guard: committees should not be able to pre-sign a
        // death certificate. The nonce is the chain's per-event
        // freshness — two certificates with different nonces produce
        // different signing_bytes, so a pre-signed signature on nonce
        // X is useless when the chain demands signature on nonce Y.
        let mut a = cert_with([0xAA; 32], vec![1]);
        let mut b = cert_with([0xAA; 32], vec![1]);
        b.nonce = [0xFF; 32];
        assert_ne!(a.signing_bytes(), b.signing_bytes());
        a.nonce[0] ^= 0x01;
        assert_ne!(a.signing_bytes(), b.signing_bytes());
    }
}
