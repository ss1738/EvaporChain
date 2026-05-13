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
//!   - **Section 1: structural checks** — `num_steps != 0`, z0 non-empty,
//!     z0 / zi arity match. **DONE** as off-circuit precondition gate
//!     ([`NovaVerifierCircuit::validate_structurally`]). Mapped to
//!     `SynthesisError::Unsatisfiable` at the top of
//!     `generate_constraints` so trusted setup, prover, and verifier
//!     all see the same rejection contract. Field-level checks
//!     against `RecursiveSNARK` private fields
//!     (`self.i == num_steps`, `instance.X.len() == 2`) are deferred
//!     to the Phase 2.3 adapter.
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
use thiserror::Error;

/// Off-circuit structural-validation errors for [`NovaVerifierCircuit`].
///
/// These are cheap precondition checks the verifier can run *before*
/// touching Groth16 / Poseidon machinery. A failure here means the
/// witness is malformed at the level of basic shape — no expensive
/// pairing or hash work is needed to reject it.
///
/// Wired into `generate_constraints` (mapped to
/// `SynthesisError::Unsatisfiable`) so trusted setup, prover, and
/// verifier paths all see an identical rejection contract.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StructuralValidationError {
    /// `num_steps` is zero, but the verifier circuit's contract requires
    /// at least one fold step to have been executed. A zero-step instance
    /// would mean the `RecursiveSNARK` accumulator never ran, which is
    /// not a state the L1 verifier should accept.
    #[error("num_steps must be non-zero; got 0")]
    NumStepsZero,

    /// `z0` is empty. A zero-arity state vector means the step circuit
    /// has no actual state — degenerate and not a valid `RecursiveSNARK`
    /// instance shape.
    #[error("z0 must be non-empty; got len=0")]
    EmptyState,

    /// `z0.len() != zi.len()`. Both vectors are the chain's state at
    /// step 0 and step `num_steps` respectively; they must have the
    /// same arity (the step circuit's state type is fixed).
    #[error("z0 / zi arity mismatch: z0.len()={z0_len}, zi.len()={zi_len}")]
    MismatchedArity {
        /// Arity of the initial-state vector observed.
        z0_len: usize,
        /// Arity of the output-state vector observed.
        zi_len: usize,
    },
}

/// Phase 2.2 skeleton — verifier circuit for a `RecursiveSNARK<E1,
/// E2, C>` where `E1 = Bn256EngineKZG`, `E2 = GrumpkinEngine`, and
/// `C = TrivialIncrementCircuit` (or whichever step circuit the
/// chain side uses). Public-input arity matches what the L1
/// Solidity verifier expects.
///
/// The witness values are passed in as `Bn254Fr` already — the
/// type-conversion adapter (Phase 2.3) feeds them in pre-converted.
///
/// **Current state (post-PR #125):**
///   - Section 1 structural checks: **DONE** via off-circuit
///     [`Self::validate_structurally`] gate, wired into
///     `generate_constraints` and mapped to
///     `SynthesisError::Unsatisfiable`.
///   - Section 2 Poseidon transcript: still TODO in
///     `generate_constraints` (constants-layer landed earlier on a
///     parallel stack; sponge framing remains BESPOKE).
///   - Section 3 RelaxedR1CS satisfiability: still TODO in
///     `generate_constraints` (BESPOKE, 3-5 days research — see
///     Open Q4 in `DESIGN.md` on the parallel stack).
#[derive(Clone, Debug)]
pub struct NovaVerifierCircuit {
    /// Number of fold steps the accumulator has executed.
    /// Section-1 check: must equal `pp.num_steps_committed`.
    pub num_steps: u64,
    /// Initial state vector at step 0. Public input.
    pub z0: Vec<Bn254Fr>,
    /// Output state vector after `num_steps` folds. Public input.
    pub zi: Vec<Bn254Fr>,
    /// First of the two committed hashes — `l_u_secondary.X[0]`,
    /// extracted via [`crate::l_u_secondary_extract`] and passed
    /// through [`crate::scalar_adapter::secondary_to_ark_fr_lossy`]
    /// at adapter time. Section-2 check (when wired): the in-
    /// circuit re-hash of `(pp.digest, num_steps, z0, zi, …)` must
    /// equal this value.
    pub committed_hash_primary: Bn254Fr,
    /// Second of the two committed hashes — `l_u_secondary.X[1]`,
    /// extracted via [`crate::l_u_secondary_extract`] and passed
    /// through [`crate::scalar_adapter::secondary_to_ark_fr_lossy`]
    /// at adapter time. Section-2 check companion to
    /// [`Self::committed_hash_primary`].
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
    ///
    /// Uses `num_steps = 1` (not 0) so the structural-validation gate
    /// in `generate_constraints` accepts the dummy during setup —
    /// otherwise setup would `Unsatisfiable` and the prover key would
    /// never get generated.
    pub fn dummy() -> Self {
        Self {
            num_steps: 1,
            z0: vec![Bn254Fr::from(0u64)],
            zi: vec![Bn254Fr::from(0u64)],
            committed_hash_primary: Bn254Fr::from(0u64),
            committed_hash_secondary: Bn254Fr::from(0u64),
        }
    }

    /// Off-circuit structural-validation gate.
    ///
    /// Runs the three shape checks documented on
    /// [`StructuralValidationError`]: `num_steps != 0`, z0 non-empty,
    /// z0 / zi arity match. Returns `Ok(())` on success.
    ///
    /// Cheap (no field arithmetic, no hashing, no R1CS work). Called
    /// from the top of `generate_constraints` so that trusted setup,
    /// proving, and verification all enforce the same precondition.
    ///
    /// Callers running a CLI verifier can also invoke this directly
    /// for a fast pre-check before paying the cost of Groth16 verify
    /// (~10ms).
    pub fn validate_structurally(&self) -> Result<(), StructuralValidationError> {
        if self.num_steps == 0 {
            return Err(StructuralValidationError::NumStepsZero);
        }
        if self.z0.is_empty() {
            return Err(StructuralValidationError::EmptyState);
        }
        if self.z0.len() != self.zi.len() {
            return Err(StructuralValidationError::MismatchedArity {
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
        // ── Section 1: Structural checks ────────────────────────────
        //
        // Off-circuit precondition gate: cheap shape checks that reject
        // malformed witnesses without touching Groth16 / Poseidon. Any
        // failure maps to `SynthesisError::Unsatisfiable` so trusted
        // setup, prover, and verifier all see the same contract.
        //
        // `dummy()` is constructed with `num_steps = 1` specifically so
        // it passes this gate at setup time.
        //
        // Per-field rationale: see `StructuralValidationError`.
        // Deferred to the Phase 2.3 adapter:
        //   - `self.i == num_steps` (requires `RecursiveSNARK` private field)
        //   - `instance.X.len() == 2` (validated adapter-side)
        self.validate_structurally()
            .map_err(|_| SynthesisError::Unsatisfiable)?;

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

    /// Empirical pin: `dummy()` must produce a *satisfied* constraint
    /// system, not just one that synthesizes without panicking.
    ///
    /// This is the contract trusted setup depends on — if dummy()
    /// were Unsatisfiable, Groth16's `Groth16::setup(dummy, rng)` call
    /// would silently produce a prover key that no real witness could
    /// later satisfy.
    ///
    /// Catches future regressions where Section 2 / Section 3 wire-in
    /// code accidentally enforces a constraint on dummy()'s zero
    /// witness values (e.g. a Poseidon-hash equality check that fails
    /// when committed_hash_primary == 0 and the re-hashed value != 0).
    #[test]
    fn dummy_circuit_is_satisfied_under_section_1_gate() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        NovaVerifierCircuit::dummy()
            .generate_constraints(cs.clone())
            .expect("dummy synthesize");
        assert!(
            cs.is_satisfied().expect("is_satisfied check"),
            "dummy() must produce a satisfied CS — Section 1 gate accepts dummy \
             (num_steps=1, |z0|=|zi|=1) and no later section adds an unsatisfied \
             constraint on dummy's zero witness"
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

    /// Happy path: a well-formed circuit passes `validate_structurally`.
    #[test]
    fn validate_structurally_accepts_well_formed() {
        let circuit = NovaVerifierCircuit::new(
            7,
            vec![Bn254Fr::from(1u64), Bn254Fr::from(2u64)],
            vec![Bn254Fr::from(3u64), Bn254Fr::from(4u64)],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        assert_eq!(circuit.validate_structurally(), Ok(()));
    }

    /// `dummy()` is constructed with `num_steps = 1` so trusted-setup-
    /// time synthesis doesn't hit the structural gate. Pin that here.
    #[test]
    fn validate_structurally_accepts_dummy() {
        assert_eq!(NovaVerifierCircuit::dummy().validate_structurally(), Ok(()));
    }

    /// `num_steps = 0` is rejected with the typed error variant.
    #[test]
    fn validate_structurally_rejects_zero_num_steps() {
        let circuit = NovaVerifierCircuit::new(
            0,
            vec![Bn254Fr::from(0u64)],
            vec![Bn254Fr::from(0u64)],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        assert_eq!(
            circuit.validate_structurally(),
            Err(StructuralValidationError::NumStepsZero),
        );
    }

    /// Empty `z0` is rejected with the typed error variant.
    #[test]
    fn validate_structurally_rejects_empty_state() {
        let circuit = NovaVerifierCircuit::new(
            1,
            vec![],
            vec![],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        assert_eq!(
            circuit.validate_structurally(),
            Err(StructuralValidationError::EmptyState),
        );
    }

    /// Mismatched z0/zi arity is rejected with the typed error variant
    /// carrying both observed lengths.
    #[test]
    fn validate_structurally_rejects_mismatched_arity() {
        let circuit = NovaVerifierCircuit::new(
            1,
            vec![Bn254Fr::from(1u64), Bn254Fr::from(2u64)],
            vec![Bn254Fr::from(3u64)],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        assert_eq!(
            circuit.validate_structurally(),
            Err(StructuralValidationError::MismatchedArity {
                z0_len: 2,
                zi_len: 1,
            }),
        );
    }

    /// Wire test: a structurally-invalid circuit produces
    /// `SynthesisError::Unsatisfiable` from `generate_constraints`,
    /// not a `Ok(())` quietly-accepting state. This is the contract
    /// that trusted setup, prover, and verifier all share.
    #[test]
    fn generate_constraints_rejects_zero_num_steps_as_unsatisfiable() {
        let circuit = NovaVerifierCircuit::new(
            0,
            vec![Bn254Fr::from(0u64)],
            vec![Bn254Fr::from(0u64)],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let result = circuit.generate_constraints(cs.clone());
        assert!(
            matches!(result, Err(SynthesisError::Unsatisfiable)),
            "expected SynthesisError::Unsatisfiable for num_steps=0, got {result:?}"
        );
    }

    /// Same wire test for mismatched arity — the most operationally
    /// likely failure mode (caller fed in the wrong z0 vector from an
    /// older block).
    #[test]
    fn generate_constraints_rejects_mismatched_arity_as_unsatisfiable() {
        let circuit = NovaVerifierCircuit::new(
            5,
            vec![Bn254Fr::from(1u64), Bn254Fr::from(2u64), Bn254Fr::from(3u64)],
            vec![Bn254Fr::from(4u64)],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let result = circuit.generate_constraints(cs.clone());
        assert!(
            matches!(result, Err(SynthesisError::Unsatisfiable)),
            "expected SynthesisError::Unsatisfiable for mismatched arity, got {result:?}"
        );
    }
}
