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
//! # MERA gate
//!
//! This crate was gated on an empirical entropy measurement (§A1.8).
//! Gate result: **PASS — MERA GO** (2026-04-29).
//! See `research/mera-gate/GATE_RESULT.md`.
//!
//! # Bond dimension
//!
//! `CHI = 4` for this prototype. Increase to 8 or 16 post-mainnet once
//! the computational budget (validator hardware) is measured.

pub mod commitment;
pub mod layer;
pub mod proof;
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
