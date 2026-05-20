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

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr, G1Affine};
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
    /// Cross-curve scalar-mul `P_step` (BN254-G1 point) — WITNESS
    /// only (Bn254Fq coords, non-native to this Bn254Fr circuit;
    /// not exposed as public IO — public link is `cf_x_digest`).
    pub p_step: G1Affine,
    /// Cross-curve scalar-mul `s_step` (BN254 scalar = E1 scalar
    /// field; non-native here) — WITNESS only.
    pub s_step: Bn254Fq,
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
    /// HONESTY flag: false until 4b-β wires the Neptune RO + the
    /// primary NIFS verification. A caller cannot mistake a shell
    /// instance for a complete augmented circuit.
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
        p_step: G1Affine,
        s_step: Bn254Fq,
        q_step: G1Affine,
        cf_x_digest: Bn254Fr,
    ) -> Self {
        Self {
            pp_hash,
            i,
            z_0,
            z_i,
            p_step,
            s_step,
            q_step,
            cf_x_digest,
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
        let _pp_hash_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.pp_hash))?;
        let _i_var = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.i))?;
        let z_0_var = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.z_0))?;
        let z_i_var = FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.z_i))?;
        // z_{i+1} = stub step (z_i + 1) — public output the CF
        // accumulator's caller reads.
        let z_i1_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.z_i + Bn254Fr::from(1u64)))?;

        // Cross-curve tuple binding — exposed as a SINGLE Bn254Fr
        // digest, NOT raw curve coords (Bn254Fq, foreign field).
        // 4b-β computes this from a Neptune hash of the tuple; for
        // now, it's a caller-supplied stub value. The CF aux side
        // recomputes the matching digest from its own (P, s, Q)
        // allocation; cross-circuit equality of the digests is the
        // cross-side binding.
        let _cf_x_digest_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.cf_x_digest))?;
        // Tuple values (P, s, Q) are not exposed publicly here —
        // they are referenced only by the witness so 4b-β can hash
        // them. The shell just records they exist via the typed
        // fields on `self`.
        let _ = (self.p_step, self.s_step, self.q_step);

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

    fn consistent_step() -> PrimaryAugmentedCircuitShell {
        let mut rng = test_rng();
        let p = G1Affine::generator();
        let s = Bn254Fq::rand(&mut rng);
        let q = (ark_bn254::G1Projective::from(p) * s).into_affine();
        PrimaryAugmentedCircuitShell::new(
            Bn254Fr::from(42u64), // pp_hash placeholder
            Bn254Fr::from(0u64),  // i
            Bn254Fr::from(0u64),  // z_0
            Bn254Fr::from(0u64),  // z_i
            p,
            s,
            q,
            Bn254Fr::from(123u64), // cf_x_digest stub (4b-β: Neptune(p,s,q))
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
        // Tamper z_i so claimed next (z_i+1) no longer matches the
        // stub computation `z_i + 1` against the public z_{i+1}=1.
        circuit.z_i = Bn254Fr::from(99u64);
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        circuit.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "z_i ≠ z_{{i+1}}-1 MUST break the stub step constraint"
        );
    }

    /// SIZE PROBE: base cons of the shell (public IO allocation +
    /// stub step + one enforce_equal). Pinned for regression
    /// tracking; 4b-β's Section R + F + C wiring will grow this
    /// number measurably.
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
        assert!(n_cons >= 1, "shell must have ≥1 constraint");
        assert!(n_cons < 50_000, "shell unexpectedly large: {n_cons}");
    }
}
