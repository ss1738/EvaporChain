//! `MortisCertificate` — the chain's death-certificate NFT.
//!
//! Singleton. Unowned. Visible to every light client forever.
//! Mints exactly once, when [`crate::monitor::MortisMonitor::tick`]
//! returns `JustTriggered`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_types::Energy;

const CERT_TAG: &[u8] = b"evaporchain-mortis-certificate";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MortisCertificate {
    /// State root at the moment of death.
    pub final_state_root: [u8; 32],
    /// Eulogy-trie root from `evaporchain-tombstone`.
    pub eulogy_trie_root: [u8; 32],
    /// Epoch the death trigger fired.
    pub epoch_of_death: u64,
    /// Refresh-pool total at death (≤ ε).
    pub final_refresh_pool: Energy,
    /// blake3 binding over all fields. Re-derivable by any verifier.
    pub witness: [u8; 32],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CertificateError {
    #[error(
        "witness mismatch: re-derived {derived:?}, certificate carries {claimed:?}"
    )]
    WitnessMismatch {
        derived: [u8; 32],
        claimed: [u8; 32],
    },
}

/// Mint the singleton death certificate.
pub fn mint_certificate(
    final_state_root: [u8; 32],
    eulogy_trie_root: [u8; 32],
    epoch_of_death: u64,
    final_refresh_pool: Energy,
) -> MortisCertificate {
    let witness = compute_witness(
        final_state_root,
        eulogy_trie_root,
        epoch_of_death,
        final_refresh_pool,
    );
    MortisCertificate {
        final_state_root,
        eulogy_trie_root,
        epoch_of_death,
        final_refresh_pool,
        witness,
    }
}

/// Re-derive the witness for an arbitrary certificate. Used by
/// verifiers / light clients to confirm the certificate hasn't been
/// tampered with.
pub fn verify_certificate(cert: &MortisCertificate) -> Result<(), CertificateError> {
    let derived = compute_witness(
        cert.final_state_root,
        cert.eulogy_trie_root,
        cert.epoch_of_death,
        cert.final_refresh_pool,
    );
    if derived != cert.witness {
        return Err(CertificateError::WitnessMismatch {
            derived,
            claimed: cert.witness,
        });
    }
    Ok(())
}

fn compute_witness(
    final_state_root: [u8; 32],
    eulogy_trie_root: [u8; 32],
    epoch_of_death: u64,
    final_refresh_pool: Energy,
) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(CERT_TAG);
    h.update(&final_state_root);
    h.update(&eulogy_trie_root);
    h.update(&epoch_of_death.to_le_bytes());
    h.update(&final_refresh_pool.to_le_bytes());
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_then_verify_round_trip() {
        let c = mint_certificate([1u8; 32], [2u8; 32], 1_000, 0);
        verify_certificate(&c).unwrap();
    }

    #[test]
    fn tampered_state_root_rejected() {
        let mut c = mint_certificate([1u8; 32], [2u8; 32], 1_000, 0);
        c.final_state_root[0] ^= 0xFF;
        let err = verify_certificate(&c).unwrap_err();
        assert!(matches!(err, CertificateError::WitnessMismatch { .. }));
    }

    #[test]
    fn tampered_eulogy_root_rejected() {
        let mut c = mint_certificate([1u8; 32], [2u8; 32], 1_000, 0);
        c.eulogy_trie_root[0] ^= 0xFF;
        let err = verify_certificate(&c).unwrap_err();
        assert!(matches!(err, CertificateError::WitnessMismatch { .. }));
    }

    #[test]
    fn tampered_epoch_rejected() {
        let mut c = mint_certificate([1u8; 32], [2u8; 32], 1_000, 0);
        c.epoch_of_death = 9_999_999;
        let err = verify_certificate(&c).unwrap_err();
        assert!(matches!(err, CertificateError::WitnessMismatch { .. }));
    }

    #[test]
    fn tampered_refresh_pool_rejected() {
        let mut c = mint_certificate([1u8; 32], [2u8; 32], 1_000, 0);
        c.final_refresh_pool = 999_999;
        let err = verify_certificate(&c).unwrap_err();
        assert!(matches!(err, CertificateError::WitnessMismatch { .. }));
    }

    #[test]
    fn deterministic_under_same_inputs() {
        let a = mint_certificate([7u8; 32], [3u8; 32], 100, 50);
        let b = mint_certificate([7u8; 32], [3u8; 32], 100, 50);
        assert_eq!(a, b);
    }
}
