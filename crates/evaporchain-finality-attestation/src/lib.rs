//! Finality Attestation — one root, three V2 commitments folded in.
//!
//! V2 hardening of the locked invention stack produced three
//! independent commitments per finalised block:
//!
//! - **Light-Cone V2** — `causal_root`, a Merkle root over the
//!   block's ancestor set.
//! - **Bell-Beacon V2** — `BellCertificate.seed`, a 32-byte
//!   anti-grinding randomness anchor.
//! - **Evap-Fork-Cert V2** — a list of `EvaporatedForkCertV2`
//!   (witness bytes), each proving an alternative fork has decayed
//!   below threshold.
//!
//! This crate folds those three into a single canonical 32-byte
//! attestation root. A light client that sees `(block_hash,
//! FinalityAttestation, attestation_root)` can verify finality
//! without holding the DAG, the beacon archive, or any fork blocks:
//!
//! ```text
//!   attestation_root = BLAKE3(domain
//!                            || block_hash
//!                            || finalised_at_epoch
//!                            || causal_root
//!                            || bell_seed
//!                            || merkle_root_of_evaporated_forks)
//! ```
//!
//! Tamper any field → root diverges → light client rejects.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT re-prove the three V2 sub-commitments. Those are
//!   produced upstream by the V2 crates; this crate just folds them.
//! - It does NOT model committee signatures over the attestation
//!   (consensus-layer concern).

pub mod attest;

pub use attest::{
    build_attestation, verify_attestation, AttestationError, EvaporatedForkWitnessRef,
    FinalityAttestation,
};

#[cfg(test)]
mod press_claim_tests {
    //! The press claim lives as a test.
    //!
    //! Source: INVENTION_STACK §4.1 row 1 (Light-Cone Consensus →
    //! `causal_root`), §4.1 row 10 (Evaporated-Fork Certificates →
    //! fork-witness list), §4.2 Bell-Certified Beacon (→ `bell_seed`).
    //!
    //! Claim: "Finality Attestation folds the three V2 commitments
    //! — Light-Cone V2 `causal_root`, Bell-Beacon V2 `bell_seed`, and
    //! Evap-Fork-Cert V2 fork-witness list — into a single canonical
    //! 32-byte attestation root.  A light client can verify finality
    //! of a block from `(block_hash, FinalityAttestation,
    //! attestation_root)` without holding the DAG, the beacon archive,
    //! or any fork blocks.  Tamper any field → root diverges →
    //! verification rejected."
    //!
    //! Three invariants that MUST hold at the runtime level:
    //!
    //! 1. **Completeness** — a well-formed attestation round-trips
    //!    through build → verify without error.
    //! 2. **Soundness (tamper-detection)** — mutating any one field of
    //!    a `FinalityAttestation` produces a divergent root; supplying
    //!    the original root to `verify_attestation` returns
    //!    `RootMismatch`.
    //! 3. **Canonicality** — an unsorted or duplicated fork-witness
    //!    list is rejected at build time before a root is produced.

    use crate::{
        build_attestation, verify_attestation, AttestationError, EvaporatedForkWitnessRef,
        FinalityAttestation,
    };

    fn att(
        block_byte: u8,
        epoch: u64,
        causal_byte: u8,
        bell_byte: u8,
        forks: Vec<EvaporatedForkWitnessRef>,
    ) -> FinalityAttestation {
        let mut block_hash = [0u8; 32];
        block_hash[0] = block_byte;
        let mut causal_root = [0u8; 32];
        causal_root[0] = causal_byte;
        let mut bell_seed = [0u8; 32];
        bell_seed[0] = bell_byte;
        FinalityAttestation {
            block_hash,
            finalised_at_epoch: epoch,
            causal_root,
            bell_seed,
            evaporated_forks: forks,
        }
    }

    fn fwr(root_byte: u8, witness_byte: u8) -> EvaporatedForkWitnessRef {
        let mut fork_root = [0u8; 32];
        fork_root[0] = root_byte;
        let mut witness = [0u8; 32];
        witness[0] = witness_byte;
        EvaporatedForkWitnessRef { fork_root, witness }
    }

    // ── 1. Completeness ──────────────────────────────────────────────

    #[test]
    fn well_formed_attestation_round_trips() {
        let a = att(
            0x01,
            100,
            0xCA,
            0xBE,
            vec![fwr(0x10, 0xAA), fwr(0x20, 0xBB)],
        );
        let root = build_attestation(&a).unwrap();
        verify_attestation(&a, &root).unwrap();
    }

    // ── 2. Soundness — six tamper vectors all rejected ───────────────

    #[test]
    fn tamper_any_field_invalidates_root() {
        let a = att(
            0x01,
            100,
            0xCA,
            0xBE,
            vec![fwr(0x10, 0xAA), fwr(0x20, 0xBB)],
        );
        let root = build_attestation(&a).unwrap();

        for tampered in [
            {
                let mut t = a.clone();
                t.block_hash[0] ^= 1;
                t
            },
            {
                let mut t = a.clone();
                t.finalised_at_epoch += 1;
                t
            },
            {
                let mut t = a.clone();
                t.causal_root[0] ^= 1;
                t
            },
            {
                let mut t = a.clone();
                t.bell_seed[0] ^= 1;
                t
            },
            {
                let mut t = a.clone();
                t.evaporated_forks[0].witness[0] ^= 1;
                t
            },
            {
                let mut t = a.clone();
                t.evaporated_forks.pop();
                t
            },
        ] {
            let err = verify_attestation(&tampered, &root).unwrap_err();
            assert!(
                matches!(err, AttestationError::RootMismatch { .. }),
                "expected RootMismatch, got {err:?}",
            );
        }
    }

    // ── 3. Canonicality ──────────────────────────────────────────────

    #[test]
    fn unsorted_fork_list_rejected_before_root_produced() {
        let a = att(0x01, 50, 0x00, 0x00, vec![fwr(0x20, 0x00), fwr(0x10, 0x00)]);
        assert_eq!(
            build_attestation(&a).unwrap_err(),
            AttestationError::UnsortedForks
        );
    }

    #[test]
    fn duplicate_fork_root_rejected_before_root_produced() {
        let a = att(0x01, 50, 0x00, 0x00, vec![fwr(0x10, 0xAA), fwr(0x10, 0xBB)]);
        assert!(matches!(
            build_attestation(&a).unwrap_err(),
            AttestationError::DuplicateForkRoot(_)
        ));
    }
}
