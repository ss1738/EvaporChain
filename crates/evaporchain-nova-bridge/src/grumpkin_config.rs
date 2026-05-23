//! Audit B-1/B-2 S4-nn: ark `SWCurveConfig` for nova-snark's Grumpkin
//! curve — the **secondary** running instance's Pedersen commitment
//! group (`E2 = GrumpkinEngine`, `CE = PedersenCommitmentEngine`).
//!
//! Grumpkin: `y² = x³ − 17` with
//! - `BaseField` (point coordinates) = BN254 **Fr** = the
//!   circuit-native field (`ark_bn254::Fr`) — this is why Section 2
//!   can absorb `comm_W.{x,y}` directly as `ArkFr`.
//! - `ScalarField` = BN254 **Fq** (`ark_bn254::Fq`) — non-native in
//!   the BN254-Fr circuit (this is the S4-nn `EmulatedFpVar<Fq,Fr>`
//!   side for the secondary MSM).
//! - Prime order (cofactor 1).
//!
//! Constants were extracted from halo2curves 0.9.0
//! `grumpkin::G1Affine::generator()` (canonical `to_repr`) and
//! on-curve self-verified (`y² ≡ x³ − 17 mod p`).
//!
//! TRUST GATE: `tests::grumpkin_config_matches_halo2curves` compares
//! every coordinate byte-for-byte against halo2curves at runtime.
//! This config is NOT trusted until that test passes on the box.

use ark_bn254::{Fq as Bn254Fq, Fr as Bn254Fr};
use ark_ec::{
    short_weierstrass::{Affine, SWCurveConfig},
    CurveConfig,
};
use ark_ff::MontFp;

/// ark curve config for nova-snark / halo2curves Grumpkin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrumpkinConfig;

impl CurveConfig for GrumpkinConfig {
    type BaseField = Bn254Fr;
    type ScalarField = Bn254Fq;

    const COFACTOR: &'static [u64] = &[1];
    const COFACTOR_INV: Bn254Fq = MontFp!("1");
}

impl SWCurveConfig for GrumpkinConfig {
    /// ark-ec 0.6 added `ZeroFlag` for explicit point-at-infinity
    /// tracking in hash-to-curve maps (WB / SWU). Grumpkin doesn't
    /// use those code paths, so `()` is correct — matches the
    /// standard SW-curve impls in ark-bn254/ark-bls12-381 0.6.
    type ZeroFlag = ();
    /// A = 0.
    const COEFF_A: Bn254Fr = MontFp!("0");
    /// B = −17 mod p, p = BN254 Fr modulus
    /// (21888242871839275222246405745257275088548364400416034343698204186575808495617).
    const COEFF_B: Bn254Fr = MontFp!(
        "21888242871839275222246405745257275088548364400416034343698204186575808495600"
    );
    /// halo2curves canonical generator (x = 1).
    const GENERATOR: Affine<Self> = Affine::new_unchecked(
        MontFp!("1"),
        MontFp!("17631683881184975370165255887551781615748388533673675138860"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent on-curve check: the generator satisfies
    /// `y² = x³ − 17` (A = 0).
    #[test]
    fn generator_is_on_curve() {
        let g = GrumpkinConfig::GENERATOR;
        assert!(!g.infinity, "generator must not be the identity");
        let rhs = g.x * g.x * g.x + GrumpkinConfig::COEFF_B;
        assert_eq!(g.y * g.y, rhs, "y^2 must equal x^3 - 17");
        assert!(g.is_on_curve(), "ark is_on_curve must agree");
    }

    /// THE TRUST GATE — cross-library byte equality vs halo2curves
    /// 0.9 (the same crate nova-snark commits with). If the
    /// representation/curve convention differs in any bit, this fails
    /// and the config must NOT be used downstream.
    #[test]
    fn grumpkin_config_matches_halo2curves() {
        use ark_ff::{BigInteger, PrimeField};
        use ff::PrimeField as _;
        use halo2curves::group::prime::PrimeCurveAffine;
        use halo2curves::grumpkin::G1Affine;
        use halo2curves::CurveAffine;

        let h = G1Affine::generator();
        let c = h.coordinates().unwrap();
        let hx = c.x().to_repr(); // canonical little-endian
        let hy = c.y().to_repr();

        let ax = GrumpkinConfig::GENERATOR.x.into_bigint().to_bytes_le();
        let ay = GrumpkinConfig::GENERATOR.y.into_bigint().to_bytes_le();

        assert_eq!(
            ax.as_slice(),
            hx.as_ref(),
            "generator x: ark vs halo2curves byte mismatch"
        );
        assert_eq!(
            ay.as_slice(),
            hy.as_ref(),
            "generator y: ark vs halo2curves byte mismatch"
        );
    }
}
