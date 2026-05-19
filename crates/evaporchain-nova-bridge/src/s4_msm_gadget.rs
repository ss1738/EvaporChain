//! Audit B-1/B-2 S4a: in-circuit Pedersen multi-scalar-multiplication
//! over the secondary (Grumpkin) commitment group.
//!
//! Nova's secondary `CommitmentEngine` (Pedersen) computes
//! `comm = Σ vᵢ·ckᵢ + r·h` (S4-0: verified, no pairing). This gadget
//! recomputes that MSM **inside the R1CS** so a future S4a binding
//! step can enforce `recomputed == comm_W` (the Section-2-bound
//! commitment), closing the "W in Section 3 ≠ W behind comm_W" hole.
//!
//! Grumpkin point coordinates are BN254 **Fr** = the circuit-native
//! field → `ProjectiveVar<GrumpkinConfig, FpVar<Fr>>` (native EC
//! arithmetic). The committed scalars are Grumpkin scalar field =
//! BN254 **Fq** (non-native) → `EmulatedFpVar<Fq, Fr>` decomposed to
//! bits for `scalar_mul_le`. The Pedersen bases (`ck`, `h`) are
//! public constants.
//!
//! This module unit-proves the gadget in isolation (in-circuit MSM ==
//! out-of-circuit ark MSM). Wiring it to a real nova fixture's `ck` /
//! `comm_W` is the next S4a sub-unit.

use crate::grumpkin_config::GrumpkinConfig;
use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr};
use ark_ec::short_weierstrass::{Affine, Projective};
use ark_r1cs_std::{
    convert::ToBitsGadget,
    fields::emulated_fp::EmulatedFpVar,
    fields::fp::FpVar,
    groups::{curves::short_weierstrass::ProjectiveVar, CurveVar},
};
use ark_relations::r1cs::SynthesisError;

/// In-circuit Grumpkin point variable (native Fr coordinates).
pub type GrumpkinVar = ProjectiveVar<GrumpkinConfig, FpVar<Bn254Fr>>;

/// Recompute the Pedersen commitment `Σ scalarsᵢ·basesᵢ + blind·h`
/// in-circuit. `bases` and `h` are public constants (the commitment
/// key); `scalars` and `blind` are non-native (Grumpkin-scalar =
/// BN254 Fq) circuit witnesses.
pub fn pedersen_msm_grumpkin(
    scalars: &[EmulatedFpVar<Bn254Fq, Bn254Fr>],
    bases: &[Affine<GrumpkinConfig>],
    blind: &EmulatedFpVar<Bn254Fq, Bn254Fr>,
    h: Affine<GrumpkinConfig>,
) -> Result<GrumpkinVar, SynthesisError> {
    assert_eq!(
        scalars.len(),
        bases.len(),
        "pedersen_msm_grumpkin: scalars/bases length mismatch"
    );
    let mut acc = GrumpkinVar::zero();
    for (s, base) in scalars.iter().zip(bases.iter()) {
        let bits = s.to_bits_le()?;
        let term = GrumpkinVar::constant(Projective::from(*base))
            .scalar_mul_le(bits.iter())?;
        acc += term;
    }
    let rbits = blind.to_bits_le()?;
    acc += GrumpkinVar::constant(Projective::from(h)).scalar_mul_le(rbits.iter())?;
    Ok(acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::{short_weierstrass::SWCurveConfig, CurveGroup};
    use ark_r1cs_std::{alloc::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;

    /// S4a-wiring-0 (diagnostic, not a correctness assert): dump the
    /// real `serde_json::to_value(pp)["ck_secondary"]` shape so the
    /// `ck`/`h` affine encoding can be pinned EXACTLY (it has no
    /// `#[serde_as]`, so it is NOT the `comm_W` EvmCompatSerde path).
    /// `#[ignore]`: needs `PublicParams::setup` (seconds, no Nova
    /// fixture). Run:
    ///   cargo test -p evaporchain-nova-bridge dump_ck_secondary_shape \
    ///     -- --ignored --nocapture
    #[test]
    #[ignore = "S4a-wiring-0 diagnostic: prints real pp ck_secondary JSON shape"]
    fn dump_ck_secondary_shape() {
        let pp = crate::recursive_snark_fixture::canonical_public_params()
            .expect("canonical pp");
        let v = serde_json::to_value(&pp).expect("pp to_value");

        let obj = v.as_object().expect("pp json is object");
        let mut keys: Vec<&String> = obj.keys().collect();
        keys.sort();
        eprintln!("PP_TOP_KEYS = {keys:?}");

        let ck_sec = &v["ck_secondary"];
        eprintln!(
            "ck_secondary IS_NULL={} IS_OBJ={} IS_ARR={} IS_STR={}",
            ck_sec.is_null(),
            ck_sec.is_object(),
            ck_sec.is_array(),
            ck_sec.is_string()
        );
        if let Some(o) = ck_sec.as_object() {
            let mut sk: Vec<&String> = o.keys().collect();
            sk.sort();
            eprintln!("ck_secondary KEYS = {sk:?}");
            let ck = &ck_sec["ck"];
            eprintln!(
                "ck_secondary.ck IS_ARR={} len={:?}",
                ck.is_array(),
                ck.as_array().map(|a| a.len())
            );
            if let Some(a) = ck.as_array() {
                if let Some(e0) = a.first() {
                    eprintln!("ck[0] = {}", serde_json::to_string(e0).unwrap_or_default());
                }
            }
            eprintln!(
                "ck_secondary.h = {}",
                serde_json::to_string(&ck_sec["h"]).unwrap_or_default()
            );
        } else {
            eprintln!(
                "ck_secondary RAW (truncated 400) = {}",
                &serde_json::to_string(ck_sec).unwrap_or_default()
                    [..serde_json::to_string(ck_sec).unwrap_or_default().len().min(400)]
            );
        }
    }

    /// THE PRIMITIVE PROOF: the in-circuit Pedersen MSM equals the
    /// out-of-circuit ark Grumpkin MSM, and the constraint system is
    /// satisfied. Small fixed case (2 bases) — fast, no nova fixture.
    #[test]
    fn pedersen_msm_grumpkin_matches_native() {
        // Public constant bases: G and 2G; blinding base h = 7G.
        let g = Projective::from(GrumpkinConfig::GENERATOR);
        let g2 = g + g;
        let h_pt = g * Bn254Fq::from(7u64);
        let bases = [g.into_affine(), g2.into_affine()];
        let h_aff = h_pt.into_affine();

        // Secret scalars (Grumpkin scalar field = BN254 Fq).
        let s0 = Bn254Fq::from(2u64);
        let s1 = Bn254Fq::from(3u64);
        let r = Bn254Fq::from(5u64);

        // Out-of-circuit expected: 2·G + 3·(2G) + 5·(7G).
        let expected = g * s0 + g2 * s1 + h_pt * r;

        // In-circuit.
        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let sv0 = EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_witness(cs.clone(), || Ok(s0)).unwrap();
        let sv1 = EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_witness(cs.clone(), || Ok(s1)).unwrap();
        let rv = EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_witness(cs.clone(), || Ok(r)).unwrap();

        let out = pedersen_msm_grumpkin(&[sv0, sv1], &bases, &rv, h_aff)
            .expect("gadget synthesis");

        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "Pedersen-MSM gadget constraint system must be satisfied"
        );
        assert_eq!(
            out.value().expect("circuit point value").into_affine(),
            expected.into_affine(),
            "in-circuit Pedersen MSM must equal out-of-circuit ark MSM"
        );
    }

    /// D.3-STYLE SIZE PREDICTION (no build/spend) for the final-layer
    /// recursion circuit's dominant term: the secondary IPA `ck_hat`
    /// = Σ sᵢ·ckᵢ MSM done **natively** (Grumpkin points have
    /// BN254-Fr coords = circuit field; scalars are non-native Fq).
    /// Measure `cs.num_constraints()` for k ∈ {1,2,4,8} terms, linear-
    /// fit `cost ≈ a·k + b`, predict at n = 10_554 (D.3-measured
    /// secondary `num_cons`). FALSIFIER: if predicted ≥ 1e9 the
    /// "recursion terminates succinct" conclusion (flow source-read
    /// #3) is FALSE — panic loudly. Fast (tiny circuits), no fixture.
    #[test]
    fn predict_native_grumpkin_msm_size_for_recursion_circuit() {
        let g = Projective::from(GrumpkinConfig::GENERATOR);
        let h_aff = (g * Bn254Fq::from(7u64)).into_affine();

        let measure = |k: usize| -> usize {
            let bases: Vec<_> =
                (1..=k).map(|i| (g * Bn254Fq::from(i as u64)).into_affine()).collect();
            let cs = ConstraintSystem::<Bn254Fr>::new_ref();
            let scalars: Vec<EmulatedFpVar<Bn254Fq, Bn254Fr>> = (1..=k)
                .map(|i| {
                    EmulatedFpVar::new_witness(cs.clone(), || Ok(Bn254Fq::from(i as u64 + 1)))
                        .unwrap()
                })
                .collect();
            let blind =
                EmulatedFpVar::<Bn254Fq, Bn254Fr>::new_witness(cs.clone(), || Ok(Bn254Fq::from(5u64)))
                    .unwrap();
            pedersen_msm_grumpkin(&scalars, &bases, &blind, h_aff).expect("synth");
            assert!(cs.is_satisfied().unwrap(), "k={k} CS must be satisfied");
            cs.num_constraints()
        };

        let (c1, c2, c4, c8) = (measure(1), measure(2), measure(4), measure(8));
        // Linear fit on the two extremes (the blind term is the +b).
        let a = (c8 as f64 - c1 as f64) / 7.0; // per-MSM-term slope
        let b = c1 as f64 - a; // intercept (incl. one blind scalar_mul)
        let n_secondary = 10_554.0_f64;
        let predicted = a * n_secondary + b;
        // HyperKZG primary verify in-circuit ≈ a constant BN254
        // pairing (~1–2e6, well-characterized); recorded, not summed
        // into the falsifier (it is bounded-constant by construction).
        let hyperkzg_pairing_const = 2.0e6_f64;
        let total = predicted + hyperkzg_pairing_const;

        eprintln!(
            "NGM_PREDICT k=1:{c1} k=2:{c2} k=4:{c4} k=8:{c8} \
             slope_a={a:.1} intercept_b={b:.1} \
             n_secondary={} predicted_msm={:.0} \
             +hyperkzg_const={:.0} TOTAL={:.0}",
            n_secondary as u64, predicted, hyperkzg_pairing_const, total
        );

        // Sanity: cost must be monotone increasing & ~linear in k.
        assert!(c2 > c1 && c4 > c2 && c8 > c4, "MSM cost must grow with k");
        // FALSIFIER (flow source-read #3): native-side recursion must
        // NOT reintroduce S4b/D.3 scale (~2e8). 1e9 is a generous
        // ceiling — tripping it falsifies "recursion terminates".
        assert!(
            total < 1.0e9,
            "FALSIFIER TRIPPED: predicted recursion-circuit size {total:.0} ≥ 1e9 \
             — 'recursion terminates succinct' is FALSE; fall to flow option (3)"
        );
    }
}
