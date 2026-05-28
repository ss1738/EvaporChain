//! Phase 2.2 finish step 1 of N — arkworks R1CS skeleton for the
//! in-circuit `RecursiveSNARK<E1, E2, C>::verify` algorithm.
//!
//! # What this module ships (Phase 2.2 DONE — all three sections wired)
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
//!   - **Section 2: Neptune sponge transcript hash** -- when
//!     `section2: Some(Section2Witness)` is attached, `enforce_neptune_sponge_primary`
//!     enforces the 250-bit truncated hash equals `committed_hash_primary`.
//!     **DONE 2026-05-13.**
//!
//!   - **Section 3: primary RelaxedR1CS satisfiability** -- when
//!     `section3: Some(Section3Witness)` is attached, `enforce_primary_relaxed_r1cs_sat`
//!     enforces `(Az)_i * (Bz)_i == u * (Cz)_i + E_i` for every row i.
//!     **DONE 2026-05-14.**
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
use ark_r1cs_std::boolean::Boolean;
use ark_r1cs_std::convert::ToBitsGadget;
use ark_r1cs_std::eq::EqGadget;
use ark_r1cs_std::fields::fp::FpVar;
use ark_r1cs_std::GR1CSVar;
use ark_relations::gr1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};
use thiserror::Error;

use crate::section2_witness::Section2Witness;
use crate::section3_witness::Section3Witness;

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

    /// Section 2 (Neptune transcript-hash binding) witness is absent.
    ///
    /// Audit B-1/B-2 S2b: the soundness bindings (Sections 2 & 3) are
    /// MANDATORY. A circuit with no Section 2 witness emits zero
    /// transcript-hash constraints — the exact constraint-vacuity that
    /// makes Groth16 keys forgeable. Such a circuit must never reach
    /// trusted setup, the prover, or the verifier.
    #[error("section2 (Neptune transcript binding) witness is mandatory but absent")]
    MissingSection2,

    /// Section 3 (primary RelaxedR1CS-satisfiability binding) witness
    /// is absent. Same B-1/B-2 S2b rationale as [`Self::MissingSection2`]:
    /// without it the circuit proves nothing about the folded instance.
    #[error("section3 (primary RelaxedR1CS binding) witness is mandatory but absent")]
    MissingSection3,

    /// Section 3 witness is internally inconsistent: its declared
    /// dimensions and its actual vector lengths disagree. The witness
    /// vector must have `num_vars` entries and the error vector must
    /// have `num_cons` entries. A witness that fails these cheap
    /// invariants is malformed and is rejected before any field /
    /// pairing work. (Exact equality to the canonical R1CS shape —
    /// including `num_cons`/`num_vars`/`num_io` magnitudes — is
    /// enforced *cryptographically* by the Groth16 keys, which S2a
    /// pins to `setup_shape()`'s shape and S6 proves bit-identical to
    /// a real prover's; it is deliberately NOT re-hardcoded here.)
    #[error(
        "section3 dims inconsistent: num_vars={num_vars} \
         w_len={w_len} e_len={e_len} num_cons={num_cons}"
    )]
    Section3DimsInconsistent {
        /// Declared private-witness count.
        num_vars: usize,
        /// Actual length of the witness vector `w_primary`.
        w_len: usize,
        /// Actual length of the error vector `e_primary`.
        e_len: usize,
        /// Declared constraint-row count.
        num_cons: usize,
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
///   - Section 2 Neptune sponge: wired 2026-05-13, gated on `section2.is_some()`.
///   - Section 3 primary RelaxedR1CS: wired 2026-05-14, gated on `section3.is_some()`.
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
    /// Section 2 Neptune witness. When `Some`, `generate_constraints`
    /// enforces the primary transcript hash in-circuit. `None` for
    /// `dummy()` (trusted setup).
    pub section2: Option<Section2Witness>,
    /// Section 3 primary RelaxedR1CS witness. When `Some`,
    /// `generate_constraints` enforces the primary R1CS row check in-circuit.
    /// `None` for `dummy()` (trusted setup safety).
    pub section3: Option<Section3Witness>,
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
            section2: None,
            section3: None,
        }
    }

    /// Attach a Section 2 Neptune witness. Builder-style; returns `self`.
    pub fn with_section2(mut self, s2: Section2Witness) -> Self {
        self.section2 = Some(s2);
        self
    }

    /// Attach a Section 3 primary RelaxedR1CS witness. Builder-style.
    pub fn with_section3(mut self, s3: Section3Witness) -> Self {
        self.section3 = Some(s3);
        self
    }

    /// Dummy instance for Groth16 setup. Shape-stable across the
    /// real call so the trusted-setup ceremony's IC[] table matches.
    ///
    /// Uses `num_steps = 1` (not 0) so the structural-validation gate
    /// in `generate_constraints` accepts the dummy during setup —
    /// Audit B-1/B-2 S2a: the trusted-setup circuit shape — `dummy()`
    /// at canonical arity WITH section2+section3 attached at the exact
    /// prover R1CS shape (derived from canonical `PublicParams`, no
    /// proof). This makes setup ≡ prover ≡ verifier R1CS so the
    /// binding constraints are part of the keyed circuit. The
    /// `#[deprecated]` insecure-randomness caveat in `groth16_wrapper`
    /// still applies (S5/MPC ceremony is the separate, unchanged gap).
    pub fn setup_shape() -> Result<Self, crate::l_u_secondary_extract::ExtractError> {
        let pp = crate::recursive_snark_fixture::canonical_public_params()
            .map_err(crate::l_u_secondary_extract::ExtractError::Serialize)?;
        let mut c = Self::dummy();
        c.section3 = Some(Section3Witness::canonical_shape(&pp)?);
        c.section2 = Some(Section2Witness::canonical_shape(pp.digest())?);
        Ok(c)
    }

    /// otherwise setup would `Unsatisfiable` and the prover key would
    /// never get generated.
    pub fn dummy() -> Self {
        Self {
            num_steps: 1,
            z0: vec![Bn254Fr::from(0u64)],
            zi: vec![Bn254Fr::from(0u64)],
            committed_hash_primary: Bn254Fr::from(0u64),
            committed_hash_secondary: Bn254Fr::from(0u64),
            section2: None,
            section3: None,
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

        // ── Audit B-1/B-2 S2b: soundness bindings are MANDATORY ──────
        //
        // Both section witnesses must be present, and Section 3's
        // declared dims must be internally consistent. These are cheap
        // (Option::is_some + integer/len compares) — no field, hash, or
        // pairing work — consistent with this gate's "reject malformed
        // shapes without paying Groth16 cost" contract. Exact equality
        // to the canonical R1CS shape is enforced *cryptographically*:
        // S2a pins the Groth16 keys to `setup_shape()`'s shape, so a
        // prover substituting a different-dimensioned circuit produces
        // an R1CS the keys do not verify.
        if self.section2.is_none() {
            return Err(StructuralValidationError::MissingSection2);
        }
        let s3 = self
            .section3
            .as_ref()
            .ok_or(StructuralValidationError::MissingSection3)?;
        if s3.w_primary.len() != s3.num_vars || s3.e_primary.len() != s3.num_cons {
            return Err(StructuralValidationError::Section3DimsInconsistent {
                num_vars: s3.num_vars,
                w_len: s3.w_primary.len(),
                e_len: s3.e_primary.len(),
                num_cons: s3.num_cons,
            });
        }

        Ok(())
    }
}

impl ConstraintSynthesizer<Bn254Fr> for NovaVerifierCircuit {
    fn generate_constraints(self, cs: ConstraintSystemRef<Bn254Fr>) -> Result<(), SynthesisError> {
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
        let committed_hash_primary_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.committed_hash_primary))?;
        let _committed_hash_secondary_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.committed_hash_secondary))?;
        let z0_vars: Vec<FpVar<Bn254Fr>> = self
            .z0
            .iter()
            .map(|z| FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(*z)))
            .collect::<Result<_, _>>()?;
        let zi_vars: Vec<FpVar<Bn254Fr>> = self
            .zi
            .iter()
            .map(|z| FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(*z)))
            .collect::<Result<_, _>>()?;

        // ── Section 2: Neptune transcript hash ──────────────────────
        //
        // When `section2` is present, enforce that neptune_sponge over
        // the 18-element primary absorb sequence, truncated to 250 bits
        // (nova-snark NUM_HASH_BITS), equals `committed_hash_primary`.
        // Source: nova-snark/src/nova/mod.rs:597-630.
        // Audit B-1/B-2 S2b: MANDATORY (no `Option` gate). `validate_
        // structurally` above already rejects a missing Section 2 with
        // `Unsatisfiable`; this `ok_or` is the defense-in-depth second
        // line so the binding can never be silently skipped.
        {
            let s2 = self
                .section2
                .as_ref()
                .ok_or(SynthesisError::AssignmentMissing)?;
            use crate::section2_gadget::enforce_neptune_sponge_primary;

            let z0_vals: Vec<Bn254Fr> = z0_vars
                .iter()
                .map(|v| v.value().unwrap_or(Bn254Fr::from(0u64)))
                .collect();
            let zi_vals: Vec<Bn254Fr> = zi_vars
                .iter()
                .map(|v| v.value().unwrap_or(Bn254Fr::from(0u64)))
                .collect();
            let absorb_seq = s2.absorb_seq(self.num_steps, &z0_vals, &zi_vals);
            let seq_vars: Vec<FpVar<Bn254Fr>> = absorb_seq
                .into_iter()
                .map(|s| FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(s)))
                .collect::<Result<_, _>>()?;

            let hash_var = enforce_neptune_sponge_primary(cs.clone(), &s2.params, &seq_vars)?;

            // Truncate to 250 bits (NUM_HASH_BITS) before comparing.
            let bits = hash_var.to_bits_le()?;
            let truncated = Boolean::<Bn254Fr>::le_bits_to_fp(&bits[..250])?;
            committed_hash_primary_var.enforce_equal(&truncated)?;
        }

        // ── Section 3: primary RelaxedR1CS row satisfiability ──────
        //
        // When `section3` is present, enforce for each row i of the
        // primary R1CS: (Az)_i * (Bz)_i == u * (Cz)_i + E_i
        // where z = [W, u, X[0], X[1]].
        //
        // Cost: num_cons mult gates (~10k for TrivialIncrementCircuit).
        // Witnesses: W (num_vars), E (num_cons), u — all allocated as
        // private witnesses here; x_primary built from z0/zi public vars.
        //
        // Gaps (documented in section3_witness.rs):
        //   - Commitment checks (comm_W, comm_E) deferred: need KZG pairing.
        //   - Secondary R1CS checks deferred: Grumpkin Fr is non-native.
        //
        // Source: nova-snark/src/nova/mod.rs:634-665
        // Audit B-1/B-2 S2b: MANDATORY (no `Option` gate) — see the
        // Section 2 note above.
        {
            let s3 = self
                .section3
                .as_ref()
                .ok_or(SynthesisError::AssignmentMissing)?;
            use crate::section3_gadget::enforce_primary_relaxed_r1cs_sat;
            // The x_primary for the gadget is r_U_primary.X[0..2].
            // These are allocated as private witnesses (not public inputs)
            // because they are the primary instance's hash outputs, distinct
            // from l_u_secondary.X[..2] which are the public inputs above.
            let x_primary_vars: Vec<FpVar<Bn254Fr>> = s3
                .x_primary
                .iter()
                .map(|&val| FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(val)))
                .collect::<Result<_, _>>()?;
            enforce_primary_relaxed_r1cs_sat(cs.clone(), s3, &x_primary_vars)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_relations::gr1cs::ConstraintSystem;

    /// Pin that the skeleton circuit compiles, synthesizes, and
    /// produces the expected public-input arity (the L1-visible
    /// wiring stays stable across Phase 2.2's sub-steps).
    /// Audit B-1/B-2 S2b: a section-less `dummy()` no longer
    /// synthesizes — `generate_constraints` rejects it at the
    /// mandatory-binding gate before any public-input wiring. The
    /// canonical 5-input arity contract is pinned instead by the S6
    /// determinism proof and the real-fixture positive
    /// (`groth16_wrapper::tests::
    /// prove_and_verify_real_fixture_round_trip_accepts`).
    #[test]
    fn skeleton_dummy_does_not_synthesize_missing_sections() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let result = NovaVerifierCircuit::dummy().generate_constraints(cs.clone());
        assert!(
            matches!(result, Err(SynthesisError::Unsatisfiable)),
            "section-less dummy() must not synthesize under S2b, got {result:?}"
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
    /// Audit B-1/B-2 S2b: `dummy()` carries NO section witnesses, so it
    /// is the constraint-vacuous circuit the hole was about. Synthesis
    /// MUST now fail (`validate_structurally` → `Unsatisfiable`) — a
    /// section-less circuit can no longer produce a satisfied CS. This
    /// inversion of the old "dummy is satisfied" test IS the proof the
    /// vacuity hole is closed.
    #[test]
    fn dummy_circuit_is_unsatisfiable_missing_sections() {
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let result = NovaVerifierCircuit::dummy().generate_constraints(cs.clone());
        assert!(
            matches!(result, Err(SynthesisError::Unsatisfiable)),
            "section-less dummy() must be Unsatisfiable under S2b, got {result:?}"
        );
    }

    /// Audit B-1/B-2 S2b: the vacuity hole is closed for ANY state-
    /// vector arity — a section-less circuit is rejected before the
    /// public-input wiring is even reached, regardless of |z0|. (The
    /// canonical public-input arity, 5, is pinned by the real-fixture
    /// path: `circuit_builder::fixture_to_circuit_to_satisfied_cs` and
    /// the S6 determinism proof.)
    #[test]
    fn section_less_circuit_is_unsatisfiable_regardless_of_arity() {
        let circuit = NovaVerifierCircuit::new(
            10,
            vec![Bn254Fr::from(1u64); 4],
            vec![Bn254Fr::from(2u64); 4],
            Bn254Fr::from(0xabcdu64),
            Bn254Fr::from(0xef01u64),
        );
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let result = circuit.generate_constraints(cs.clone());
        assert!(
            matches!(result, Err(SynthesisError::Unsatisfiable)),
            "section-less circuit must be Unsatisfiable for any arity, got {result:?}"
        );
    }

    /// Audit B-1/B-2 S2b: a circuit that is well-formed at the Section 1
    /// level (num_steps, z0/zi arity) but carries NO mandatory section
    /// bindings is now REJECTED by `validate_structurally`. `new()`
    /// initialises both sections to `None`, so it can never stand in
    /// for a real prover circuit.
    #[test]
    fn validate_structurally_rejects_section_less_circuit() {
        let circuit = NovaVerifierCircuit::new(
            7,
            vec![Bn254Fr::from(1u64), Bn254Fr::from(2u64)],
            vec![Bn254Fr::from(3u64), Bn254Fr::from(4u64)],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        assert_eq!(
            circuit.validate_structurally(),
            Err(StructuralValidationError::MissingSection2),
        );
    }

    /// Audit B-1/B-2 S2b: `dummy()` (no section witnesses) is rejected
    /// by `validate_structurally`. Setup is keyed over `setup_shape()`
    /// (S2a), which DOES carry canonical sections — `dummy()` is no
    /// longer a valid setup/prove input.
    #[test]
    fn validate_structurally_rejects_dummy_missing_sections() {
        assert_eq!(
            NovaVerifierCircuit::dummy().validate_structurally(),
            Err(StructuralValidationError::MissingSection2),
        );
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
        let circuit =
            NovaVerifierCircuit::new(1, vec![], vec![], Bn254Fr::from(0u64), Bn254Fr::from(0u64));
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
            vec![
                Bn254Fr::from(1u64),
                Bn254Fr::from(2u64),
                Bn254Fr::from(3u64),
            ],
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

    /// Parallel of the num_steps / mismatched-arity wire tests for
    /// the EmptyState variant — `z0 = []` must surface as
    /// `Unsatisfiable` at the top of `generate_constraints`.
    #[test]
    fn generate_constraints_rejects_empty_state_as_unsatisfiable() {
        let circuit =
            NovaVerifierCircuit::new(5, vec![], vec![], Bn254Fr::from(0u64), Bn254Fr::from(0u64));
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let result = circuit.generate_constraints(cs.clone());
        assert!(
            matches!(result, Err(SynthesisError::Unsatisfiable)),
            "expected SynthesisError::Unsatisfiable for empty z0, got {result:?}"
        );
    }

    /// `new()` initialises both section witnesses to None — a
    /// constructor-contract pin. Post-S2a/S2b a section-less circuit is
    /// NOT a valid setup or prover input (trusted setup is keyed over
    /// `setup_shape()`, which carries canonical sections); this only
    /// asserts the raw constructor default, not provability.
    #[test]
    fn new_initialises_sections_to_none() {
        let c = NovaVerifierCircuit::new(
            1,
            vec![Bn254Fr::from(0u64)],
            vec![Bn254Fr::from(0u64)],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        assert!(c.section2.is_none(), "section2 must default to None");
        assert!(c.section3.is_none(), "section3 must default to None");
    }

    /// `dummy()` carries no section witnesses — constructor-contract
    /// pin. NOTE: post-S2a `dummy()` is NOT the trusted-setup witness
    /// (`setup_shape()` is); under S2b a section-less `dummy()` is
    /// rejected by `validate_structurally`. This only pins the raw
    /// constructor default + base shape.
    #[test]
    fn dummy_has_no_section_witnesses() {
        let d = NovaVerifierCircuit::dummy();
        assert!(d.section2.is_none());
        assert!(d.section3.is_none());
        // And the shape must match what setup expects.
        assert_eq!(d.num_steps, 1);
        assert_eq!(d.z0.len(), 1);
        assert_eq!(d.zi.len(), 1);
    }

    /// `validate_structurally`'s MismatchedArity variant carries the
    /// observed `z0_len` / `zi_len` so the caller can render a useful
    /// error. Pin both values.
    #[test]
    fn mismatched_arity_error_carries_correct_lengths() {
        let c = NovaVerifierCircuit::new(
            1,
            vec![Bn254Fr::from(0u64); 3],
            vec![Bn254Fr::from(0u64); 5],
            Bn254Fr::from(0u64),
            Bn254Fr::from(0u64),
        );
        match c.validate_structurally() {
            Err(StructuralValidationError::MismatchedArity { z0_len, zi_len }) => {
                assert_eq!(z0_len, 3);
                assert_eq!(zi_len, 5);
            }
            other => panic!("expected MismatchedArity {{ z0_len:3, zi_len:5 }}, got {other:?}"),
        }
    }

    /// `StructuralValidationError` Display impls render the diagnostic
    /// context: each variant's `to_string()` should mention the
    /// numeric or descriptive context so log lines are useful.
    #[test]
    fn structural_validation_error_displays_all_variants() {
        let zero = StructuralValidationError::NumStepsZero.to_string();
        let empty = StructuralValidationError::EmptyState.to_string();
        let mismatch = StructuralValidationError::MismatchedArity {
            z0_len: 7,
            zi_len: 9,
        }
        .to_string();
        assert!(zero.contains("num_steps"), "got: {zero}");
        assert!(empty.contains("z0"), "got: {empty}");
        assert!(
            mismatch.contains("7") && mismatch.contains("9"),
            "got: {mismatch}"
        );
    }

    /// `StructuralValidationError` Clone + PartialEq smoke (derived,
    /// but pinning here so a derive removal causes a loud failure —
    /// downstream callers may use `assert_eq!` on these errors).
    #[test]
    fn structural_validation_error_clone_and_eq() {
        let a = StructuralValidationError::MismatchedArity {
            z0_len: 1,
            zi_len: 2,
        };
        assert_eq!(a.clone(), a);
        assert_ne!(
            a,
            StructuralValidationError::MismatchedArity {
                z0_len: 1,
                zi_len: 3
            },
        );
        assert_ne!(a, StructuralValidationError::NumStepsZero);
    }

    /// Empty `z0` AND empty `zi` is still rejected as `EmptyState`,
    /// NOT as `MismatchedArity` (their lengths happen to match). Pin
    /// the dispatch precedence: EmptyState check must fire before
    /// the arity-match check.
    #[test]
    fn empty_state_takes_precedence_over_arity_match() {
        let c =
            NovaVerifierCircuit::new(1, vec![], vec![], Bn254Fr::from(0u64), Bn254Fr::from(0u64));
        assert_eq!(
            c.validate_structurally(),
            Err(StructuralValidationError::EmptyState),
        );
    }
}
