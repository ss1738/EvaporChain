//! Energy-Padded Authenticated Merkle Mountain Range (EPA-MMR).
//!
//! ## What an MMR is
//!
//! Merkle Mountain Range (Todd 2017; Mina, Polkadot, Beacon chain
//! all use a variant): an append-only log structure built as a
//! sequence of perfect binary trees of strictly decreasing
//! heights. Each tree's root is a "peak"; the MMR root commits to
//! the bagged sequence of all peaks. Append is amortised O(1);
//! inclusion proofs are O(log N).
//!
//! ## What "EPA" adds
//!
//! Energy-Padded Authenticated MMR carries an `energy` and
//! `minted_at_epoch` on each leaf. The verifier configurable
//! `energy_floor` rejects proofs whose leaf energy is below the
//! floor — the inclusion proof is *structurally invalid* for
//! decayed leaves. This is what makes the MMR decay-native:
//! evaporated leaves are unprovable, not just "not visible".
//!
//! Three structural decisions:
//!
//! 1. **Domain-separated leaf / inner / peak / bag hashes.**
//!    Standard Merkle hardening: `H_leaf(...) ≠ H_inner(...) ≠
//!    H_peak(...) ≠ H_bag(...)`. Prevents second-pre-image
//!    attacks where a leaf's canonical bytes might collide with
//!    an inner-node bit pattern.
//!
//! 2. **The leaf hash binds energy.** A verifier presented with
//!    `(value_hash, energy)` cannot pass off a different energy
//!    for the same leaf — the leaf hash includes both. This is
//!    why the energy-floor check is sound: if the prover gives
//!    you a leaf, they're committed to the energy too.
//!
//! 3. **Peaks-array representation, no per-node Vec.** State is a
//!    `Vec<[u8;32]>` of the current peaks (one per tree of size
//!    2^k for each set bit in `leaf_count`), plus the leaf count
//!    itself. Validators agree on the peaks ↔ leaf_count
//!    bijection.
//!
//! ## What this crate does NOT do
//!
//! - **Full sumcheck-folded inclusion proofs.** That's V2.
//!   Multilinear sumcheck (Lund-Fortnow-Karloff-Nisan 1992)
//!   gives sublinear-in-N verifier work; implementing it
//!   correctly is a months-of-effort polynomial-IOP project.
//!   V1 ships standard O(log N) hash paths.
//! - **Persistence / on-chain integration.** Pure data
//!   structure; the chain wraps it.
//! - **Energy update / decay tick.** This layer trusts the
//!   chain's decay model to call `update_energy(leaf_idx,
//!   new_energy)` before requesting a proof. The verifier's
//!   floor check is the static gate.
//!
//! ## Module map
//!
//! - [`leaf`] — [`EnergyLeaf`] + leaf-hash domain tag.
//! - [`mmr`] — [`EpaMmr`] peaks-array MMR + append + root.
//! - [`proof`] — [`InclusionProof`] + verify.

pub mod leaf;
pub mod mmr;
pub mod proof;

pub use leaf::{leaf_hash, EnergyLeaf, LEAF_TAG};
pub use mmr::{EpaMmr, MmrError};
pub use proof::{verify_inclusion, InclusionProof, ProofError, INNER_TAG, PEAK_BAG_TAG};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "EPA-MMR is an append-only log with O(log N)
    /// inclusion proofs that become STRUCTURALLY INVALID once a
    /// leaf's energy decays below a verifier-supplied floor.
    /// Decayed leaves are unprovable, not just hidden."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let mut mmr = EpaMmr::new();
        for i in 0u8..7 {
            mmr.append(EnergyLeaf::new([i; 32], 1_000, 0));
        }
        let root = mmr.root().unwrap();

        // Inclusion proof for leaf 3 with floor=500 verifies.
        let proof = InclusionProof::build(&mmr, 3).unwrap();
        verify_inclusion(&proof, &root, 500).unwrap();

        // After energy decays below the floor, the same structural
        // proof is rejected — the leaf is unprovable above floor=2000.
        verify_inclusion(&proof, &root, 2_000).unwrap_err();

        // Decay tick simulation: drop the leaf's energy below floor.
        // Re-built proof carries the new leaf, still rejected when
        // floor exceeds it.
        mmr.update_energy(3, 100).unwrap();
        let stale_root = mmr.root().unwrap();
        let new_proof = InclusionProof::build(&mmr, 3).unwrap();
        let res = verify_inclusion(&new_proof, &stale_root, 500);
        assert!(matches!(res, Err(ProofError::EnergyBelowFloor { .. })));
    }
}
