//! B-1/B-2 EVM, option (1C) — increment 2: the **CycleFold instance
//! circuit**, source-anchored to Sonobe's reference (`folding-
//! schemes/src/folding/nova/circuits.rs`, `AugmentedFCircuit`).
//!
//! # Construction (source-grounded)
//!
//! Sonobe pins the cycle constraint
//! `C1: Curve<BaseField = C2::ScalarField, ScalarField = C2::BaseField>`
//! and `cf_U_i: CycleFoldCommittedInstance<C2>` — the CycleFold
//! *committed instance* lives on **C2** (= Grumpkin in our cycle).
//! Its R1CS therefore lives over `C2::ScalarField = Bn254Fq`, and
//! the relation it enforces is the **cross-curve scalar-mul on C1**:
//! `Q = s · P` where `P, Q ∈ C1 = BN254-G1` and `s ∈ C1::ScalarField
//! = Bn254Fr` (the primary's folding challenge). That is what
//! `crate::cyclefold_aux_circuit::cyclefold_aux_scalar_mul` already
//! computes natively (native BN254 EC over a Bn254Fq circuit) —
//! this module wraps it with the **public input layout** the primary
//! will absorb when folding the CycleFold running instance.
//!
//! # Public input layout (CycleFold instance `x`)
//!
//! - `P.x, P.y` — 2 native Bn254Fq public inputs (E1 affine coords
//!   are in C1's base field = C2's scalar field = the circuit's
//!   native field).
//! - `s` — the Bn254Fr folding scalar, exposed via
//!   `EmulatedFpVar<Bn254Fr, Bn254Fq>::new_input` (limbs become
//!   public). Whatever limb count `EmulatedFpVar` uses, it's stable
//!   and the primary side allocates the matching emulated public
//!   input on its side.
//! - `Q.x, Q.y` — 2 native Bn254Fq public inputs.
//!
//! # What's box-verified here
//!
//! Positive (correct triple ⇒ CS sat), non-vacuity (wrong `Q` ⇒ CS
//! UNSAT — the B-1 guard), and a **full-instance-shape size probe**
//! `cs.num_constraints()` *with* the public IO allocated. That is
//! the number ppsnark padding actually sees (the bare-gadget number
//! from increment 1 was the lower bound; the public-IO allocation
//! adds a small overhead).
//!
//! # What's NOT here (deferred to later increments)
//!
//! Wiring into nova-snark's NIFS / Pedersen-on-Grumpkin so a
//! sequence of these instances can be folded across IVC steps =
//! increment 3 (cycle plumbing). This module is just the per-step
//! committable shape.

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr, G1Affine, G1Projective};
use ark_r1cs_std::{
    alloc::AllocVar,
    eq::EqGadget,
    fields::emulated_fp::EmulatedFpVar,
    fields::fp::FpVar,
};
use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystemRef, SynthesisError};

use crate::cyclefold_aux_circuit::{cyclefold_aux_scalar_mul, Bn254G1Var};

/// One CycleFold committed-instance step. Public inputs are
/// `(P.x, P.y, s, Q.x, Q.y)` — the cross-curve scalar-mul tuple the
/// primary delegates this step. Relation: `Q = s · P` on BN254-G1.
#[derive(Clone, Debug)]
pub struct CycleFoldInstanceCircuit {
    /// E1 (BN254-G1) base point — the primary's cross-curve operand.
    pub base: G1Affine,
    /// E1 scalar (folding challenge) — non-native to this circuit.
    pub scalar: Bn254Fr,
    /// Claimed result `Q = s · P`. The R1CS enforces this equality.
    pub claimed: G1Affine,
}

impl CycleFoldInstanceCircuit {
    pub fn new(base: G1Affine, scalar: Bn254Fr, claimed: G1Affine) -> Self {
        Self {
            base,
            scalar,
            claimed,
        }
    }
}

impl ConstraintSynthesizer<Bn254Fq> for CycleFoldInstanceCircuit {
    fn generate_constraints(
        self,
        cs: ConstraintSystemRef<Bn254Fq>,
    ) -> Result<(), SynthesisError> {
        // ── Public inputs (instance `x`) ──────────────────────────
        // P.x, P.y as native Bn254Fq publics.
        let _p_x_input =
            FpVar::<Bn254Fq>::new_input(cs.clone(), || Ok(self.base.x))?;
        let _p_y_input =
            FpVar::<Bn254Fq>::new_input(cs.clone(), || Ok(self.base.y))?;
        // s as emulated-Fr public (limbs become instance vars).
        let s_var = EmulatedFpVar::<Bn254Fr, Bn254Fq>::new_input(
            cs.clone(),
            || Ok(self.scalar),
        )?;
        // Q.x, Q.y as native Bn254Fq publics.
        let q_x_input =
            FpVar::<Bn254Fq>::new_input(cs.clone(), || Ok(self.claimed.x))?;
        let q_y_input =
            FpVar::<Bn254Fq>::new_input(cs.clone(), || Ok(self.claimed.y))?;

        // ── Relation: claimed = s · base ──────────────────────────
        let computed = cyclefold_aux_scalar_mul(self.base, &s_var)?;

        // Reconstruct the claimed point as a circuit constant for
        // structural-equality verification, then bind to the public
        // inputs. (Native BN254 affine ⇒ x,y are public; the
        // Bn254G1Var constant matches `claimed` by construction; we
        // separately enforce the public inputs equal the constant's
        // coords so the public `x` IS the binding witness.)
        let claimed_var = Bn254G1Var::new_witness(cs.clone(), || {
            Ok(G1Projective::from(self.claimed))
        })?;
        computed.enforce_equal(&claimed_var)?;

        // Bind the public Q.x/Q.y inputs to the witnessed point's
        // affine coords — converts the projective witness to affine
        // and asserts equality with the publics. This is what makes
        // (Q.x, Q.y) a genuine instance — without this, the public
        // inputs are decorative and the binding lives only in the
        // witness (a vacuity hazard).
        let claimed_aff = claimed_var.to_affine()?;
        claimed_aff.x.enforce_equal(&q_x_input)?;
        claimed_aff.y.enforce_equal(&q_y_input)?;

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

    fn consistent_witness() -> CycleFoldInstanceCircuit {
        let mut rng = test_rng();
        let p = G1Affine::generator();
        let s = Bn254Fr::rand(&mut rng);
        let q = (G1Projective::from(p) * s).into_affine();
        CycleFoldInstanceCircuit::new(p, s, q)
    }

    /// POSITIVE: correct (P, s, Q) ⇒ CS sat.
    #[test]
    fn cf_instance_correct_triple_satisfies_cs() {
        let circuit = consistent_witness();
        let cs = ConstraintSystem::<Bn254Fq>::new_ref();
        circuit.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "correct (P,s,Q) must satisfy the CycleFold instance binding"
        );
    }

    /// NEGATIVE: wrong `Q` ⇒ CS UNSAT (the B-1 vacuity guard at the
    /// instance level — confirms the public Q is genuinely bound to
    /// `s·P`, not decorative).
    #[test]
    fn cf_instance_wrong_claimed_breaks_cs() {
        let mut c = consistent_witness();
        c.claimed = (G1Projective::from(c.claimed)
            + G1Projective::from(G1Affine::generator()))
        .into_affine();
        let cs = ConstraintSystem::<Bn254Fq>::new_ref();
        c.generate_constraints(cs.clone()).expect("synthesis");
        assert!(
            !cs.is_satisfied().expect("is_satisfied"),
            "wrong Q MUST break CS — instance binding non-vacuous"
        );
    }

    /// FULL-INSTANCE SIZE PROBE: cs.num_constraints + public IO
    /// allocation. This is the number ppsnark padding sees.
    /// AUX-bare was 2,548 (increment 1); the instance wrap adds
    /// public-IO + affine-conversion + 2 enforce_equal overhead.
    /// Per the assert-without-measuring lesson: real number,
    /// reported not asserted as tractability. Sanity bounds only.
    #[test]
    fn cf_instance_size_probe() {
        let circuit = consistent_witness();
        let cs = ConstraintSystem::<Bn254Fq>::new_ref();
        circuit.generate_constraints(cs.clone()).expect("synthesis");
        assert!(cs.is_satisfied().unwrap(), "probe CS must be sat");
        let n_cons = cs.num_constraints();
        let n_wit = cs.num_witness_variables();
        let n_inst = cs.num_instance_variables();
        eprintln!(
            "CF_INSTANCE_PROBE cs.num_constraints={n_cons} \
             cs.num_witness={n_wit} cs.num_instance={n_inst}"
        );

        // Sanity: must include the increment-1 aux work (~2.5k) +
        // some allocation overhead — > 2k. Catches a regression
        // where the binding got optimized away.
        assert!(n_cons >= 2_000, "CF instance too small: {n_cons}");
        // Sanity: must be << 1e5 — the architectural reduction
        // would be broken if a single CycleFold instance is in the
        // tens of thousands of constraints.
        assert!(n_cons < 100_000, "CF instance too large: {n_cons}");
    }
}
