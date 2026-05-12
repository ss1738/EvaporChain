//! Phase 2.2-section-1 — arkworks R1CS skeleton for the
//! in-circuit `RecursiveSNARK<E1, E2, C>::verify` algorithm.
//!
//! # What this module ships (Phase 2.2 partial — ~25% complete)
//!
//! The arkworks-side `ConstraintSynthesizer` type ([`NovaVerifierCircuit`])
//! that the Groth16 wrapper eventually proves. Carries the witness
//! values (z0, zi, num_steps, R1CS instances) and the three
//! verification sections:
//!
//!   - **Section 1: structural checks** — `num_steps != 0`, `z0`
//!     non-empty, `z0.len() == zi.len()`. **DONE** as an off-circuit
//!     precondition gate ([`NovaVerifierCircuit::validate_structurally`])
//!     called at the top of `generate_constraints`. Field-level
//!     checks (`self.i == num_steps`, `instance.X.len() == 2`) belong
//!     in the Phase 2.3 adapter — they need access to
//!     `RecursiveSNARK` private fields that aren't witnessed in the
//!     verifier circuit. See `StructuralValidationError`.
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

/// Structural-validation errors surfaced by [`NovaVerifierCircuit::validate_structurally`].
///
/// Returned BEFORE any constraint-system allocation, so a malformed
/// circuit fails fast with a typed error rather than producing a
/// satisfying proof for the wrong shape. The adapter (Phase 2.3) is
/// expected to call `validate_structurally` immediately after
/// constructing the circuit from raw `RecursiveSNARK` bytes — any
/// failure there is a hard error before reaching the prover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructuralValidationError {
    /// `num_steps == 0` — the accumulator claims zero folds, which
    /// is meaningless. Maps to `nova-snark/src/nova/mod.rs:575`
    /// `is_num_steps_zero`.
    NumStepsIsZero,
    /// `z0.is_empty()` — the state vector has zero arity, which
    /// cannot represent any chain state. Caught here because the
    /// downstream circuit shape would degenerate.
    Z0Empty,
    /// `z0.len() != zi.len()` — initial-state arity must match
    /// output-state arity (the step circuit preserves arity). Maps
    /// to `nova-snark/src/nova/mod.rs:578` `is_inputs_not_match`
    /// generalised to vector lengths.
    StateVectorArityMismatch {
        /// Length of the initial state vector `z0`.
        z0_len: usize,
        /// Length of the current state vector `zi` after `num_steps` folds.
        zi_len: usize,
    },
}

impl std::fmt::Display for StructuralValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NumStepsIsZero => write!(f, "num_steps is zero — accumulator must have ≥1 fold"),
            Self::Z0Empty => write!(f, "z0 is empty — state vector arity must be ≥1"),
            Self::StateVectorArityMismatch { z0_len, zi_len } => write!(
                f,
                "state vector arity mismatch: z0.len() = {z0_len}, zi.len() = {zi_len}"
            ),
        }
    }
}

impl std::error::Error for StructuralValidationError {}

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
    /// `num_steps = 1` (NOT 0) so the structural-validation pass
    /// accepts this instance — without this, calling
    /// `Groth16::circuit_specific_setup(NovaVerifierCircuit::dummy())`
    /// would fail at the structural-validation stage.
    pub fn dummy() -> Self {
        Self {
            num_steps: 1,
            z0: vec![Bn254Fr::from(0u64)],
            zi: vec![Bn254Fr::from(0u64)],
            committed_hash_primary: Bn254Fr::from(0u64),
            committed_hash_secondary: Bn254Fr::from(0u64),
        }
    }

    /// Section-1 structural validation (Phase 2.2-section-1).
    ///
    /// Runs the off-circuit precondition checks corresponding to
    /// `nova-snark::nova::RecursiveSNARK::verify` lines 574-595
    /// that map cleanly to non-cryptographic shape-checks. The
    /// remaining structural checks (`self.i == num_steps`, instance
    /// `X.len() == 2`) require fields from a real `RecursiveSNARK`
    /// value that this circuit struct does not yet carry; those move
    /// to the adapter (Phase 2.3) which builds `NovaVerifierCircuit`
    /// from a `RecursiveSNARK` and is the right layer to compare
    /// `accumulator.i` against the claimed `num_steps`.
    ///
    /// Called from [`generate_constraints`] before any allocation,
    /// so structural failures error out without polluting the
    /// constraint system.
    pub fn validate_structurally(&self) -> Result<(), StructuralValidationError> {
        if self.num_steps == 0 {
            return Err(StructuralValidationError::NumStepsIsZero);
        }
        if self.z0.is_empty() {
            return Err(StructuralValidationError::Z0Empty);
        }
        if self.z0.len() != self.zi.len() {
            return Err(StructuralValidationError::StateVectorArityMismatch {
                z0_len: self.z0.len(),
                zi_len: self.zi.len(),
            });
        }
        Ok(())
    }
}

impl ConstraintSynthesizer<Bn254Fr> for NovaVerifierCircuit {
    fn generate_constraints(
        self,
        cs: ConstraintSystemRef<Bn254Fr>,
    ) -> Result<(), SynthesisError> {
        // ── Section 1: Structural validation (Phase 2.2-section-1) ──
        //
        // Run off-circuit precondition checks BEFORE any allocation.
        // Maps to `nova-snark::nova::RecursiveSNARK::verify` lines
        // 574-595. The arity / shape checks happen here; the
        // accumulator-state checks (`self.i == num_steps`, instance
        // `X.len() == 2`) belong in the adapter (Phase 2.3) which has
        // access to the underlying `RecursiveSNARK` fields.
        //
        // A structural failure here is surfaced as
        // `SynthesisError::Unsatisfiable` so callers get a typed
        // error path without the prover producing a (wrongly-shaped)
        // witness assignment.
        self.validate_structurally().map_err(|_| SynthesisError::Unsatisfiable)?;

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
        // **DONE** as off-circuit precondition gate (see the call to
        // `validate_structurally()` at the top of this function):
        //   - num_steps != 0
        //   - z0 non-empty
        //   - z0.len() == zi.len() (state-vector arity match)
        //
        // **Adapter-time** (Phase 2.3, off-circuit, with access to
        // RecursiveSNARK private fields):
        //   - self.i == num_steps (verifier check on the immediate
        //     fold counter)
        //   - instance.X.len() == 2 for the three R1CS instances
        //   - z0 / zi shapes match the StepCircuit arity baked into
        //     the public parameters digest
        //
        // Why split this way: the spec-listed checks that compare
        // *fields of the proof object* (`.i`, instance lengths) can
        // only be expressed once the bytes have been parsed into a
        // `RecursiveSNARK`. The verifier circuit only sees the
        // already-reduced public scalars (committed hashes, z0, zi).
        // Putting field-level checks here would force the prover to
        // re-allocate the entire RecursiveSNARK structure as witness
        // — orders of magnitude wasted constraints for a check the
        // adapter does in microseconds.
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

    // ── Section-1 structural-validation tests ──────────────────────
    //
    // Pin off-circuit precondition behaviour:
    // 1. Well-formed circuit validates and synthesizes.
    // 2. Each of the 3 documented failure shapes is caught by
    //    `validate_structurally` AND propagates as
    //    `SynthesisError::Unsatisfiable` through `generate_constraints`.

    #[test]
    fn validate_structurally_accepts_well_formed_circuit() {
        let circuit = NovaVerifierCircuit::new(
            1,
            vec![Bn254Fr::from(7u64)],
            vec![Bn254Fr::from(11u64)],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        assert_eq!(circuit.validate_structurally(), Ok(()));
    }

    #[test]
    fn validate_structurally_rejects_zero_num_steps() {
        let circuit = NovaVerifierCircuit::new(
            0,
            vec![Bn254Fr::from(1u64)],
            vec![Bn254Fr::from(1u64)],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        assert_eq!(
            circuit.validate_structurally(),
            Err(StructuralValidationError::NumStepsIsZero)
        );
    }

    #[test]
    fn validate_structurally_rejects_empty_z0() {
        let circuit = NovaVerifierCircuit::new(
            1,
            vec![],
            vec![],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        assert_eq!(
            circuit.validate_structurally(),
            Err(StructuralValidationError::Z0Empty)
        );
    }

    #[test]
    fn validate_structurally_rejects_arity_mismatch() {
        let circuit = NovaVerifierCircuit::new(
            1,
            vec![Bn254Fr::from(1u64); 3],
            vec![Bn254Fr::from(2u64); 5],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        assert_eq!(
            circuit.validate_structurally(),
            Err(StructuralValidationError::StateVectorArityMismatch {
                z0_len: 3,
                zi_len: 5,
            })
        );
    }

    #[test]
    fn generate_constraints_surfaces_structural_failure_as_unsatisfiable() {
        // num_steps = 0 → Section-1 gate must short-circuit before
        // any public-input allocation runs.
        let circuit = NovaVerifierCircuit::new(
            0,
            vec![Bn254Fr::from(1u64)],
            vec![Bn254Fr::from(1u64)],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let err = circuit
            .generate_constraints(cs.clone())
            .expect_err("structurally-invalid circuit must not synthesize");
        assert!(
            matches!(err, SynthesisError::Unsatisfiable),
            "expected Unsatisfiable, got {err:?}"
        );
        // No public inputs allocated when the precondition fails.
        // (The Groth16-convention constant input is still present.)
        assert_eq!(cs.num_instance_variables(), 1);
    }
}
