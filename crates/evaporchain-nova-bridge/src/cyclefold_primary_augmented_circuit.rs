//! B-1/B-2 EVM, option (1C) — increment 4b-α: **primary augmented
//! circuit SHELL** for the CycleFold IVC.
//!
//! # Why this is a shell, not the finished circuit
//!
//! The complete primary augmented circuit (CycleFold-style) per
//! step has to: (1) verify the previous step's RO transcript via
//! Neptune, (2) run the inner step circuit `F` (`z_{i+1} = F(z_i)`),
//! (3) absorb the CF running instance into the RO, (4) emit the
//! cross-curve scalar-mul tuple `(P, s, Q)` for the CF instance
//! circuit to attest to. (1) and (3) are the heavy pieces — multi-
//! day Neptune wiring. This shell does (2) + (4) + the public IO
//! allocation matching what the integrated harness will need; it
//! defers (1) and (3) behind an explicit `sections_wired:bool`
//! honesty flag so a caller cannot mistake this for a complete
//! augmented circuit.
//!
//! Pattern reused from `RecursionDeciderCircuit` and
//! `CycleFoldInstanceCircuit`: real load-bearing pieces live, heavy
//! constant-size pieces stay as explicit deferred stubs with the
//! flag false until wired.
//!
//! # What 4b-α delivers
//!
//! - Struct + arkworks `ConstraintSynthesizer<Bn254Fr>` impl that
//!   compiles and synthesises against a real `ConstraintSystem`.
//! - Stub step (z_{i+1} = z_i + 1, same as `TrivialIncrementCircuit`).
//! - Public IO layout matching the CycleFold IVC schema — ALL
//!   `Bn254Fr` scalars: `[pp_hash, i, z_0, z_i, z_{i+1}, cf_x_digest]`.
//!   `cf_x_digest` is a single Bn254Fr hash that binds the cross-
//!   curve tuple `(P, s, Q)` (a `Bn254Fr` digest of those values,
//!   recomputed independently on the aux side via the matching RO
//!   so the link is enforced cross-circuit). Per Sonobe
//!   `circuits.rs` L230/L280 (`FpVar::new_input(..., x.value())?
//!   .enforce_equal(&x)?`), CF-augmented IO exposes scalar digests,
//!   NOT raw curve coordinates — exposing `P.x, P.y` (Bn254Fq) as
//!   inputs of a Bn254Fr circuit is a type/architecture error
//!   (caught by the compiler at HEAD `3afabb13` on first build; see
//!   the fix commit for the full surfaced correction).
//! - The actual `(P, s, Q)` raw values are carried in the witness
//!   only (so 4b-β can hash them into `cf_x_digest`); they are NOT
//!   public.
//! - `sections_wired: bool` — false; flips to true only when the
//!   RO/fold-verification stubs become real in 4b-β.
//! - Box-measured base constraint count `cs.num_constraints()`.

use ark_bn254::{Fr as Bn254Fr, G1Affine};
use ark_r1cs_std::{
    alloc::AllocVar, eq::EqGadget, fields::fp::FpVar, fields::FieldVar,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

/// Witness for one step of the (shell) primary augmented circuit.
///
/// `step_index = i`; the step circuit advances `z_i → z_{i+1}`
/// (here stubbed as `z_{i+1} = z_i + 1`). The cross-curve scalar-
/// mul tuple `(P_step, s_step, Q_step)` is what the CF instance
/// circuit attests to ([`crate::cyclefold_instance_circuit::Cycle
/// FoldInstanceCircuit`]).
#[derive(Clone, Debug)]
pub struct PrimaryAugmentedCircuitShell {
    /// Public param digest (placeholder — RO wiring is 4b-β).
    pub pp_hash: Bn254Fr,
    /// Step counter `i`.
    pub i: Bn254Fr,
    /// Initial state `z_0` (single-element here for the stub step).
    pub z_0: Bn254Fr,
    /// Current state `z_i`.
    pub z_i: Bn254Fr,
    /// Next state `z_{i+1}` supplied by the prover. Constraint
    /// `z_{i+1} == z_i + 1` enforces consistency; a malicious
    /// prover supplying a wrong `z_i1` must be rejected.
    pub z_i1: Bn254Fr,
    /// Cross-curve scalar-mul `P_step` (BN254-G1 point) — WITNESS
    /// only (Bn254Fq coords, non-native to this Bn254Fr circuit;
    /// not exposed as public IO — public link is `cf_x_digest`).
    pub p_step: G1Affine,
    /// Cross-curve scalar-mul `s_step` (E1.scalar = Bn254Fr —
    /// the primary's folding challenge; NATIVE to this Bn254Fr
    /// circuit, NON-NATIVE on the CF aux side as
    /// `EmulatedFpVar<Bn254Fr, Bn254Fq>` —
    /// matches [`crate::cyclefold_instance_circuit::CycleFold
    /// InstanceCircuit::scalar`]) — WITNESS only.
    pub s_step: Bn254Fr,
    /// Cross-curve scalar-mul `Q_step = s_step · P_step` — WITNESS
    /// only.
    pub q_step: G1Affine,
    /// PUBLIC: Bn254Fr digest binding the cross-curve tuple
    /// `(p_step, s_step, q_step)`. Recomputed independently on the
    /// CF aux side via the matching Neptune RO; equality of the
    /// two digests is the cross-circuit binding. Stubbed as a
    /// caller-supplied value here (4b-β computes it from a
    /// real Neptune hash of the tuple components).
    pub cf_x_digest: Bn254Fr,
    /// PUBLIC: Section-R transcript hash for THIS step — Neptune
    /// hash of `[pp_hash, i, z_0, z_i, z_{i+1}, cf_x_digest]` (the
    /// natively-Fr-representable IO fields). The CF running
    /// instance absorb (Bn254Fq `u`/`x` via limb decomp) is
    /// deferred to 4b-β-4b. This value is what the NEXT step
    /// chains against; Section F (4b-β-5) will absorb it as the
    /// previous-step hash and enforce NIFS fold consistency.
    pub current_step_hash: Bn254Fr,
    /// Neptune sponge params for the in-circuit `cf_x_digest`
    /// gadget (Section C). Constructed once by the caller via
    /// `params_from_dump_path("neptune-bn256-standard.json")` and
    /// shared across IVC steps. Cloned per shell because
    /// `NeptuneParams` derives `Clone`.
    pub params: crate::neptune_permutation_gadget::NeptuneParams<Bn254Fr>,
    /// HONESTY flag: false until ALL deferred sections (R Neptune
    /// RO previous-step binding + F primary NIFS verification) are
    /// also wired. Section C (cf_x_digest) goes live in 4b-β-3 but
    /// the full shell-as-augmented-circuit needs R + F too.
    pub sections_wired: bool,
}

impl PrimaryAugmentedCircuitShell {
    /// Shell constructor (4b-α). Sets `sections_wired:false`.
    /// `cf_x_digest` is a stubbed Bn254Fr value; 4b-β will compute
    /// it from a real Neptune hash of `(p_step, s_step, q_step)`.
    pub fn new(
        pp_hash: Bn254Fr,
        i: Bn254Fr,
        z_0: Bn254Fr,
        z_i: Bn254Fr,
        z_i1: Bn254Fr,
        p_step: G1Affine,
        s_step: Bn254Fr,
        q_step: G1Affine,
        cf_x_digest: Bn254Fr,
        current_step_hash: Bn254Fr,
        params: crate::neptune_permutation_gadget::NeptuneParams<Bn254Fr>,
    ) -> Self {
        Self {
            pp_hash,
            i,
            z_0,
            z_i,
            z_i1,
            p_step,
            s_step,
            q_step,
            cf_x_digest,
            current_step_hash,
            params,
            sections_wired: false,
        }
    }
}

impl ConstraintSynthesizer<Bn254Fr> for PrimaryAugmentedCircuitShell {
    fn generate_constraints(
        self,
        cs: ConstraintSystemRef<Bn254Fr>,
    ) -> Result<(), SynthesisError> {
        // ── Public inputs (instance `x`) ──────────────────────────
        // Pinned schema; 4b-β extends with cf-running-instance fields
        // + folds them into the Neptune sponge.
        let pp_hash_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.pp_hash))?;
        let i_var = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.i))?;
        let z_0_var = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.z_0))?;
        let z_i_var = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.z_i))?;
        // z_{i+1} supplied by the prover (separate field from z_i).
        // The step constraint below enforces consistency.
        let z_i1_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.z_i1))?;

        // Cross-curve tuple binding — exposed as a SINGLE Bn254Fr
        // digest, NOT raw curve coords (Bn254Fq, foreign field).
        let cf_x_digest_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.cf_x_digest))?;

        // ── Section C [LIVE since 4b-β-3] ─────────────────────────
        // Allocate (P, s, Q) as witnesses; in-circuit `cf_x_digest`
        // recomputed from them via `enforce_cf_x_digest`; enforce
        // it equals the public `cf_x_digest_var`. A malicious
        // prover supplying an inconsistent (P, s, Q, cf_x_digest)
        // is rejected here, before Sections R/F reach for the
        // tuple. (R and F still deferred — sections_wired stays
        // false until those are also wired.)
        use ark_r1cs_std::fields::emulated_fp::EmulatedFpVar;
        let p_x_var = EmulatedFpVar::<ark_bn254::Fq, Bn254Fr>::new_witness(
            cs.clone(),
            || Ok(self.p_step.x),
        )?;
        let p_y_var = EmulatedFpVar::<ark_bn254::Fq, Bn254Fr>::new_witness(
            cs.clone(),
            || Ok(self.p_step.y),
        )?;
        let s_step_var =
            FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(self.s_step))?;
        let q_x_var = EmulatedFpVar::<ark_bn254::Fq, Bn254Fr>::new_witness(
            cs.clone(),
            || Ok(self.q_step.x),
        )?;
        let q_y_var = EmulatedFpVar::<ark_bn254::Fq, Bn254Fr>::new_witness(
            cs.clone(),
            || Ok(self.q_step.y),
        )?;

        let computed_digest = crate::cyclefold_cf_x_digest::enforce_cf_x_digest(
            cs.clone(),
            &p_x_var,
            &p_y_var,
            &s_step_var,
            &q_x_var,
            &q_y_var,
            &self.params,
        )?;
        computed_digest.enforce_equal(&cf_x_digest_var)?;

        // ── Section R [LIVE (stub-form) since 4b-β-4] ────────────
        // current_step_hash = Neptune([pp_hash, i, z_0, z_i, z_{i+1},
        // cf_x_digest]). Absorbs only the natively-Fr-representable
        // IO fields; CF running instance absorb (Bn254Fq u/x via
        // limb decomp) deferred to 4b-β-4b. This hash is what the
        // next step's Section F (4b-β-5) will absorb as
        // previous-step-hash and verify NIFS fold consistency
        // against. Same Neptune infrastructure as Section C.
        let current_step_hash_var = FpVar::<Bn254Fr>::new_input(
            cs.clone(),
            || Ok(self.current_step_hash),
        )?;
        let r_absorb: Vec<FpVar<Bn254Fr>> = vec![
            pp_hash_var.clone(),
            i_var.clone(),
            z_0_var.clone(),
            z_i_var.clone(),
            z_i1_var.clone(),
            cf_x_digest_var.clone(),
        ];
        let computed_step_hash =
            crate::section2_gadget::enforce_neptune_sponge_primary(
                cs.clone(),
                &self.params,
                &r_absorb,
            )?;
        // Apply 250-bit truncation to match the native helper's
        // squeeze (NUM_HASH_BITS=250), same pattern as Section C.
        use ark_r1cs_std::boolean::Boolean;
        use ark_r1cs_std::convert::ToBitsGadget;
        let raw_bits = computed_step_hash.to_bits_le()?;
        let trunc_bits = &raw_bits[..250usize.min(raw_bits.len())];
        let truncated_step_hash = Boolean::le_bits_to_fp(trunc_bits)?;
        truncated_step_hash.enforce_equal(&current_step_hash_var)?;

        // ── Stub step: z_{i+1} = z_i + 1 ──────────────────────────
        // Real step circuit `F` plugs in here in 4b-β.
        let computed_next = &z_i_var + FpVar::<Bn254Fr>::constant(Bn254Fr::from(1u64));
        computed_next.enforce_equal(&z_i1_var)?;

        // ── DEFERRED STUBS (4b-β) ─────────────────────────────────
        // Section R: Neptune RO transcript binding (incoming
        //   instance hash matches absorbed values).
        // Section F: Primary NIFS verification (the fold relation
        //   between previous primary instance and incoming step).
        // Section C: CF running instance absorption + tuple binding
        //   (Q == s · P at the primary level, mirroring what the CF
        //   aux side enforces — redundant in-circuit but pins the
        //   public output to the witness).
        //
        // While these are stubs, `sections_wired:false` records the
        // gap so the integrated harness cannot ship a forged
        // primary instance through this circuit.

        // Use z_0_var to suppress unused-var warning while keeping
        // the public input live (it'll be absorbed in Section R).
        let _ = z_0_var;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::{AffineRepr, CurveGroup};
    use ark_ff::UniformRand;
    use ark_relations::r1cs::ConstraintSystem;
    use ark_std::test_rng;

    /// Native helper mirroring the in-circuit Section R hash —
    /// `neptune_hash_primary([pp_hash, i, z_0, z_i, z_{i+1},
    /// cf_x_digest])` with the same 250-bit truncation the
    /// in-circuit gadget applies.
    fn compute_current_step_hash_native(
        pp_hash: Bn254Fr,
        i: Bn254Fr,
        z_0: Bn254Fr,
        z_i: Bn254Fr,
        z_i1: Bn254Fr,
        cf_x_digest: Bn254Fr,
    ) -> Bn254Fr {
        use crate::neptune_reference::neptune_hash_primary;
        use crate::scalar_adapter::{ark_fr_to_primary, primary_to_ark_fr};
        let absorbed = [pp_hash, i, z_0, z_i, z_i1, cf_x_digest]
            .map(ark_fr_to_primary);
        primary_to_ark_fr(neptune_hash_primary(&absorbed))
    }

    fn consistent_step() -> PrimaryAugmentedCircuitShell {
        let mut rng = test_rng();
        let p = G1Affine::generator();
        let s = Bn254Fr::rand(&mut rng);
        let q = (ark_bn254::G1Projective::from(p) * s).into_affine();
        // Section C: compute the REAL cf_x_digest via the 4b-β-1
        // oracle so the binding is satisfiable.
        let cf_x_digest =
            crate::cyclefold_cf_x_digest::compute_cf_x_digest_native(p, s, q);
        let pp_hash = Bn254Fr::from(42u64);
        let i = Bn254Fr::from(0u64);
        let z_0 = Bn254Fr::from(0u64);
        let z_i = Bn254Fr::from(0u64);
        let z_i1 = Bn254Fr::from(1u64);
        // Section R: compute the REAL current_step_hash so its
        // binding is satisfiable too.
        let current_step_hash = compute_current_step_hash_native(
            pp_hash, i, z_0, z_i, z_i1, cf_x_digest,
        );
        let params = crate::neptune_permutation_gadget::params_from_dump_path(
            concat!(env!("CARGO_MANIFEST_DIR"), "/neptune-bn256-standard.json"),
        )
        .expect("load neptune params from crate-relative dump");
        PrimaryAugmentedCircuitShell::new(
            pp_hash,
            i,
            z_0,
            z_i,
            z_i1,
            p,
            s,
            q,
            cf_x_digest,
            current_step_hash,
            params,
        )
    }

    /// POSITIVE: shell synthesises and CS is satisfied. Stub step
    /// `z_{i+1} = z_i + 1` is the only enforced relation; tuple
    /// (P, s, Q) is public IO but not yet bound by Q = s·P in-
    /// circuit (that's Section C, 4b-β; the CF aux side enforces
    /// it independently).
    #[test]
    fn shell_synthesises_and_cs_is_satisfied() {
        let circuit = consistent_step();
        assert!(!circuit.sections_wired, "shell must have sections_wired=false");
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "shell with consistent z must satisfy CS"
        );
    }

    /// NEGATIVE: wrong z_{i+1} ⇒ CS UNSAT (the step relation IS
    /// enforced even though tuple binding is deferred). Confirms
    /// the one live constraint is non-vacuous.
    #[test]
    fn shell_wrong_next_z_breaks_cs() {
        let mut circuit = consistent_step();
        // Tamper z_i1 (the prover-supplied next state) so the step
        // constraint `z_i1 == z_i + 1` no longer holds.
        circuit.z_i1 = Bn254Fr::from(99u64);
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "z_i ≠ z_{{i+1}}-1 MUST break the stub step constraint"
        );
    }

    /// SECTION C NON-VACUITY: tamper the witnessed `p_step` so it
    /// no longer matches the public `cf_x_digest` → in-circuit
    /// gadget computes a different digest → `enforce_equal` fails
    /// → CS UNSAT. Proves the wired Section C binding actually
    /// constrains `(P, s, Q)` against the public IO, not vacuous.
    #[test]
    fn shell_section_c_wrong_p_breaks_cs() {
        let mut c = consistent_step();
        // Tamper P only — gadget digest will differ from the
        // public cf_x_digest (which was computed from the
        // ORIGINAL P).
        let g = ark_bn254::G1Projective::from(G1Affine::generator());
        c.p_step = (ark_bn254::G1Projective::from(c.p_step) + g).into_affine();
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        c.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "tampered P MUST break Section C's cf_x_digest binding"
        );
    }

    /// SECTION C NON-VACUITY (mirror): tamper `s_step` → digest
    /// mismatch → CS UNSAT. Covers the scalar component of the
    /// binding (different break path than tampering a point coord).
    #[test]
    fn shell_section_c_wrong_s_breaks_cs() {
        let mut c = consistent_step();
        c.s_step = c.s_step + Bn254Fr::from(1u64);
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        c.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "tampered s MUST break Section C's cf_x_digest binding"
        );
    }

    /// SECTION R NON-VACUITY: tamper the absorbed `i` (step
    /// counter) → in-circuit transcript hash differs from the
    /// public `current_step_hash` → CS UNSAT. Proves Section R's
    /// binding actually constrains the public IO + cf_x_digest
    /// chain through the Neptune sponge.
    #[test]
    fn shell_section_r_wrong_i_breaks_cs() {
        let mut c = consistent_step();
        // Tamper i only — gadget recomputes hash with the WRONG i,
        // but public `current_step_hash` was computed with the
        // original i = 0.
        c.i = Bn254Fr::from(7u64);
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        c.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "tampered i MUST break Section R's transcript binding"
        );
    }

    /// SECTION R NON-VACUITY (mirror): tamper `pp_hash` → hash
    /// chain breaks → CS UNSAT. Different break path than
    /// tampering i; covers absorbing-position 0 of the Neptune
    /// sponge.
    #[test]
    fn shell_section_r_wrong_pp_hash_breaks_cs() {
        let mut c = consistent_step();
        c.pp_hash = c.pp_hash + Bn254Fr::from(1u64);
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        c.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "tampered pp_hash MUST break Section R's transcript binding"
        );
    }

    /// SIZE PROBE: base cons of the shell (public IO + step +
    /// Section C cf_x_digest + Section R transcript hash). 4b-β-3
    /// baseline 6,267 cons; Section R adds another Neptune sponge
    /// (6 absorbs + permute + squeeze + 250-bit truncation) for
    /// ~few thousand more.
    #[test]
    fn shell_size_probe() {
        let circuit = consistent_step();
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).expect("synthesis");
        assert!(cs.is_satisfied().unwrap());
        let n_cons = cs.num_constraints();
        let n_wit = cs.num_witness_variables();
        let n_inst = cs.num_instance_variables();
        eprintln!(
            "PRIMARY_SHELL_PROBE cs.num_constraints={n_cons} \
             cs.num_witness={n_wit} cs.num_instance={n_inst}"
        );
        // Sanity: stub step + emulated s_step + public IO is
        // non-trivial (catches a regression where the stub got
        // elided). Upper bound is loose; the real budget belongs
        // to 4b-β's Sections R/F/C.
        // With Section C wired (limb decomp + Neptune sponge +
        // 250-bit truncation), expect ~thousands of cons. Lower
        // bound bumped: a regression elision would be detected if
        // we see <500 (essentially "Section C disappeared").
        assert!(
            n_cons >= 500,
            "shell unexpectedly small after Section C wiring: {n_cons}"
        );
        // Upper bound bumped generously; tighter once 4b-β-5
        // Section F lands.
        assert!(n_cons < 300_000, "shell unexpectedly large: {n_cons}");
    }
}
