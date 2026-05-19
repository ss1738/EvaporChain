//! Audit B-1/B-2 S4a (PHASE B): in-circuit Pedersen/HyperKZG MSM over
//! the **primary** commitment group (bn256 G1).
//!
//! Field-inverse of [`crate::s4_msm_gadget`] (Grumpkin/secondary):
//! - Primary commitment group = bn256 G1 = `ark_bn254::g1::Config`.
//! - Point coordinates = `bn256::Base` = BN254 **Fq** → non-native →
//!   `ProjectiveVar<ark_bn254::g1::Config, EmulatedFpVar<Fq, Fr>>`.
//! - MSM scalars = `bn256::Scalar` = BN254 **Fr** → circuit-NATIVE →
//!   `FpVar<Fr>` (NOT `EmulatedFpVar`).
//!
//! HyperKZG `commit` is `Σ vᵢ·ckᵢ + r·h` — identical MSM form to
//! Pedersen (S4-0 source-verified; no pairing). This recomputes that
//! MSM in-circuit so a later step can bind Section-3 primary `W` to
//! the Section-2-bound primary `comm_W`. Unit-proven in isolation
//! here (in-circuit == out-of-circuit ark MSM); real-fixture wiring
//! is PHASE B.3.

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr};
use ark_ec::short_weierstrass::{Affine, Projective};
use ark_r1cs_std::{
    convert::ToBitsGadget,
    fields::emulated_fp::EmulatedFpVar,
    fields::fp::FpVar,
    groups::{curves::short_weierstrass::ProjectiveVar, CurveVar},
};
use ark_relations::r1cs::SynthesisError;

/// Primary commitment-group curve config (reused, not bespoke).
pub type PrimaryCfg = ark_bn254::g1::Config;

/// In-circuit bn256-G1 point variable — NON-NATIVE coordinates
/// (`EmulatedFpVar<Fq, Fr>`), the inverse of the Grumpkin gadget's
/// native `FpVar<Fr>` coords.
pub type Bn256G1Var = ProjectiveVar<PrimaryCfg, EmulatedFpVar<Bn254Fq, Bn254Fr>>;

/// Recompute the primary commitment `Σ scalarsᵢ·basesᵢ + blind·h`
/// in-circuit. `bases`/`h` are public constants (the commitment
/// key); `scalars`/`blind` are circuit-NATIVE (`bn256::Scalar` =
/// BN254 Fr) witnesses.
pub fn pedersen_msm_bn256_g1(
    scalars: &[FpVar<Bn254Fr>],
    bases: &[Affine<PrimaryCfg>],
    blind: &FpVar<Bn254Fr>,
    h: Affine<PrimaryCfg>,
) -> Result<Bn256G1Var, SynthesisError> {
    assert_eq!(
        scalars.len(),
        bases.len(),
        "pedersen_msm_bn256_g1: scalars/bases length mismatch"
    );
    let mut acc = Bn256G1Var::zero();
    for (s, base) in scalars.iter().zip(bases.iter()) {
        let bits = s.to_bits_le()?;
        let term =
            Bn256G1Var::constant(Projective::from(*base)).scalar_mul_le(bits.iter())?;
        acc += term;
    }
    let rbits = blind.to_bits_le()?;
    acc += Bn256G1Var::constant(Projective::from(h)).scalar_mul_le(rbits.iter())?;
    Ok(acc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::{short_weierstrass::SWCurveConfig, CurveGroup};
    use ark_r1cs_std::{alloc::AllocVar, R1CSVar};
    use ark_relations::r1cs::ConstraintSystem;

    /// THE PRIMITIVE PROOF (primary side): in-circuit bn256-G1 MSM
    /// (non-native coords, native Fr scalars) equals the
    /// out-of-circuit ark MSM, satisfied CS. Small fixed case.
    #[test]
    fn pedersen_msm_bn256_g1_matches_native() {
        let g = Projective::<PrimaryCfg>::from(PrimaryCfg::GENERATOR);
        let g2 = g + g;
        let h_pt = g * Bn254Fr::from(7u64);
        let bases = [g.into_affine(), g2.into_affine()];
        let h_aff = h_pt.into_affine();

        // Native scalars (bn256 scalar field = BN254 Fr).
        let s0 = Bn254Fr::from(2u64);
        let s1 = Bn254Fr::from(3u64);
        let r = Bn254Fr::from(5u64);
        let expected = g * s0 + g2 * s1 + h_pt * r;

        let cs = ConstraintSystem::<Bn254Fr>::new_ref();
        let sv0 = FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(s0)).unwrap();
        let sv1 = FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(s1)).unwrap();
        let rv = FpVar::<Bn254Fr>::new_witness(cs.clone(), || Ok(r)).unwrap();

        let out = pedersen_msm_bn256_g1(&[sv0, sv1], &bases, &rv, h_aff)
            .expect("primary msm gadget");

        assert!(
            cs.is_satisfied().expect("is_satisfied"),
            "primary bn256-G1 MSM gadget CS must be satisfied"
        );
        assert_eq!(
            out.value().expect("circuit point").into_affine(),
            expected.into_affine(),
            "in-circuit bn256-G1 MSM must equal out-of-circuit ark MSM"
        );
    }
}
