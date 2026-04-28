//! `apply_amendment` — the chain-side gate that accepts a proven
//! amendment and registers the new protocol version on the EPV
//! registry.
//!
//! Three checks before any state mutation:
//!
//!   1. `from_version` is currently registered.
//!   2. `to_version` is NOT yet registered (no collisions).
//!   3. The supplied `ProofVerifier` accepts the proof against the
//!      expected invariant id and the amendment's canonical hash.
//!
//! Only after all three pass do we register the new version on the
//! EPV registry. The new version is seeded with whatever
//! `to_version_seed_energy` the caller supplies (chain-set policy).

use evaporchain_epv::{EpvRegistry, ProtocolVersion};
use evaporchain_types::Energy;

use crate::amendment::{Amendment, AmendmentError};
use crate::proof::{InvariantId, ProofVerifier};

pub fn apply_amendment<V: ProofVerifier>(
    registry: &mut EpvRegistry,
    amendment: &Amendment,
    expected_invariant: InvariantId,
    to_version_seed_energy: Energy,
    activation_epoch: u64,
    verifier: &V,
) -> Result<(), AmendmentError> {
    // 1. From-version must exist.
    if !registry.contains(amendment.from_version) {
        return Err(AmendmentError::FromVersionAbsent(amendment.from_version));
    }
    // 2. To-version must NOT exist.
    if registry.contains(amendment.to_version) {
        return Err(AmendmentError::ToVersionExists(amendment.to_version));
    }
    // 3. Proof must verify against the canonical amendment hash.
    let expected_hash = amendment.hash();
    verifier.verify(&amendment.proof, expected_invariant, expected_hash)?;
    // All three pass — register.
    registry
        .register(ProtocolVersion::new(
            amendment.to_version,
            to_version_seed_energy,
            activation_epoch,
        ))
        .map_err(|_| AmendmentError::ToVersionExists(amendment.to_version))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proof::{AlwaysAcceptVerifier, AlwaysRejectVerifier, LlsaProof};
    use evaporchain_epv::ProtocolVersion;

    fn fresh_registry() -> EpvRegistry {
        let mut r = EpvRegistry::new();
        r.register(ProtocolVersion::new(1, 1_000_000, 0)).unwrap();
        r
    }

    fn well_formed_amendment(invariant: InvariantId) -> Amendment {
        let a = Amendment {
            from_version: 1,
            to_version: 2,
            step_new_descriptor: b"v2-impl".to_vec(),
            proof: LlsaProof {
                coq_term_hash: [9u8; 32],
                target_invariant_id: invariant,
                bound_amendment_hash: [0u8; 32], // patched below
                proof_bytes: vec![1, 2, 3],
            },
        };
        let h = a.hash();
        let mut a = a;
        a.proof.bound_amendment_hash = h;
        a
    }

    #[test]
    fn happy_path_registers_to_version() {
        let mut r = fresh_registry();
        let inv = [42u8; 32];
        let a = well_formed_amendment(inv);
        apply_amendment(&mut r, &a, inv, 1_000_000, 100, &AlwaysAcceptVerifier).unwrap();
        assert!(r.contains(2));
    }

    #[test]
    fn missing_from_version_rejected() {
        let mut r = EpvRegistry::new();
        let inv = [42u8; 32];
        let a = well_formed_amendment(inv);
        let err = apply_amendment(
            &mut r,
            &a,
            inv,
            1_000_000,
            100,
            &AlwaysAcceptVerifier,
        )
        .unwrap_err();
        assert_eq!(err, AmendmentError::FromVersionAbsent(1));
    }

    #[test]
    fn collision_with_existing_to_version_rejected() {
        let mut r = fresh_registry();
        r.register(ProtocolVersion::new(2, 100, 50)).unwrap();
        let inv = [42u8; 32];
        let a = well_formed_amendment(inv);
        let err = apply_amendment(
            &mut r,
            &a,
            inv,
            1_000_000,
            100,
            &AlwaysAcceptVerifier,
        )
        .unwrap_err();
        assert_eq!(err, AmendmentError::ToVersionExists(2));
    }

    #[test]
    fn verifier_rejection_blocks_registration() {
        let mut r = fresh_registry();
        let inv = [42u8; 32];
        let a = well_formed_amendment(inv);
        let err = apply_amendment(
            &mut r,
            &a,
            inv,
            1_000_000,
            100,
            &AlwaysRejectVerifier,
        )
        .unwrap_err();
        assert!(matches!(err, AmendmentError::Proof(_)));
        // To-version is NOT registered after rejection.
        assert!(!r.contains(2));
    }

    #[test]
    fn proof_bound_to_wrong_amendment_rejected() {
        let mut r = fresh_registry();
        let inv = [42u8; 32];
        let mut a = well_formed_amendment(inv);
        // Tamper with the amendment AFTER proof was bound — proof's
        // hash no longer matches.
        a.step_new_descriptor = b"different-impl".to_vec();
        let err = apply_amendment(
            &mut r,
            &a,
            inv,
            1_000_000,
            100,
            &AlwaysAcceptVerifier,
        )
        .unwrap_err();
        match err {
            AmendmentError::Proof(crate::proof::ProofError::WrongAmendment { .. }) => {}
            other => panic!("wrong error: {other:?}"),
        }
    }
}
