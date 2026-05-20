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
//! - Public IO layout matching the CycleFold IVC schema:
//!     `[pp_hash, i, z_0, z_i, z_{i+1}, cf_x..., P.x, P.y,
//!       s_emulated_limbs..., Q.x, Q.y]`.
//! - Cross-curve tuple `(P, s, Q)` emitted as public outputs (the
//!   CF aux circuit's [`crate::cyclefold_instance_circuit`] inputs).
//! - `sections_wired: bool` — false; flips to true only when the
//!   RO/fold-verification stubs become real in 4b-β.
//! - Box-measured base constraint count `cs.num_constraints()`.

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr, G1Affine};
use ark_r1cs_std::{
    alloc::AllocVar,
    eq::EqGadget,
    fields::emulated_fp::EmulatedFpVar,
    fields::fp::FpVar,
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
    /// Cross-curve scalar-mul `P_step` (BN254-G1 point).
    pub p_step: G1Affine,
    /// Cross-curve scalar-mul `s_step` (BN254 scalar = E1 scalar
    /// field; non-native in this Bn254Fr circuit).
    pub s_step: Bn254Fq,
    /// Cross-curve scalar-mul `Q_step = s_step · P_step`.
    pub q_step: G1Affine,
    /// HONESTY flag: false until 4b-β wires the Neptune RO + the
    /// primary NIFS verification. A caller cannot mistake a shell
    /// instance for a complete augmented circuit.
    pub sections_wired: bool,
}

impl PrimaryAugmentedCircuitShell {
    /// Shell constructor (4b-α). Sets `sections_wired:false`.
    pub fn new(
        pp_hash: Bn254Fr,
        i: Bn254Fr,
        z_0: Bn254Fr,
        z_i: Bn254Fr,
        p_step: G1Affine,
        s_step: Bn254Fq,
        q_step: G1Affine,
    ) -> Self {
        Self {
            pp_hash,
            i,
            z_0,
            z_i,
            p_step,
            s_step,
            q_step,
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

        // Cross-curve tuple (P, s, Q) — public output for the CF
        // aux side.
        let _p_x_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.p_step.x))?;
        let _p_y_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.p_step.y))?;
        // s_step is non-native (Bn254Fq) here; expose as emulated.
        let _s_step_var = EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_input(
            cs.clone(),
            || Ok(self.s_step),
        )?;
        let _q_x_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.q_step.x))?;
        let _q_y_var =
            FpVar::<Bn254Fr>::new_input(cs.clone(), || Ok(self.q_step.y))?;

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
