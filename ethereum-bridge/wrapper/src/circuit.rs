//! Groth16 wrapper circuit skeleton (sub-B starter).
//!
//! # In-circuit Halo2 IPA verifier — TODO (sub-B-finish, multi-week)
//!
//! The full circuit body would, in pseudocode:
//!
//! ```text
//! 1. Witness:  halo2_ipa_proof_bytes (3.8 KB), Pallas curve points,
//!              IPA challenge scalars, scalar-multiplication ladder bits.
//! 2. Allocate non-native arithmetic gadgets for Pallas Fq inside BN254 Fr.
//!    Pallas Fq is 254-bit; BN254 Fr is 254-bit; representable as a single
//!    Fr element per Pallas Fq element with ~5 Fr-multiplications per
//!    Fq-multiplication (Schoolbook), or ~3 with CRT decomposition.
//! 3. Replay the Halo2 IPA verifier algorithm constraint-by-constraint:
//!      a. Domain-separated transcript via Fiat-Shamir (Poseidon over Fr).
//!      b. Reconstruct verifier challenges round-by-round.
//!      c. Final inner-product check: <a, b> = c via accumulator equation.
//! 4. Bind the 4 public inputs (state_root, key, value_commitment,
//!    params_fingerprint) into the transcript so a forged proof for a
//!    different commitment cannot reuse the same Groth16 witness.
//! ```
//!
//! Constraint count estimate: ~80k-200k Groth16 constraints, dominated
//! by ~10 rounds of IPA × ~3 non-native Fq operations × ~3k constraints
//! each. Trusted-setup phase 2 (Powers-of-Tau ≥ 2^18) suffices for k≤17
//! circuits — comfortable headroom over the estimated 80k-200k.
//!
//! # What this starter ships
//!
//! A **placeholder binding constraint**: `state_root * 1 = state_root`
//! (tautology, but exercises the public-input wiring). The
//! `halo2_ipa_proof_bytes` from the fixture is accepted but ignored.
//!
//! This is correct as a starter because:
//!
//! 1. It lets Groth16 setup/prove/verify run end-to-end against the
//!    real BN254 backend, so the surrounding pipeline (CLI tool, fixture
//!    load, prove-verify timing, Solidity calldata shape) can be built
//!    and tested before the heavy in-circuit work.
//! 2. The interface is stable: `WrapperPublicInputs` order, the
//!    256-byte Groth16 proof encoding, and the trusted-setup parameter
//!    domain don't change between starter and finish — only the
//!    constraint body grows.
//! 3. Anyone deploying the starter VK gets a verifier that accepts ANY
//!    well-formed proof, which is loudly wrong (the L1
//!    `VerkleProofVerifier` currently reverts with `Groth16VKNotWired`
//!    so production deployment is impossible). Sub-B-finish replaces
//!    the placeholder constraint with the real verifier.

use crate::inputs::WrapperPublicInputs;
use ark_bn254::Fr;
use ark_r1cs_std::alloc::AllocVar;
use ark_r1cs_std::eq::EqGadget;
use ark_r1cs_std::fields::fp::FpVar;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

/// The starter circuit. Holds the 4 public-input anchors + the raw
/// Halo2 IPA proof bytes (the latter currently ignored until sub-B-finish).
#[derive(Clone, Debug)]
pub struct WrapperCircuit {
    pub public_inputs: WrapperPublicInputs,
    /// Raw Halo2 IPA proof from `VerkleProverV2`. Ignored by the
    /// starter; sub-B-finish allocates witness vars for each
    /// transcript chunk and runs the in-circuit IPA verifier.
    pub halo2_ipa_proof_bytes: Vec<u8>,
}

impl WrapperCircuit {
    /// Constructor with explicit anchors + proof bytes.
    pub fn new(public_inputs: WrapperPublicInputs, halo2_ipa_proof_bytes: Vec<u8>) -> Self {
        Self {
            public_inputs,
            halo2_ipa_proof_bytes,
        }
    }

    /// Dummy instance for Groth16 setup. The trusted-setup ceremony
    /// only depends on the circuit's *shape* (number of constraints +
    /// public-input arity), not on the witness values. Sub-B-finish
    /// will need a dummy whose constraint count matches the real
    /// in-circuit IPA verifier — at that point this changes.
    pub fn dummy() -> Self {
        Self {
            public_inputs: WrapperPublicInputs {
                state_root: Fr::from(1u64),
                key: Fr::from(2u64),
                value_commitment: Fr::from(3u64),
                params_fingerprint: Fr::from(4u64),
            },
            halo2_ipa_proof_bytes: Vec::new(),
        }
    }
}

impl ConstraintSynthesizer<Fr> for WrapperCircuit {
    fn generate_constraints(self, cs: ConstraintSystemRef<Fr>) -> Result<(), SynthesisError> {
        // ── Public inputs: 4 BN254 Fr anchors, in canonical order ───
        let state_root_var =
            FpVar::<Fr>::new_input(cs.clone(), || Ok(self.public_inputs.state_root))?;
        let key_var = FpVar::<Fr>::new_input(cs.clone(), || Ok(self.public_inputs.key))?;
        let value_commitment_var =
            FpVar::<Fr>::new_input(cs.clone(), || Ok(self.public_inputs.value_commitment))?;
        let params_fingerprint_var =
            FpVar::<Fr>::new_input(cs.clone(), || Ok(self.public_inputs.params_fingerprint))?;

        // ── Placeholder binding constraint (sub-B starter only) ────
        //
        // `state_root_var == state_root_var` — a tautology that
        // exercises the public-input wiring + EqGadget without
        // tightening anything. Sub-B-finish replaces this with the
        // in-circuit Halo2 IPA verifier whose acceptance set is bound
        // to (state_root, key, value_commitment, params_fingerprint)
        // via Fiat-Shamir transcript inclusion.
        state_root_var.enforce_equal(&state_root_var)?;

        // Silence "unused variable" warnings on the other anchors —
        // they're allocated but not constrained yet. The order of
        // `new_input` calls IS the IC[] index order Groth16 emits, so
        // these allocations are load-bearing even though no constraint
        // touches them. (Without these refs the optimizer might
        // hypothetically elide them — defensive ref.)
        let _ = &key_var;
        let _ = &value_commitment_var;
        let _ = &params_fingerprint_var;

        // halo2_ipa_proof_bytes intentionally ignored at starter scope.
        let _ = &self.halo2_ipa_proof_bytes;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_relations::r1cs::ConstraintSystem;

    /// The starter circuit must be satisfiable for any well-formed
    /// public inputs. This pins the placeholder-constraint contract:
    /// sub-B-starter accepts everything; sub-B-finish accepts only
    /// well-formed Halo2 IPA proofs.
    #[test]
    fn starter_circuit_is_satisfiable_for_arbitrary_inputs() {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let circuit = WrapperCircuit::new(
            WrapperPublicInputs {
                state_root: Fr::from(42u64),
                key: Fr::from(99u64),
                value_commitment: Fr::from(7u64),
                params_fingerprint: Fr::from(2026u64),
            },
            vec![0xde, 0xad, 0xbe, 0xef],
        );
        circuit.generate_constraints(cs.clone()).expect("synthesize");
        assert!(cs.is_satisfied().expect("is_satisfied"));
    }

    /// Public-input arity must be exactly 4. The trusted-setup
    /// ceremony bakes the IC[] table for arity-4; changing it later
    /// invalidates the VK and forces a re-ceremony.
    #[test]
    fn public_input_arity_is_four() {
        let cs = ConstraintSystem::<Fr>::new_ref();
        let circuit = WrapperCircuit::dummy();
        circuit.generate_constraints(cs.clone()).expect("synthesize");
        assert_eq!(
            cs.num_instance_variables(),
            5,
            "must be 4 public inputs + 1 for the constant (Groth16 convention)"
        );
    }

    /// Dummy must produce the same constraint shape as a real-input
    /// circuit — required for trusted-setup correctness (the ceremony
    /// runs against `dummy()`, then the prover runs against real
    /// inputs against the same `pk`).
    #[test]
    fn dummy_and_real_have_matching_constraint_counts() {
        let cs_dummy = ConstraintSystem::<Fr>::new_ref();
        WrapperCircuit::dummy()
            .generate_constraints(cs_dummy.clone())
            .expect("dummy synthesize");

        let cs_real = ConstraintSystem::<Fr>::new_ref();
        WrapperCircuit::new(
            WrapperPublicInputs {
                state_root: Fr::from(100u64),
                key: Fr::from(200u64),
                value_commitment: Fr::from(300u64),
                params_fingerprint: Fr::from(400u64),
            },
            vec![0; 3872], // matches real Halo2 IPA proof byte length
        )
        .generate_constraints(cs_real.clone())
        .expect("real synthesize");

        assert_eq!(
            cs_dummy.num_constraints(),
            cs_real.num_constraints(),
            "trusted-setup invariant: dummy and real must have matching constraint counts"
        );
        assert_eq!(
            cs_dummy.num_instance_variables(),
            cs_real.num_instance_variables(),
            "trusted-setup invariant: dummy and real must have matching public-input arity"
        );
    }
}
