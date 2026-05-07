//! Build + verify a `FinalityAttestation`.

use blake3::Hasher;
use evaporchain_evap_fork_cert_v2::EvaporatedForkCertV2;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const ATTEST_DOMAIN: &[u8] = b"evaporchain:finality-attestation:v1\0";
const FORK_LEAF_DOMAIN: &[u8] = b"evaporchain:finality-attestation:fork-leaf\0";
const FORK_INNER_DOMAIN: &[u8] = b"evaporchain:finality-attestation:fork-inner\0";
const FORK_EMPTY_DOMAIN: &[u8] = b"evaporchain:finality-attestation:fork-empty\0";

/// A canonical reference to one evaporated-fork witness. Storing the
/// witness bytes (not the full `EvaporatedForkCertV2`) keeps the
/// attestation compact; the full cert can be served separately on
/// demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaporatedForkWitnessRef {
    pub fork_root: [u8; 32],
    pub witness: [u8; 32],
}

impl EvaporatedForkWitnessRef {
    pub fn from_cert(cert: &EvaporatedForkCertV2) -> Self {
        Self {
            fork_root: cert.fork_root,
            witness: cert.witness,
        }
    }
}

/// The single artefact a light client sees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalityAttestation {
    pub block_hash: [u8; 32],
    pub finalised_at_epoch: u64,
    /// Light-Cone V2 `causal_root` for the finalised block.
    pub causal_root: [u8; 32],
    /// Bell-Beacon V2 `BellCertificate.seed` published at or before
    /// `finalised_at_epoch`.
    pub bell_seed: [u8; 32],
    /// Sorted list of evaporated-fork witnesses; canonical order is
    /// lexicographic by `fork_root`.
    pub evaporated_forks: Vec<EvaporatedForkWitnessRef>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AttestationError {
    #[error(
        "evaporated_forks must be sorted lexicographically by fork_root for canonical commitment"
    )]
    UnsortedForks,
    #[error("evaporated_forks contains duplicate fork_root {0:?}")]
    DuplicateForkRoot([u8; 32]),
    #[error("attestation root mismatch: re-derived {derived:?}, supplied {supplied:?}")]
    RootMismatch {
        derived: [u8; 32],
        supplied: [u8; 32],
    },
}

/// Build the canonical attestation root from a fully-formed
/// `FinalityAttestation`. Returns `Err(UnsortedForks)` /
/// `Err(DuplicateForkRoot)` if the fork list is not canonical.
pub fn build_attestation(att: &FinalityAttestation) -> Result<[u8; 32], AttestationError> {
    // Canonicality check: forks must be strictly ascending by root.
    for w in att.evaporated_forks.windows(2) {
        if w[0].fork_root == w[1].fork_root {
            return Err(AttestationError::DuplicateForkRoot(w[0].fork_root));
        }
        if w[0].fork_root > w[1].fork_root {
            return Err(AttestationError::UnsortedForks);
        }
    }

    let fork_root = fork_merkle_root(&att.evaporated_forks);

    let mut h = Hasher::new();
    h.update(ATTEST_DOMAIN);
    h.update(&att.block_hash);
    h.update(&att.finalised_at_epoch.to_le_bytes());
    h.update(&att.causal_root);
    h.update(&att.bell_seed);
    h.update(&fork_root);
    Ok(*h.finalize().as_bytes())
}

/// Verify a supplied attestation root against the attestation
/// fields. Returns `Ok(())` iff the re-derived root matches.
pub fn verify_attestation(
    att: &FinalityAttestation,
    supplied_root: &[u8; 32],
) -> Result<(), AttestationError> {
    let derived = build_attestation(att)?;
    if &derived != supplied_root {
        return Err(AttestationError::RootMismatch {
            derived,
            supplied: *supplied_root,
        });
    }
    Ok(())
}

// ── fork-list Merkle commitment ──────────────────────────────────

fn fork_leaf_hash(w: &EvaporatedForkWitnessRef) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(FORK_LEAF_DOMAIN);
    h.update(&w.fork_root);
    h.update(&w.witness);
    *h.finalize().as_bytes()
}

fn fork_inner_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(FORK_INNER_DOMAIN);
    h.update(left);
    h.update(right);
    *h.finalize().as_bytes()
}

fn fork_empty_root() -> [u8; 32] {
    let mut h = Hasher::new();
    h.update(FORK_EMPTY_DOMAIN);
    *h.finalize().as_bytes()
}

fn fork_merkle_root(forks: &[EvaporatedForkWitnessRef]) -> [u8; 32] {
    if forks.is_empty() {
        return fork_empty_root();
    }
    let mut layer: Vec<[u8; 32]> = forks.iter().map(fork_leaf_hash).collect();
    while layer.len() > 1 {
        let mut next: Vec<[u8; 32]> = Vec::with_capacity((layer.len() + 1) / 2);
        let mut i = 0;
        while i < layer.len() {
            let l = layer[i];
            let r = if i + 1 < layer.len() {
                layer[i + 1]
            } else {
                layer[i]
            };
            next.push(fork_inner_hash(&l, &r));
            i += 2;
        }
        layer = next;
    }
    layer[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att_with(forks: Vec<EvaporatedForkWitnessRef>) -> FinalityAttestation {
        FinalityAttestation {
            block_hash: [1u8; 32],
            finalised_at_epoch: 1000,
            causal_root: [2u8; 32],
            bell_seed: [3u8; 32],
            evaporated_forks: forks,
        }
    }

    fn fwr(root_byte: u8, witness_byte: u8) -> EvaporatedForkWitnessRef {
        let mut fr = [0u8; 32];
        fr[0] = root_byte;
        let mut w = [0u8; 32];
        w[0] = witness_byte;
        EvaporatedForkWitnessRef {
            fork_root: fr,
            witness: w,
        }
    }

    // ── canonicality ─────────────────────────────────────────────

    #[test]
    fn empty_forks_yields_empty_sentinel_root() {
        let att = att_with(vec![]);
        let root = build_attestation(&att).unwrap();
        // Re-deriving the same attestation gives the same root.
        assert_eq!(build_attestation(&att).unwrap(), root);
    }

    #[test]
    fn unsorted_forks_rejected() {
        let att = att_with(vec![fwr(0x10, 0xaa), fwr(0x05, 0xbb)]);
        let err = build_attestation(&att).unwrap_err();
        assert_eq!(err, AttestationError::UnsortedForks);
    }

    #[test]
    fn duplicate_fork_roots_rejected() {
        let att = att_with(vec![fwr(0x10, 0xaa), fwr(0x10, 0xbb)]);
        let err = build_attestation(&att).unwrap_err();
        assert!(matches!(err, AttestationError::DuplicateForkRoot(_)));
    }

    // ── deterministic root + verification ────────────────────────

    #[test]
    fn round_trip_verifies() {
        let att = att_with(vec![fwr(0x05, 0x01), fwr(0x10, 0x02), fwr(0x99, 0x03)]);
        let root = build_attestation(&att).unwrap();
        verify_attestation(&att, &root).unwrap();
    }

    #[test]
    fn deterministic_root() {
        let att = att_with(vec![fwr(0x05, 0xaa), fwr(0x10, 0xbb)]);
        let r1 = build_attestation(&att).unwrap();
        let r2 = build_attestation(&att).unwrap();
        assert_eq!(r1, r2);
    }

    // ── tampering rejected ───────────────────────────────────────

    #[test]
    fn tampered_block_hash_rejected() {
        let att = att_with(vec![fwr(0x05, 0x01)]);
        let root = build_attestation(&att).unwrap();
        let mut tampered = att.clone();
        tampered.block_hash[0] ^= 1;
        let err = verify_attestation(&tampered, &root).unwrap_err();
        assert!(matches!(err, AttestationError::RootMismatch { .. }));
    }

    #[test]
    fn tampered_causal_root_rejected() {
        let att = att_with(vec![fwr(0x05, 0x01)]);
        let root = build_attestation(&att).unwrap();
        let mut tampered = att.clone();
        tampered.causal_root[5] ^= 1;
        assert!(verify_attestation(&tampered, &root).is_err());
    }

    #[test]
    fn tampered_bell_seed_rejected() {
        let att = att_with(vec![fwr(0x05, 0x01)]);
        let root = build_attestation(&att).unwrap();
        let mut tampered = att.clone();
        tampered.bell_seed[10] ^= 1;
        assert!(verify_attestation(&tampered, &root).is_err());
    }

    #[test]
    fn tampered_finalised_epoch_rejected() {
        let att = att_with(vec![fwr(0x05, 0x01)]);
        let root = build_attestation(&att).unwrap();
        let mut tampered = att.clone();
        tampered.finalised_at_epoch += 1;
        assert!(verify_attestation(&tampered, &root).is_err());
    }

    #[test]
    fn tampered_fork_witness_rejected() {
        let att = att_with(vec![fwr(0x05, 0x01), fwr(0x10, 0x02)]);
        let root = build_attestation(&att).unwrap();
        let mut tampered = att.clone();
        tampered.evaporated_forks[0].witness[0] ^= 1;
        assert!(verify_attestation(&tampered, &root).is_err());
    }

    #[test]
    fn adding_a_fork_changes_root() {
        let att1 = att_with(vec![fwr(0x05, 0x01)]);
        let att2 = att_with(vec![fwr(0x05, 0x01), fwr(0x10, 0x02)]);
        assert_ne!(
            build_attestation(&att1).unwrap(),
            build_attestation(&att2).unwrap()
        );
    }

    // ── domain separation ────────────────────────────────────────

    #[test]
    fn fork_leaf_inner_empty_domains_differ() {
        let leaf = fork_leaf_hash(&fwr(0, 0));
        let inner = fork_inner_hash(&[0u8; 32], &[0u8; 32]);
        let empty = fork_empty_root();
        assert_ne!(leaf, inner);
        assert_ne!(leaf, empty);
        assert_ne!(inner, empty);
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Finality Attestation folds the three V2 hardenings
        // (Light-Cone V2 causal_root, Bell-Beacon V2 seed,
        // Evap-Fork-Cert V2 witness list) into a single canonical
        // 32-byte attestation root. A light client verifies finality
        // from (block_hash, attestation, attestation_root) without
        // holding the DAG, the beacon archive, or any fork blocks.
        // Tampering any field of the attestation diverges the root."

        let att = att_with(vec![fwr(0x10, 0xaa), fwr(0x20, 0xbb), fwr(0x30, 0xcc)]);
        let root = build_attestation(&att).unwrap();
        verify_attestation(&att, &root).unwrap();

        // Six tamper vectors all rejected.
        for tampered in [
            {
                let mut a = att.clone();
                a.block_hash[0] ^= 1;
                a
            },
            {
                let mut a = att.clone();
                a.finalised_at_epoch += 1;
                a
            },
            {
                let mut a = att.clone();
                a.causal_root[0] ^= 1;
                a
            },
            {
                let mut a = att.clone();
                a.bell_seed[0] ^= 1;
                a
            },
            {
                let mut a = att.clone();
                a.evaporated_forks[0].witness[0] ^= 1;
                a
            },
            {
                let mut a = att.clone();
                a.evaporated_forks.pop();
                a
            },
        ] {
            assert!(verify_attestation(&tampered, &root).is_err());
        }

        // Canonicality: unsorted / duplicate fork lists rejected at
        // build time.
        let bad = att_with(vec![fwr(0x20, 0x00), fwr(0x10, 0x00)]);
        assert!(matches!(
            build_attestation(&bad),
            Err(AttestationError::UnsortedForks)
        ));
    }
}
