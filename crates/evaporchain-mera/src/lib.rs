//! Authenticated Energy-MERA — EvaporChain state commitment.
//!
//! # Theory
//!
//! Multi-scale Entanglement Renormalization Ansatz (Vidal 2007;
//! Evenbly-Vidal 2011). Applied here as a *state commitment scheme*:
//! instead of a flat Merkle tree, account energies are coarse-grained
//! through a tensor network whose filtration parameter is the chain's
//! single-λ decay constant.
//!
//! ```text
//!  Layer 0 (τ₀·2⁰):  [e₀][e₁][e₂][e₃] … [eₙ]   ← physical accounts
//!                       ╲W╱   ╲W╱               disentanglers (λ-parameterised)
//!                        U     U                 isometries (coarse-grain 2→1)
//!  Layer 1 (τ₀·2¹):    [e'₀] [e'₁] …
//!                         ╲W╱
//!                          U
//!  Layer 2 (τ₀·2²):       [e''₀] …
//!                           …
//!  Root:                   [root]               ← commitment
//! ```
//!
//! Key properties:
//! - Layer ℓ captures correlations at scale 2^ℓ accounts, with half-life τ₀·2^ℓ.
//! - Disentanglers W(λ, ℓ) remove short-range entanglement; parameterised by λ
//!   so the RG flow *is* the energy filtration.
//! - Root hash = `blake3(root_tensor_bytes)` — compact 32-byte commitment.
//! - Per-account proof = path of sibling tensors from leaf to root (O(log N)).
//!
//! # MERA gate — FAILED on real Ethereum data (2026-05-03)
//!
//! Doctrine §A1.8 / §A1.9 rule 12 / §A4.4 rule 23 gated this crate on
//! an empirical entropy measurement of real chain workloads. The
//! synthetic Pareto proxy passed (2026-04-29) but the real-data
//! re-run on 2026-05-03 — across three independent angles — failed:
//!
//! | Sample | Mode | Power-law R² | Flat ratio | Verdict |
//! |---|---|---|---|---|
//! | 1,000 blocks (real Ethereum 19_900_000-19_901_000) | binary | 0.7112 | 3.1× | VERKLE |
//! | 3,000 blocks (real Ethereum 19_900_000-19_903_000) | binary | 0.6913 | 3.1× | VERKLE |
//! | 3,000 blocks | **energy-weighted** (gas-summed, the test the doctrine actually implies — "Disentanglers = decay operator, energy filtration IS the MERA RG flow") | 0.6614 | **5.4×** | VERKLE |
//!
//! All three cross the doctrine's `R² ≥ 0.85` threshold from the
//! wrong side. The energy-weighted matrix is *more* flat than the
//! binary one, ruling out "the gate tested the wrong matrix" as a
//! possible escape clause. Real Ethereum activity does not have the
//! tensor-network structure MERA's RG flow is supposed to
//! coarse-grain.
//!
//! Per the doctrine's own contingency (§A1.8 — "If random: drop
//! tensor networks; ship Verkle + Energy-Verkle"): **MERA does not
//! ship.** The chain's authenticated state commitment is the
//! Energy-Verkle Trie (already in `evaporchain-state`).
//!
//! **This crate is retained as a research artefact only.** The
//! Givens-rotation disentanglers, isometries, end-to-end verifiable
//! proof round-trip, and χ=4 tensor algebra are correct mathematics
//! and may serve as a reference implementation for any future
//! tensor-network state-commitment effort that runs its own gate on
//! a workload with stronger log-correlation structure than Ethereum.
//! It is NOT wired into block production, finality, or light-client
//! verification on EvaporChain.
//!
//! See `research/mera-gate/GATE_RESULT.md` for the full numerical
//! report.
//!
//! # Bond dimension
//!
//! `CHI = 4` for this prototype. Increase to 8 or 16 post-mainnet once
//! the computational budget (validator hardware) is measured.

pub mod commitment;
pub mod gate;
pub mod layer;
pub mod proof;
pub mod synthetic;
pub mod tensor;
pub mod tree;

pub use commitment::{MeraCommitment, MeraCommitmentError};
pub use layer::MeraLayer;
pub use proof::{MeraProof, ProofVerifyError};
pub use tensor::{Tensor, CHI};
pub use tree::MeraTree;

/// Build a MERA commitment from a slice of account energy values.
///
/// `lambda_half_life`: the chain's λ expressed as epochs per halving.
/// `base_half_life`:   τ₀ — the half-life assigned to layer 0 (epochs).
///
/// Returns the commitment (compact root hash + layer hashes) and the full
/// tree (needed to generate per-account proofs).
pub fn commit(
    energies: &[u64],
    lambda_half_life: u64,
    base_half_life: u64,
) -> (MeraCommitment, MeraTree) {
    let tree = MeraTree::build(energies, lambda_half_life, base_half_life);
    let commitment = MeraCommitment::from_tree(&tree);
    (commitment, tree)
}

/// Verify a single account's energy against a pre-computed commitment.
pub fn verify_account(
    account_index: usize,
    energy: u64,
    proof: &MeraProof,
    commitment: &MeraCommitment,
) -> Result<(), ProofVerifyError> {
    proof.verify(account_index, energy, commitment)
}

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "MERA tensor-network state commitment is retained
    /// as a research artefact (gate FAILED on real Ethereum data
    /// 2026-05-03 — chain ships Energy-Verkle Trie instead). The
    /// commitment math is correct: per-account proofs round-trip
    /// against the commitment; tampering with the energy or the
    /// account index causes verification to reject."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let energies: Vec<u64> = (0..16u64).map(|i| 1_000 + i * 100).collect();
        let (commitment, tree) = commit(&energies, 100, 50);

        // Honest round-trip: every account's proof verifies against
        // its own (index, energy) under the commitment.
        for (i, &e) in energies.iter().enumerate() {
            let proof = MeraProof::generate(&tree, i);
            verify_account(i, e, &proof, &commitment).unwrap();
        }

        // Tampered energy is rejected.
        let i = 3;
        let proof = MeraProof::generate(&tree, i);
        let res = verify_account(i, energies[i] + 1, &proof, &commitment);
        assert!(res.is_err(), "tampered energy must reject");

        // Wrong account index using another account's proof: rejected.
        let proof_other = MeraProof::generate(&tree, 5);
        let res2 = verify_account(i, energies[i], &proof_other, &commitment);
        assert!(res2.is_err(), "swapped proof must reject");
    }
}
