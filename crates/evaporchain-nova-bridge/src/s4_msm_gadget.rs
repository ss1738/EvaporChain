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
}
