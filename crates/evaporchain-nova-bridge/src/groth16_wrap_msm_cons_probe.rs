//! B-1/B-2 EVM, option (1C) — increment (d)-1: **constraint-count
//! probe** for the dominant cost of the planned Groth16-wrap
//! circuit (the secondary IPA's `ck_hat` MSM).
//!
//! # Purpose
//!
//! Per `B1_B2_AUDIT_DOSSIER.md` §7, the 1C remaining mainnet work
//! starts with building a CompressedSNARK-verifier circuit (so
//! `groth16_wrapper.rs` can be re-pointed off the dead
//! `NovaVerifierCircuit`). Step 3 of that chain is a constraint-
//! count probe BEFORE attempting Groth16 setup — applying the same
//! discipline that the D.3 measurement applied to the old raw-S4b
//! path. This module is that probe for the dominant component.
//!
//! # What is measured
//!
//! `pedersen_msm_grumpkin(scalars, bases, blind, h)` — the
//! in-circuit Grumpkin-side Pedersen MSM gadget from
//! `s4_msm_gadget`. This is what the Groth16-wrap circuit would
//! call to verify the secondary IPA's `ck_hat = ⟨s, ck⟩` MSM
//! natively (Grumpkin EC over BN254-Fr; the curve cycle makes this
//! NATIVE — no foreign field for the EC, scalars are non-native
//! BN254-Fq via `EmulatedFpVar`).
//!
//! # Falsifier
//!
//! Per-base constraint cost at n=16 must be ≤ ~5,000 cons. The
//! Groth16-wrap circuit's MSM-portion at n_aux=16,384 = per_base ×
//! 16,384. At 5,000 cons/base that's ~80M cons (tractable on the
//! Mini cluster with patience). At 50,000 cons/base it's 800M
//! cons (Groth16 setup memory wall on 128 GB RAM).
//!
//! If per-base ≥ 50k cons, the Groth16-wrap circuit is architectural
//! dead-end — falls back to fallback ladder §6.

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr};
use ark_ec::CurveGroup;
use ark_ff::UniformRand;
use ark_r1cs_std::{alloc::AllocVar, fields::emulated_fp::EmulatedFpVar};
use ark_relations::r1cs::ConstraintSystem;
use ark_std::test_rng;

use crate::grumpkin_config::GrumpkinConfig;
use crate::s4_msm_gadget::pedersen_msm_grumpkin;

use ark_ec::short_weierstrass::Affine;

pub struct MsmConsMeasurement {
    pub n: usize,
    pub num_constraints: usize,
    pub num_witness_vars: usize,
    pub num_instance_vars: usize,
}

/// Build a CS, allocate `n` scalars + bases, call
/// `pedersen_msm_grumpkin`, report constraint count. NO proof — just
/// the cs.num_constraints() probe (fast).
pub fn measure_grumpkin_msm_cons(n: usize) -> MsmConsMeasurement {
    let mut rng = test_rng();
    let cs = ConstraintSystem::<Bn254Fr>::new_ref();

    // Synthesize n random Grumpkin bases (constants in the circuit).
    let g = <ark_grumpkin::Projective as ark_std::Zero>::zero();
    let g_aff: Affine<GrumpkinConfig> =
        ark_grumpkin::Affine::generator();
    let mut bases = Vec::with_capacity(n);
    let mut cur = ark_grumpkin::Projective::from(g_aff);
    for _ in 0..n {
        bases.push(cur.into_affine());
        cur += ark_grumpkin::Projective::from(g_aff);
    }
    let h = (cur + ark_grumpkin::Projective::from(g_aff)).into_affine();

    // Allocate n random non-native scalars (Grumpkin scalar field = Bn254Fq).
    let scalars: Vec<EmulatedFpVar<Bn254Fq, Bn254Fr>> = (0..n)
        .map(|_| {
            EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_witness(cs.clone(), || {
                Ok(Bn254Fq::rand(&mut rng))
            })
            .expect("scalar alloc")
        })
        .collect();
    let blind = EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_witness(cs.clone(), || {
        Ok(Bn254Fq::rand(&mut rng))
    })
    .expect("blind alloc");

    let _ = pedersen_msm_grumpkin(&scalars, &bases, &blind, h)
        .expect("pedersen_msm_grumpkin");

    let _ = g; // silence unused

    MsmConsMeasurement {
        n,
        num_constraints: cs.num_constraints(),
        num_witness_vars: cs.num_witness_variables(),
        num_instance_vars: cs.num_instance_variables(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (d)-1 PROBE: scan small n, report cons per base, extrapolate
    /// to n_aux=16,384.
    #[test]
    #[ignore = "(d)-1 probe: synthesises n MSM terms, measures cs.num_constraints"]
    fn groth16_wrap_msm_cons_scaling() {
        let ns = [1usize, 2, 4, 8, 16];
        let mut results = Vec::new();
        for &n in &ns {
            let m = measure_grumpkin_msm_cons(n);
            println!(
                "MSM_CONS n={} cons={} witness={} instance={} per_base={}",
                m.n,
                m.num_constraints,
                m.num_witness_vars,
                m.num_instance_vars,
                if m.n > 0 { m.num_constraints / m.n } else { 0 }
            );
            results.push(m);
        }

        // Linear regression on the upper-end (avoid overhead-
        // dominated small n).
        let big = &results[results.len() - 1];
        let small = &results[1]; // n=2
        let per_base = (big.num_constraints.saturating_sub(small.num_constraints))
            / big.n.saturating_sub(small.n);
        let fixed_overhead =
            big.num_constraints.saturating_sub(per_base * big.n);

        println!(
            "MSM_CONS_MODEL per_base_cons={} fixed_overhead={} ",
            per_base, fixed_overhead
        );

        // Extrapolation to n_aux=16,384 (real CycleFold IPA size).
        let n_aux: usize = 16_384;
        let pred = fixed_overhead + per_base * n_aux;
        println!(
            "MSM_CONS_EXTRAPOLATION n_aux={} predicted_cons={} predicted_M={}",
            n_aux,
            pred,
            pred / 1_000_000
        );

        // Falsifier: per_base ≥ 50,000 cons ⇒ Groth16-wrap MSM
        // portion at n_aux=16,384 exceeds 800M cons (memory wall on
        // 128 GB). Below that, tractable.
        println!(
            "FALSIFIER_CHECK per_base={} threshold=50000 \
             groth16_wrap_tractable={}",
            per_base,
            per_base < 50_000
        );
    }
}
