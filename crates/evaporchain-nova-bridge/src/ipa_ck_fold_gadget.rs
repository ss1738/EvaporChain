//! B-1/B-2 EVM option (2): the **recursive-fold** form of the
//! secondary IPA `ck_hat`, and a D.3-style probe deciding whether it
//! is cheaper in-circuit than the flat `pedersen_msm_grumpkin`
//! (measured 2,533 cons / base-term).
//!
//! `nova-snark`'s prover folds the commitment key in `log₂ n` rounds
//! (`pedersen.rs::fold` L487, weights `(r⁻¹, r)`):
//! `ck'[i] = r⁻¹·ck_lo[i] + r·ck_hi[i]`, collapsing `ck` to the one
//! point `ck_hat`. A prior flow turn *asserted* this fold is a
//! "~10-100× lever" over the flat size-`n` MSM. Honest structural
//! analysis says the opposite: total scalar-muls ≈ `n/2 + n/4 + … ≈
//! n` **pairs** ⇒ ~2n scalar-muls, and rounds ≥ 1 act on
//! **non-constant** points (variable-base, costlier than the flat
//! MSM's constant bases). So the fold looks ~2× *worse*. Per the
//! no-overhype / cheapest-falsifying-test discipline this module
//! does NOT assert that — it builds the gadget and **measures** it
//! against the flat 2,533/term baseline. The probe decides: keep
//! option (2), or escalate to flow option (1) CycleFold / (3)
//! native-Solidity.

use crate::grumpkin_config::GrumpkinConfig;
use crate::s4_msm_gadget::GrumpkinVar;
use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr};
use ark_ec::short_weierstrass::{Affine, Projective};
use ark_r1cs_std::{
    convert::ToBitsGadget,
    eq::EqGadget,
    fields::emulated_fp::EmulatedFpVar,
    fields::FieldVar,
    groups::{curves::short_weierstrass::ProjectiveVar, CurveVar},
};
use ark_relations::gr1cs::SynthesisError;

/// In-circuit recursive fold of a constant commitment key `ck` into
/// the single point `ck_hat`, exactly as `nova-snark`'s prover does
/// (`pedersen.rs::fold`, weights `(r⁻¹, r)`; `ipa_pc` prove loop,
/// challenges in round order `r[0]..r[L-1]`).
///
/// `r` / `r_inv` are the per-round non-native (Grumpkin-scalar =
/// BN254-Fq) challenges and their inverses; `r·r⁻¹ = 1` is enforced
/// per round (cheap vs the scalar-muls). `ck.len()` must be
/// `2^{r.len()}`.
pub fn tensor_fold_ck_hat(
    ck: &[Affine<GrumpkinConfig>],
    r: &[EmulatedFpVar<Bn254Fq, Bn254Fr>],
    r_inv: &[EmulatedFpVar<Bn254Fq, Bn254Fr>],
) -> Result<GrumpkinVar, SynthesisError> {
    assert_eq!(r.len(), r_inv.len(), "r / r_inv length mismatch");
    assert_eq!(
        ck.len(),
        1usize << r.len(),
        "ck.len() must be 2^rounds"
    );

    let mut cur: Vec<GrumpkinVar> = ck
        .iter()
        .map(|b| GrumpkinVar::constant(Projective::from(*b)))
        .collect();

    let one = EmulatedFpVar::<Bn254Fq, Bn254Fr>::one();
    for round in 0..r.len() {
        // r · r⁻¹ = 1 (binds r_inv to r; non-native, ~negligible).
        let prod = &r[round] * &r_inv[round];
        prod.enforce_equal(&one)?;

        let r_bits = r[round].to_bits_le()?;
        let rinv_bits = r_inv[round].to_bits_le()?;
        let half = cur.len() / 2;
        let mut next = Vec::with_capacity(half);
        for i in 0..half {
            // ck'[i] = r⁻¹·ck_lo[i] + r·ck_hi[i].
            let lo = cur[i].scalar_mul_le(rinv_bits.iter())?;
            let hi = cur[i + half].scalar_mul_le(r_bits.iter())?;
            next.push(lo + hi);
        }
        cur = next;
    }
    debug_assert_eq!(cur.len(), 1);
    Ok(cur.pop().expect("fold collapses to one point"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::{
        short_weierstrass::SWCurveConfig, CurveGroup,
    };
    use ark_ff::Field;
    use ark_r1cs_std::{alloc::AllocVar, GR1CSVar};
    use ark_relations::gr1cs::ConstraintSystem;

    /// CORRECTNESS: in-circuit fold == out-of-circuit recursive fold
    /// == flat `Σ sᵢ·ckᵢ` (closing the loop with `ipa_s_tensor`'s
    /// already-verified equivalence). Small n.
    #[test]
    fn fold_matches_native_recursive_fold() {
        use crate::ipa_s_tensor::ipa_s_vector;
        let g = Projective::from(GrumpkinConfig::GENERATOR);
        let rounds = 3usize;
        let n = 1usize << rounds;
        let ck: Vec<Affine<GrumpkinConfig>> = (0..n)
            .map(|i| (g * Bn254Fq::from((i + 1) as u64)).into_affine())
            .collect();
        let r_val: Vec<Bn254Fq> =
            (0..rounds).map(|i| Bn254Fq::from(i as u64 + 3)).collect();

        // Native truth via the verified tensor-s MSM.
        let s = ipa_s_vector(&r_val);
        let expected = ck
            .iter()
            .zip(s.iter())
            .fold(Projective::<GrumpkinConfig>::from(
                GrumpkinConfig::GENERATOR,
            ) * Bn254Fq::from(0u64), |acc, (c, si)| {
                acc + Projective::from(*c) * *si
            });

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let rv: Vec<_> = r_val
            .iter()
            .map(|x| {
                EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_witness(cs.clone(), || Ok(*x))
                    .unwrap()
            })
            .collect();
        let riv: Vec<_> = r_val
            .iter()
            .map(|x| {
                EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_witness(cs.clone(), || {
                    Ok(x.inverse().unwrap())
                })
                .unwrap()
            })
            .collect();
        let out = tensor_fold_ck_hat(&ck, &rv, &riv).expect("fold synth");
        assert!(cs.is_satisfied().unwrap(), "fold CS must be satisfied");
        assert_eq!(
            out.value().unwrap().into_affine(),
            expected.into_affine(),
            "in-circuit fold must equal the tensor-s MSM (native truth)"
        );
    }

    /// THE DECISION PROBE: measure fold `cs.num_constraints()` at
    /// n ∈ {4,8,16,32}; fit cost ≈ A·n + B; compare A to the flat
    /// baseline 2,533/term. Verdict printed; assert fold is not
    /// absurdly worse (sanity), and surface A_fold vs 2533 so the
    /// flow can decide option (2) vs (1)/(3) WITHOUT a heavy run.
    #[test]
    fn probe_fold_vs_flat_slope() {
        let g = Projective::from(GrumpkinConfig::GENERATOR);
        let measure = |rounds: usize| -> usize {
            let n = 1usize << rounds;
            let ck: Vec<Affine<GrumpkinConfig>> = (0..n)
                .map(|i| (g * Bn254Fq::from((i + 1) as u64)).into_affine())
                .collect();
            let cs = ConstraintSystem::<Bn254Fr>::new_ref();
            let rv: Vec<_> = (0..rounds)
                .map(|i| {
                    EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_witness(cs.clone(), || {
                        Ok(Bn254Fq::from(i as u64 + 3))
                    })
                    .unwrap()
                })
                .collect();
            let riv: Vec<_> = (0..rounds)
                .map(|i| {
                    EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_witness(cs.clone(), || {
                        Ok(Bn254Fq::from(i as u64 + 3).inverse().unwrap())
                    })
                    .unwrap()
                })
                .collect();
            tensor_fold_ck_hat(&ck, &rv, &riv).expect("synth");
            assert!(cs.is_satisfied().unwrap());
            cs.num_constraints()
        };

        let (c4, c8, c16, c32) =
            (measure(2), measure(3), measure(4), measure(5));
        // Fit on the extremes in n (n = 4 .. 32).
        let a = (c32 as f64 - c4 as f64) / (32.0 - 4.0);
        let b = c4 as f64 - a * 4.0;
        let flat_slope = 2533.0_f64;
        let n_real = 131_072.0_f64;
        let fold_pred = a * n_real + b;
        let flat_pred = flat_slope * n_real + 2521.0;

        eprintln!(
            "FOLD_PROBE n4:{c4} n8:{c8} n16:{c16} n32:{c32} \
             A_fold={a:.1} B_fold={b:.1} flat_slope={flat_slope} \
             fold_pred@131072={fold_pred:.0} flat_pred@131072={flat_pred:.0} \
             ratio_fold_over_flat={:.3}",
            fold_pred / flat_pred
        );

        assert!(c8 > c4 && c16 > c8 && c32 > c16, "fold cost must grow");
        // Sanity ceiling only — the REAL decision is the printed
        // ratio, read by the flow (no silent pass/fail on the verdict).
        assert!(
            fold_pred < 1.0e10,
            "fold prediction {fold_pred:.0} absurd (>1e10) — gadget bug"
        );
    }
}
