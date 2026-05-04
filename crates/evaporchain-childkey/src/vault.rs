//! Singh Vault — opaque blob + key-share commitments.
//!
//! On-chain we keep only:
//!
//! - the BLAKE3 hash of the ciphertext (the actual ciphertext lives
//!   off-chain or in a separate blob layer)
//! - the threshold-cryptography committee — N validators with M-of-N
//!   key shares; on unlock, M shares combine to release the
//!   decryption key
//!
//! The protocol guarantees the *commitment* is durable; the threshold
//! committee is responsible for actually publishing the combined key
//! at unlock time. If the committee misbehaves (refuses to publish,
//! key-share leaked), that's a slashable event in a higher layer —
//! out of scope for this primitive.

use evaporchain_types::AccountAddress;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VaultError {
    #[error("threshold m must be > 0")]
    ZeroThreshold,
    #[error("threshold m={m} cannot exceed committee size n={n}")]
    ThresholdAboveCommittee { m: usize, n: usize },
    #[error("committee must be non-empty")]
    EmptyCommittee,
    #[error("committee contains duplicate validator")]
    DuplicateInCommittee,
}

/// A validator's commitment to hold one key share. The actual share
/// is exchanged off-chain via the threshold-DKG ceremony at sealing
/// time; on chain we record only the validator address (so we know
/// who is supposed to have the share).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyShareCommitment {
    pub validator: AccountAddress,
}

/// On-chain representation of the encrypted payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultBlob {
    /// BLAKE3 hash of the off-chain ciphertext. Validators verify
    /// the hash matches at unlock time before publishing the
    /// combined key.
    pub ciphertext_hash: [u8; 32],
    /// Length of the off-chain ciphertext in bytes (authenticated
    /// alongside the hash so the unlocker can size the download).
    pub ciphertext_len: u64,
    /// Threshold parameters: any `m_threshold` of `committee.len()`
    /// validators must publish their share to release the key.
    pub m_threshold: usize,
    /// Validators committed to hold key shares.
    pub committee: Vec<KeyShareCommitment>,
}

impl VaultBlob {
    /// Construct a vault, validating threshold + committee invariants.
    pub fn new(
        ciphertext_hash: [u8; 32],
        ciphertext_len: u64,
        m_threshold: usize,
        committee: Vec<KeyShareCommitment>,
    ) -> Result<Self, VaultError> {
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
        // Reject duplicate validators in the committee.
        let mut seen = std::collections::HashSet::new();
        for kc in &committee {
            if !seen.insert(kc.validator) {
                return Err(VaultError::DuplicateInCommittee);
            }
        }
        Ok(Self {
            ciphertext_hash,
            ciphertext_len,
            m_threshold,
            committee,
        })
    }

    pub fn committee_size(&self) -> usize {
        self.committee.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validator(b: u8) -> KeyShareCommitment {
        let mut a = [0u8; 32];
        a[0] = b;
        KeyShareCommitment { validator: a }
    }

    #[test]
    fn rejects_empty_committee() {
        let err = VaultBlob::new([0; 32], 100, 1, vec![]).unwrap_err();
        assert_eq!(err, VaultError::EmptyCommittee);
    }

    #[test]
    fn rejects_zero_threshold() {
        let err =
            VaultBlob::new([0; 32], 100, 0, vec![validator(1)]).unwrap_err();
        assert_eq!(err, VaultError::ZeroThreshold);
    }

    #[test]
    fn rejects_threshold_above_committee() {
        let err = VaultBlob::new(
            [0; 32],
            100,
            5,
            vec![validator(1), validator(2)],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            VaultError::ThresholdAboveCommittee { m: 5, n: 2 }
        ));
    }

    #[test]
    fn rejects_duplicate_validator() {
        let err = VaultBlob::new(
            [0; 32],
            100,
            2,
            vec![validator(1), validator(1), validator(2)],
        )
        .unwrap_err();
        assert_eq!(err, VaultError::DuplicateInCommittee);
    }

    #[test]
    fn valid_threshold_committee_constructs() {
        let v = VaultBlob::new(
            [0xAB; 32],
            1024,
            3,
            vec![validator(1), validator(2), validator(3), validator(4), validator(5)],
        )
        .unwrap();
        assert_eq!(v.committee_size(), 5);
        assert_eq!(v.m_threshold, 3);
    }

    #[test]
    fn round_trip_serde() {
        let v = VaultBlob::new(
            [0x77; 32],
            2048,
            2,
            vec![validator(1), validator(2), validator(3)],
        )
        .unwrap();
        let s = serde_json::to_string(&v).unwrap();
        let back: VaultBlob = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);
    }
}
