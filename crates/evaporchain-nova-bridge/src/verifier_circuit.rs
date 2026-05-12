//! Phase 2.2 finish step 1 of N — arkworks R1CS skeleton for the
//! in-circuit `RecursiveSNARK<E1, E2, C>::verify` algorithm.
//!
//! # What this module ships (Phase 2.2 partial — ~15% complete)
//!
//! The arkworks-side `ConstraintSynthesizer` type ([`NovaVerifierCircuit`])
//! that the Groth16 wrapper eventually proves. Carries the witness
//! values (z0, zi, num_steps, R1CS instances) and the three
//! verification sections sketched but not implemented:
//!
//!   - **Section 1: structural checks** — `num_steps != 0`,
//!     `self.i == num_steps`, `self.z0 == z0`, instances have 2
//!     public outputs. Cheap. **TODO in synthesize() — straightforward;
//!     ~10 lines per check.**
//!
//!   - **Section 2: Poseidon transcript hash** — re-hash
//!     `(pp.digest, num_steps, z0, zi, R1CS-instance, ri)` with
//!     Poseidon and compare against the two committed hashes on
//!     `l_u_secondary.X[..2]`. Uses arkworks Poseidon gadget (already
//!     available in `ark-r1cs-std`). **TODO in synthesize() — ~1 day
//!     work using existing Poseidon gadgets; the open question is
//!     parameter alignment with nova-snark's Poseidon (which uses
//!     bellman-style sponge constants).**
//!
//!   - **Section 3: RelaxedR1CS satisfiability** — verify the three
//!     R1CS instances are satisfied by their witnesses
//!     (`is_sat_relaxed` × 2 + `is_sat` × 1). **TODO in synthesize()
//!     — this is the BESPOKE part. ~3 days work + research into
//!     Nova's sparse-R1CS encoding. The verifier needs to encode
//!     `<a, z> · <b, z> = <c, z>` for each row of the constraint
//!     system, OR use a sumcheck-style protocol-replay (smaller).
//!     Open Q4 in `DESIGN.md`.**
//!
//! # Type-conversion note (open architecture question)
//!
//! `RecursiveSNARK<E1=Bn256EngineKZG, E2=GrumpkinEngine, C>` carries
//! nova-snark's native field types (its own `bn256::Scalar`,
//! `grumpkin::Scalar`). The arkworks circuit operates on
//! `ark_bn254::Fr`. These are the SAME mathematical fields but
//! DIFFERENT Rust types.
//!
//! Phase 2.2's adapter (next milestone, separate file) will bridge
//! them via byte serialization: `nova-snark scalar → 32 LE bytes →
//! ark_bn254::Fr`. This skeleton uses `ark_bn254::Fr` directly for
//! the witness slots and assumes the adapter pre-converts.
//!
//! See `DESIGN.md` for the full Phase 2 milestone breakdown.

use ark_bn254::Fr as Bn254Fr;
use ark_r1cs_std::alloc::AllocVar;
use ark_r1cs_std::fields::fp::FpVar;
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

/// Phase 2.2 skeleton — verifier circuit for a `RecursiveSNARK<E1,
/// E2, C>` where `E1 = Bn256EngineKZG`, `E2 = GrumpkinEngine`, and
/// `C = TrivialIncrementCircuit` (or whichever step circuit the
/// chain side uses). Public-input arity matches what the L1
/// Solidity verifier expects.
///
/// The witness values are passed in as `Bn254Fr` already — the
/// type-conversion adapter (Phase 2.3) feeds them in pre-converted.
///
/// **Current state:** structural skeleton + ConstraintSynthesizer
/// stub. Sections 1-3 of the verify algorithm are documented as
/// TODOs in `generate_constraints` with effort estimates inline.
#[derive(Clone, Debug)]
pub struct NovaVerifierCircuit {
    /// Number of fold steps the accumulator has executed.
    /// Section-1 check: must equal `pp.num_steps_committed`.
    pub num_steps: u64,
    /// Initial state vector at step 0. Public input.
    pub z0: Vec<Bn254Fr>,
    /// Output state vector after `num_steps` folds. Public input.
    pub zi: Vec<Bn254Fr>,
    /// The two committed hashes from `l_u_secondary.X[..2]`.
    /// Section-2 check: re-hash everything and compare.
    pub committed_hash_primary: Bn254Fr,
    /// Companion to `committed_hash_primary` — `l_u_secondary.X[1]`.
    /// Pre-converted from `<E2 as Engine>::Scalar` to `Bn254Fr` via
    /// PR #66's scalar adapter at adapter-time.
    pub committed_hash_secondary: Bn254Fr,
    // Section-3 witnesses (RelaxedR1CS instances + their R1CS
    // satisfying assignments) are intentionally NOT field members
    // yet — the type to use depends on the adapter design decision
    // (Phase 2.3). Adding them as `Vec<Bn254Fr>` placeholders without
    // a concrete schema would force a churn-prone refactor when 2.3
    // lands.
}

impl NovaVerifierCircuit {
    /// Constructor with explicit witness values.
    pub fn new(
        num_steps: u64,
        z0: Vec<Bn254Fr>,
        zi: Vec<Bn254Fr>,
        committed_hash_primary: Bn254Fr,
        committed_hash_secondary: Bn254Fr,
    ) -> Self {
        Self {
            num_steps,
            z0,
            zi,
            committed_hash_primary,
            committed_hash_secondary,
        }
    }

    /// Dummy instance for Groth16 setup. Shape-stable across the
    /// real call so the trusted-setup ceremony's IC[] table matches.
    pub fn dummy() -> Self {
        Self {
            num_steps: 0,
            z0: vec![Bn254Fr::from(0u64)],
            zi: vec![Bn254Fr::from(0u64)],
            committed_hash_primary: Bn254Fr::from(0u64),
            committed_hash_secondary: Bn254Fr::from(0u64),
        }
    }
}

impl ConstraintSynthesizer<Bn254Fr> for NovaVerifierCircuit {
    fn generate_constraints(
        self,
        cs: ConstraintSystemRef<Bn254Fr>,
    ) -> Result<(), SynthesisError> {
        // ── Public input wiring (stable across all Phase 2.2 sub-steps) ──
        //
        // The L1 verifier reads:
        //   1. The two hash anchors (l_u_secondary.X[..2])
        //   2. The initial-state z0 vector
        //   3. The output-state zi vector
        //
        // num_steps is NOT a public input — it's an immediate
        // baked into the circuit shape at trusted-setup time
        // (each chain advance has a fixed num_steps cadence).
        let _committed_hash_primary_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.committed_hash_primary))?;
        let _committed_hash_secondary_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.committed_hash_secondary))?;
        for z in &self.z0 {
            let _ = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(*z))?;
        }
        for z in &self.zi {
            let _ = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(*z))?;
        }

        // ── Section 1: Structural checks ────────────────────────────
        //
        // TODO (Phase 2.2 step 2 of N — ~half day work):
        //   - assert num_steps != 0 (use `Boolean::is_zero` + enforce_false)
        //   - assert self.i == num_steps (the .i field is an immediate
        //     baked into the circuit; this comes via the adapter)
        //   - assert z0 / zi shapes match expected arity
        //   - assert instances each have X.len() == 2 (validated at
        //     adapter-time before this circuit even sees them)
        //
        // Source: nova-snark/src/nova/mod.rs:574-595

        // ── Section 2: Poseidon transcript hash ────────────────────
        //
        // TODO (Phase 2.2 step 3 of N — ~1 day work):
        //   - Use ark-crypto-primitives Poseidon gadget
        //   - Absorb: pp.digest (constant), num_steps (constant),
        //     z0[..] (public), zi[..] (public), R1CS-instance fields
        //     (via adapter), ri_primary/ri_secondary (witness)
        //   - Squeeze NUM_HASH_BITS
        //   - Compare against committed_hash_primary/secondary vars
        //
        // OPEN QUESTION: align Poseidon parameters with nova-snark's
        // bellman-style sponge. nova-snark uses its own Poseidon
        // constants; arkworks ships generic Poseidon. Parameter
        // mismatch → hash divergence → false-reject.
        //
        // Source: nova-snark/src/nova/mod.rs:597-630 (hash_primary,
        // hash_secondary construction).

        // ── Section 3: RelaxedR1CS satisfiability check ────────────
        //
        // TODO (Phase 2.2 step 4 of N — BESPOKE, ~3-5 days research):
        //   Three R1CS-satisfaction checks:
        //     1. r1cs_shape_primary.is_sat_relaxed(ck, r_U_primary,
        //        r_W_primary)
        //     2. r1cs_shape_secondary.is_sat_relaxed(ck, r_U_secondary,
        //        r_W_secondary)
        //     3. r1cs_shape_secondary.is_sat(ck, l_u_secondary,
        //        l_w_secondary)
        //
        // Each is_sat / is_sat_relaxed verifies that for every row
        // (a, b, c) of the R1CS matrix triple, <a, z>·<b, z> = <c, z>
        // (with the relaxation slack for RelaxedR1CS). Naively
        // encoding this in-circuit costs O(N_rows × cost_per_check)
        // — easily 100k+ constraints if not careful.
        //
        // Open research path (the bespoke part): use a sumcheck-style
        // protocol replay OR direct sparse-R1CS evaluation gadget.
        //
        // Source: nova-snark/src/nova/mod.rs:634-665 (rayon::join
        // wrapping the three is_sat calls).

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_relations::r1cs::ConstraintSystem;

    /// Pin that the skeleton circuit compiles, synthesizes, and
    /// produces the expected public-input arity (the L1-visible
    /// wiring stays stable across Phase 2.2's sub-steps).
    #[test]
    fn skeleton_dummy_synthesizes_with_expected_public_input_arity() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        NovaVerifierCircuit::dummy()
            .generate_constraints(cs.clone())
            .expect("dummy synthesize");
        // Public inputs: 2 hashes + 1 z0 entry + 1 zi entry = 4
        // Plus the Groth16-convention constant input = 5 total.
        assert_eq!(
            cs.num_instance_variables(),
            5,
            "Phase 2.2 public-input arity contract: 2 hashes + |z0| + |zi| + 1 const"
        );
    }

    /// Pin that a circuit with non-trivial state-vector arity
    /// produces matching public-input count. Real chain `z` vectors
    /// will be larger (e.g., 4-entry state hash); this catches any
    /// off-by-one in the public-input wiring when Section 1/2 are
    /// filled in.
    #[test]
    fn skeleton_arity_scales_with_state_vector_size() {
        let z0 = vec![Bn254Fr::from(1u64); 4];
        let zi = vec![Bn254Fr::from(2u64); 4];
        let circuit = NovaVerifierCircuit::new(
            10,
            z0,
            zi,
            Bn254Fr::from(0xabcdu64),
            Bn254Fr::from(0xef01u64),
        );
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).expect("synthesize");
        // 2 hashes + 4 z0 + 4 zi + 1 const = 11
        assert_eq!(cs.num_instance_variables(), 11);
    }
}
