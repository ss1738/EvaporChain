//! Genesis with LLSA-checked invariants — the **Birth** act of
//! EvaporChain's four-act narrative spine.
//!
//! Per `research/INVENTION_STACK.md` Amendment 2 §A2.5:
//!
//! > **Birth** | Genesis with LLSA-checked invariants | "The first
//! > blockchain whose constitution is a Coq-checked proof."
//!
//! ## What this wires together
//!
//! 1. **`evaporchain_llsa`** holds the proof artefact + a
//!    `ProofVerifier` trait. The chain's *constitution* is an
//!    `InvariantId` plus a Coq term proving every step preserves it.
//! 2. **`evaporchain_epv`** holds the protocol-version registry. The
//!    genesis version `v0` is the chain's first protocol identity;
//!    its energy decays under the chain-global λ, and the `v0`
//!    verifier module is pruned once the chain governance amends to
//!    `v1` and `v0`'s energy drops below `e_min`.
//!
//! Substrate function `genesis_with_invariant_proof` runs the LLSA
//! verifier against a *genesis amendment* (from `None` → `v0`) and,
//! on success, registers `v0` on a fresh `EpvRegistry`. The
//! constitution is thus a Coq term *before* the first block exists.
//!
//! Pairs with `evaporchain-tombstone`, `evaporchain-mortis`,
//! `evaporchain-sentinel` — the four acts together (Birth → Life
//! → Small deaths → Final death) form the chain's narrative spine.

use thiserror::Error;

use evaporchain_epv::{EpvRegistry, ProtocolVersion};
use evaporchain_llsa::{
    proof::{InvariantId, ProofVerifier},
    LlsaProof,
};
use evaporchain_types::Energy;

/// 32-byte commitment proving the genesis amendment's binding.
/// Built deterministically from the (genesis_version_id, descriptor)
/// pair so every node in the network derives the same hash.
fn genesis_amendment_hash(genesis_version_id: u32, genesis_descriptor: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"evaporchain-genesis-amendment");
    h.update(&genesis_version_id.to_le_bytes());
    h.update(&(genesis_descriptor.len() as u64).to_le_bytes());
    h.update(genesis_descriptor);
    *h.finalize().as_bytes()
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GenesisError {
    #[error(
        "constitution proof rejected: {0}"
    )]
    ProofRejected(String),
    #[error("genesis registry already contains protocol version {0}")]
    AlreadyRegistered(u32),
}

/// Run the LLSA verifier against the supplied constitution proof.
/// On success, register protocol version `v0` on a fresh
/// `EpvRegistry` with `seed_energy` and return it.
///
/// `proof` must bind `expected_invariant_id` and an
/// `bound_amendment_hash` equal to
/// `blake3('evaporchain-genesis-amendment' || v0_id || len ||
///  descriptor)`.
pub fn genesis_with_invariant_proof<V: ProofVerifier>(
    expected_invariant_id: InvariantId,
    genesis_version_id: u32,
    genesis_descriptor: &[u8],
    seed_energy: Energy,
    activation_epoch: u64,
    proof: &LlsaProof,
    verifier: &V,
) -> Result<EpvRegistry, GenesisError> {
    let expected_amendment_hash = genesis_amendment_hash(genesis_version_id, genesis_descriptor);
    verifier
        .verify(proof, expected_invariant_id, expected_amendment_hash)
        .map_err(|e| GenesisError::ProofRejected(format!("{e}")))?;
    let mut registry = EpvRegistry::new();
    registry
        .register(ProtocolVersion::new(
            genesis_version_id,
            seed_energy,
            activation_epoch,
        ))
        .map_err(|_| GenesisError::AlreadyRegistered(genesis_version_id))?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_llsa::proof::{AlwaysAcceptVerifier, AlwaysRejectVerifier};

    fn proof_for(inv: InvariantId, amendment: [u8; 32]) -> LlsaProof {
        LlsaProof {
            coq_term_hash: [9u8; 32],
            target_invariant_id: inv,
            bound_amendment_hash: amendment,
            proof_bytes: vec![1, 2, 3],
        }
    }

    #[test]
    fn happy_path_registers_v0() {
        let inv = [42u8; 32];
        let v0_id = 0u32;
        let descriptor = b"genesis-state-root-v0";
        let expected_hash = genesis_amendment_hash(v0_id, descriptor);
        let proof = proof_for(inv, expected_hash);
        let registry = genesis_with_invariant_proof(
            inv,
            v0_id,
            descriptor,
            1_000_000,
            0,
            &proof,
            &AlwaysAcceptVerifier,
        )
        .unwrap();
        assert!(registry.contains(v0_id));
        let v0 = registry.get(v0_id).unwrap();
        assert_eq!(v0.seed_energy, 1_000_000);
        assert_eq!(v0.activated_epoch, 0);
    }

    #[test]
    fn verifier_rejection_blocks_genesis() {
        let inv = [42u8; 32];
        let descriptor = b"v0";
        let proof = proof_for(inv, genesis_amendment_hash(0, descriptor));
        let err = genesis_with_invariant_proof(
            inv,
            0,
            descriptor,
            1_000_000,
            0,
            &proof,
            &AlwaysRejectVerifier,
        )
        .unwrap_err();
        assert!(matches!(err, GenesisError::ProofRejected(_)));
    }

    #[test]
    fn wrong_invariant_rejected() {
        let inv = [42u8; 32];
        let descriptor = b"v0";
        // Proof binds DIFFERENT invariant id.
        let proof = proof_for([99u8; 32], genesis_amendment_hash(0, descriptor));
        let err = genesis_with_invariant_proof(
            inv,
            0,
            descriptor,
            1_000_000,
            0,
            &proof,
            &AlwaysAcceptVerifier,
        )
        .unwrap_err();
        assert!(matches!(err, GenesisError::ProofRejected(_)));
    }

    #[test]
    fn descriptor_tamper_breaks_amendment_binding() {
        let inv = [42u8; 32];
        // Bind proof to descriptor "v0" but pass "v1" at verification.
        let proof = proof_for(inv, genesis_amendment_hash(0, b"v0"));
        let err = genesis_with_invariant_proof(
            inv,
            0,
            b"v1", // wrong descriptor
            1_000_000,
            0,
            &proof,
            &AlwaysAcceptVerifier,
        )
        .unwrap_err();
        assert!(matches!(err, GenesisError::ProofRejected(_)));
    }

    #[test]
    fn deterministic_amendment_hash() {
        // Two nodes computing the genesis hash from the same inputs
        // arrive at the same bytes — chain-side determinism check.
        let h1 = genesis_amendment_hash(0, b"genesis-state-root-v0");
        let h2 = genesis_amendment_hash(0, b"genesis-state-root-v0");
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_descriptors_distinct_hashes() {
        let h1 = genesis_amendment_hash(0, b"v0-mainnet");
        let h2 = genesis_amendment_hash(0, b"v0-testnet");
        assert_ne!(h1, h2);
    }

    #[test]
    fn different_version_ids_distinct_hashes() {
        let h1 = genesis_amendment_hash(0, b"x");
        let h2 = genesis_amendment_hash(1, b"x");
        assert_ne!(h1, h2);
    }
}
