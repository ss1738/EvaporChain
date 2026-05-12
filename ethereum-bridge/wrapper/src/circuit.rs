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
use crate::pallas_g1::{enforce_g1_add, NonNativePallasPoint};
use ark_bn254::Fr;
use ark_ec::{AffineRepr, CurveGroup, PrimeGroup};
use ark_pallas::{Affine as PallasAffine, Projective as PallasProjective};
use ark_r1cs_std::alloc::AllocVar;
use ark_r1cs_std::eq::EqGadget;
use ark_r1cs_std::fields::fp::FpVar;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

/// A `(P1, P2, P3 = P1 + P2)` Pallas G1 triple used as witness for
/// the in-circuit g1_add gadget. T0.10 sub-B-finish integration step:
/// the wrapper circuit now actually invokes `enforce_g1_add` on a
/// real Pallas triple, proving the gadget composes with Groth16
/// setup/prove/verify end-to-end.
///
/// In the full sub-B-finish the wrapper will issue ~k generic G1
/// adds (one per IPA challenge round). This single-triple variant
/// proves the composition shape works and pins the per-add cost.
#[derive(Clone, Debug)]
pub struct G1AddWitness {
    pub p1: PallasAffine,
    pub p2: PallasAffine,
    pub p3: PallasAffine,
}

impl G1AddWitness {
    /// Dummy triple for trusted-setup shape baseline: `(G, 2G, 3G)`
    /// on Pallas. Generator + 2× generator + 3× generator — distinct
    /// x-coords (so the affine-add precondition `x₁ ≠ x₂` holds),
    /// and `3G = G + 2G` by the group law. The triple is hard-coded
    /// so the trusted-setup ceremony always sees the same constraint
    /// shape regardless of caller.
    pub fn dummy() -> Self {
        let g = PallasProjective::generator();
        let p1_proj = g;
        let p2_proj = g + g;
        let p3_proj = p1_proj + p2_proj;
        Self {
            p1: p1_proj.into_affine(),
            p2: p2_proj.into_affine(),
            p3: p3_proj.into_affine(),
        }
    }

    /// Off-circuit sanity: returns `true` iff `p3 = p1 + p2` and the
    /// affine-add precondition (`p1.x ≠ p2.x` after rejecting identity)
    /// holds. Callers wiring real Halo2 IPA witnesses should call this
    /// before constructing the circuit — a failing precondition makes
    /// `enforce_g1_add` return `SynthesisError::Unsatisfiable`.
    pub fn precondition_holds(&self) -> bool {
        if self.p1.is_zero() || self.p2.is_zero() || self.p3.is_zero() {
            return false;
        }
        let (x1, _) = match self.p1.xy() {
            Some(xy) => xy,
            None => return false,
        };
        let (x2, _) = match self.p2.xy() {
            Some(xy) => xy,
            None => return false,
        };
        if x1 == x2 {
            return false;
        }
        let computed_p3 = (PallasProjective::from(self.p1) + PallasProjective::from(self.p2))
            .into_affine();
        computed_p3 == self.p3
    }
}

/// The starter circuit. Holds the 4 public-input anchors + the raw
/// Halo2 IPA proof bytes + one g1_add witness triple. Sub-B-finish
/// will issue many such triples (one per IPA round); this single-add
/// integration pins the gadget composition with Groth16.
#[derive(Clone, Debug)]
pub struct WrapperCircuit {
    pub public_inputs: WrapperPublicInputs,
    /// Raw Halo2 IPA proof from `VerkleProverV2`. Ignored by the
    /// starter; sub-B-finish allocates witness vars for each
    /// transcript chunk and runs the in-circuit IPA verifier.
    pub halo2_ipa_proof_bytes: Vec<u8>,
    /// One `(P1, P2, P3)` Pallas G1 triple. The circuit enforces
    /// `P3 = P1 + P2` in-circuit via `enforce_g1_add`. Default is
    /// `G1AddWitness::dummy()` so existing callers (pre-integration)
    /// still produce satisfying proofs without behaviour change.
    pub g1_add_witness: G1AddWitness,
}

impl WrapperCircuit {
    /// Constructor with explicit anchors + proof bytes. The g1_add
    /// witness defaults to `G1AddWitness::dummy()`; override via
    /// `with_g1_add` to feed a real Halo2-IPA-round triple.
    pub fn new(public_inputs: WrapperPublicInputs, halo2_ipa_proof_bytes: Vec<u8>) -> Self {
        Self {
            public_inputs,
            halo2_ipa_proof_bytes,
            g1_add_witness: G1AddWitness::dummy(),
        }
    }

    /// Builder-style g1_add witness override.
    pub fn with_g1_add(mut self, g1_add_witness: G1AddWitness) -> Self {
        self.g1_add_witness = g1_add_witness;
        self
    }

    /// Dummy instance for Groth16 setup. The trusted-setup ceremony
    /// only depends on the circuit's *shape* (number of constraints +
    /// public-input arity), not on the witness values. The g1_add
    /// dummy triple is fixed so the ceremony's constraint shape is
    /// stable across operator machines.
    pub fn dummy() -> Self {
        Self {
            public_inputs: WrapperPublicInputs {
                state_root: Fr::from(1u64),
                key: Fr::from(2u64),
                value_commitment: Fr::from(3u64),
                params_fingerprint: Fr::from(4u64),
            },
            halo2_ipa_proof_bytes: Vec::new(),
            g1_add_witness: G1AddWitness::dummy(),
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

        // ── Placeholder anchor tautology (sub-B starter remnant) ───
        //
        // `state_root_var == state_root_var` — a tautology that
        // exercises the public-input wiring + EqGadget. Sub-B-finish
        // replaces this with the in-circuit Halo2 IPA verifier whose
        // acceptance set is bound to (state_root, key, value_commitment,
        // params_fingerprint) via Fiat-Shamir transcript inclusion.
        state_root_var.enforce_equal(&state_root_var)?;

        // ── g1_add integration (T0.10 sub-B-finish, 2026-05-12) ────
        //
        // Allocate the `(P1, P2, P3)` Pallas triple as non-native
        // witnesses and enforce `P3 = P1 + P2`. This is the first real
        // arithmetic constraint in the wrapper — sub-B-finish will
        // issue many such triples, one per IPA challenge round.
        //
        // Marginal cost: ~4000 R1CS constraints per add (per the
        // g1_add_constraint_count_in_expected_range test in
        // pallas_g1.rs). Well within the 2^18 Powers-of-Tau budget
        // even at the sub-B-finish ~80-200k constraint target.
        let p1_var = NonNativePallasPoint::alloc_witness(cs.clone(), self.g1_add_witness.p1)?;
        let p2_var = NonNativePallasPoint::alloc_witness(cs.clone(), self.g1_add_witness.p2)?;
        let p3_var = NonNativePallasPoint::alloc_witness(cs.clone(), self.g1_add_witness.p3)?;
        enforce_g1_add(cs.clone(), &p1_var, &p2_var, &p3_var)?;

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
    use ark_std::rand::SeedableRng;
    use ark_std::UniformRand;

    fn seeded_rng() -> ark_std::rand::rngs::StdRng {
        ark_std::rand::rngs::StdRng::seed_from_u64(0xC0FFEE_u64)
    }

    /// Build a valid `(P1, P2, P3 = P1+P2)` Pallas triple from a
    /// deterministic RNG. Used by the integration tests below.
    fn random_valid_g1_triple(rng: &mut impl ark_std::rand::Rng) -> G1AddWitness {
        loop {
            let p1_proj = PallasProjective::rand(rng);
            let p2_proj = PallasProjective::rand(rng);
            let p3_proj = p1_proj + p2_proj;
            let p1 = p1_proj.into_affine();
            let p2 = p2_proj.into_affine();
            let p3 = p3_proj.into_affine();
            if p1.is_zero() || p2.is_zero() || p3.is_zero() {
                continue;
            }
            let (x1, _) = p1.xy().expect("p1 xy");
            let (x2, _) = p2.xy().expect("p2 xy");
            if x1 == x2 {
                continue;
            }
            let w = G1AddWitness { p1, p2, p3 };
            assert!(w.precondition_holds());
            return w;
        }
    }

    /// The starter circuit (with default `G1AddWitness::dummy()`) must
    /// be satisfiable for any well-formed public inputs. Pins the
    /// shape contract for trusted setup: ceremony runs against
    /// `dummy()`, prover runs against real witness, both produce
    /// matching constraint counts.
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

    /// T0.10 sub-B-finish integration test (HEADLINE).
    /// A wrapper circuit constructed with a fresh random valid
    /// `(P1, P2, P3 = P1+P2)` Pallas triple must produce a satisfying
    /// constraint system. This proves the g1_add gadget composes
    /// with the wrapper public-input wiring + the existing tautology
    /// constraint — and is the integration counterpart of the
    /// unit-level `g1_add_satisfied_for_valid_triple` test.
    #[test]
    fn wrapper_with_real_g1_triple_is_satisfied() {
        let mut rng = seeded_rng();
        let cs = ConstraintSystem::<Fr>::new_ref();
        let circuit = WrapperCircuit::new(
            WrapperPublicInputs {
                state_root: Fr::from(0x0900u64),
                key: Fr::from(0x2b00u64),
                value_commitment: Fr::from(0x2200u64),
                params_fingerprint: Fr::from(0x2e00u64),
            },
            vec![0xc0, 0xff, 0xee],
        )
        .with_g1_add(random_valid_g1_triple(&mut rng));
        circuit.generate_constraints(cs.clone()).expect("synthesize");
        assert!(cs.is_satisfied().expect("is_satisfied"));
    }

    /// Soundness — a wrapper circuit constructed with a TAMPERED
    /// `P3 ≠ P1 + P2` triple must produce an UNSATISFIED constraint
    /// system. Without this gate the wrapper would accept arbitrary
    /// triples and the wrapped proof would be vacuous.
    #[test]
    fn wrapper_with_wrong_g1_triple_is_unsatisfied() {
        let mut rng = seeded_rng();
        let valid = random_valid_g1_triple(&mut rng);
        // Replace P3 with an independent random point — vanishingly
        // unlikely to coincide with the true sum.
        let wrong_p3 = PallasProjective::rand(&mut rng).into_affine();
        let tampered = G1AddWitness {
            p1: valid.p1,
            p2: valid.p2,
            p3: wrong_p3,
        };
        let cs = ConstraintSystem::<Fr>::new_ref();
        let circuit = WrapperCircuit::dummy().with_g1_add(tampered);
        circuit.generate_constraints(cs.clone()).expect("synthesize");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "tampered P3 must NOT satisfy wrapper constraints"
        );
    }

    /// Pin that the g1_add gadget is ACTUALLY being called from the
    /// wrapper. Pre-integration the wrapper had ~5 constraints
    /// (allocations + tautology); post-integration the g1_add
    /// gadget adds ~4000. If a future refactor accidentally drops
    /// the gadget call, the wrapper would silently revert to the
    /// pre-integration ~5 — this test catches that loudly.
    #[test]
    fn wrapper_constraint_count_includes_g1_add() {
        let cs = ConstraintSystem::<Fr>::new_ref();
        WrapperCircuit::dummy()
            .generate_constraints(cs.clone())
            .expect("synthesize");
        let n = cs.num_constraints();
        // Range pinned from the g1_add canonical-form constraint
        // count (~4037 per add) + the wrapper's anchor allocations
        // + the existing tautology. Reasonable bracketing — caught
        // both "gadget not called" (n ≈ 5) and "unexpected
        // arkworks-version-bump constraint inflation" (n > 8000).
        assert!(
            n > 2_000 && n < 8_000,
            "wrapper constraint count out of expected range: {} \
             (expected 2k-8k, dominated by one g1_add ≈ 4k)",
            n
        );
    }

    /// The dummy G1AddWitness is valid by construction (G, 2G, 3G).
    /// Pin this so a future refactor that accidentally breaks the
    /// dummy triple is caught — without it, trusted-setup ceremony
    /// would silently produce an invalid VK.
    #[test]
    fn dummy_g1_triple_precondition_holds() {
        assert!(G1AddWitness::dummy().precondition_holds());
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
