//! `SealedVault` — payload commitment + threshold committee + public
//! decryption-key commitment.
//!
//! On chain we keep:
//! - BLAKE3 hash of the off-chain ciphertext (the actual encrypted
//!   blob lives off-chain).
//! - Length of the ciphertext (so the unlocker knows what to download).
//! - The threshold committee — `N` validator addresses; `M` of them
//!   must sign a death certificate to release.
//! - A 32-byte `pubkey_commitment` — the BLAKE3 hash of the public
//!   threshold key. The actual key combination happens off-chain
//!   under FROST or similar; the chain only verifies its commitment
//!   matches what the committee distributed at sealing time.

use evaporchain_types::AccountAddress;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VaultError {
    #[error("committee must be non-empty")]
    EmptyCommittee,
    #[error("threshold m must be > 0")]
    ZeroThreshold,
    #[error("threshold m={m} cannot exceed committee size n={n}")]
    ThresholdAboveCommittee { m: usize, n: usize },
    #[error("committee contains a duplicate validator")]
    DuplicateMember,
    #[error("ciphertext_hash must not be all-zero")]
    EmptyCiphertextHash,
    #[error("ciphertext_len must be > 0")]
    ZeroCiphertextLen,
    #[error("pubkey_commitment must not be all-zero")]
    EmptyPubkeyCommitment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedVault {
    /// BLAKE3 hash of the off-chain encrypted payload.
    pub ciphertext_hash: [u8; 32],
    /// Byte length of that payload.
    pub ciphertext_len: u64,
    /// Threshold parameters: any `m_threshold` of `committee.len()`
    /// validators must sign the DeathCertificate to release.
    pub m_threshold: usize,
    /// Committee members (validator addresses). BTreeSet-canonicalised
    /// at construction; duplicates rejected.
    pub committee: Vec<AccountAddress>,
    /// BLAKE3 commitment to the threshold public key. Lets a verifier
    /// check the committee's published public key matches what was
    /// agreed at sealing time, without putting the key bytes on chain.
    pub pubkey_commitment: [u8; 32],
}

impl SealedVault {
    pub fn new(
        ciphertext_hash: [u8; 32],
        ciphertext_len: u64,
        m_threshold: usize,
        committee: Vec<AccountAddress>,
        pubkey_commitment: [u8; 32],
    ) -> Result<Self, VaultError> {
        if ciphertext_hash == [0u8; 32] {
            return Err(VaultError::EmptyCiphertextHash);
        }
        if ciphertext_len == 0 {
            return Err(VaultError::ZeroCiphertextLen);
        }
        if pubkey_commitment == [0u8; 32] {
            return Err(VaultError::EmptyPubkeyCommitment);
        }
        if committee.is_empty() {
            return Err(VaultError::EmptyCommittee);
        }
        if m_threshold == 0 {
            return Err(VaultError::ZeroThreshold);
        }
        if m_threshold > committee.len() {
            return Err(VaultError::ThresholdAboveCommittee {
                m: m_threshold,
                n: committee.len(),
            });
        }
        let mut seen: HashSet<AccountAddress> = HashSet::new();
        for v in &committee {
            if !seen.insert(*v) {
                return Err(VaultError::DuplicateMember);
            }
        }
        Ok(Self {
            ciphertext_hash,
            ciphertext_len,
            m_threshold,
            committee,
            pubkey_commitment,
        })
    }

    pub fn committee_size(&self) -> usize {
        self.committee.len()
    }

    /// True iff `validator` is a committee member. Used by the
    /// certificate verifier when counting signatures.
    pub fn is_committee_member(&self, validator: &AccountAddress) -> bool {
        self.committee.iter().any(|v| v == validator)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validator(b: u8) -> AccountAddress {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    #[test]
    fn rejects_empty_ciphertext_hash() {
        let err = SealedVault::new([0; 32], 10, 1, vec![validator(1)], [1; 32]).unwrap_err();
        assert_eq!(err, VaultError::EmptyCiphertextHash);
    }

    #[test]
    fn rejects_zero_ciphertext_len() {
        let err = SealedVault::new([1; 32], 0, 1, vec![validator(1)], [1; 32]).unwrap_err();
        assert_eq!(err, VaultError::ZeroCiphertextLen);
    }

    #[test]
    fn rejects_empty_pubkey_commitment() {
        let err = SealedVault::new([1; 32], 10, 1, vec![validator(1)], [0; 32]).unwrap_err();
        assert_eq!(err, VaultError::EmptyPubkeyCommitment);
    }

    #[test]
    fn rejects_empty_committee() {
        let err = SealedVault::new([1; 32], 10, 1, vec![], [1; 32]).unwrap_err();
        assert_eq!(err, VaultError::EmptyCommittee);
    }

    #[test]
    fn rejects_threshold_above_committee() {
        let err = SealedVault::new([1; 32], 10, 5, vec![validator(1), validator(2)], [1; 32])
            .unwrap_err();
        assert!(matches!(
            err,
            VaultError::ThresholdAboveCommittee { m: 5, n: 2 }
        ));
    }

    #[test]
    fn rejects_duplicate_member() {
        let err = SealedVault::new(
            [1; 32],
            10,
            2,
            vec![validator(1), validator(1), validator(2)],
            [1; 32],
        )
        .unwrap_err();
        assert_eq!(err, VaultError::DuplicateMember);
    }

    #[test]
    fn valid_vault_constructs() {
        let v = SealedVault::new(
            [0xAB; 32],
            1024,
            3,
            vec![
                validator(1),
                validator(2),
                validator(3),
                validator(4),
                validator(5),
            ],
            [0xCD; 32],
        )
        .unwrap();
        assert_eq!(v.committee_size(), 5);
        assert_eq!(v.m_threshold, 3);
        assert!(v.is_committee_member(&validator(2)));
        assert!(!v.is_committee_member(&validator(99)));
    }

    #[test]
    fn round_trip_serde() {
        let v = SealedVault::new(
            [0x77; 32],
            2048,
            2,
            vec![validator(1), validator(2), validator(3)],
            [0xCD; 32],
        )
        .unwrap();
        let s = serde_json::to_string(&v).unwrap();
        let back: SealedVault = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);
    }

    /// T1.20 — SealedVault::new rejects zero m_threshold (line 77).
    #[test]
    fn t1_20_rejects_zero_threshold() {
        let err = SealedVault::new([1; 32], 10, 0, vec![validator(1)], [1; 32]).unwrap_err();
        assert_eq!(err, VaultError::ZeroThreshold);
    }
}
