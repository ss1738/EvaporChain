//! Audit B-1/B-2 S4-nn: primary commitment group validation.
//!
//! The **primary** running instance (`E1 = Bn256EngineKZG`,
//! `CE = HyperKZGCommitmentEngine`) commits over **BN256 G1**.
//! Unlike Grumpkin (no ark crate → bespoke [`crate::grumpkin_config`]),
//! BN254/bn256 G1 is library-provided as `ark_bn254::g1::Config`
//! (`y² = x³ + 3`, generator `(1, 2)`), which is **identical** to
//! halo2curves `bn256` G1. There is therefore NO bespoke primary
//! config — we reuse `ark_bn254::g1::Config` directly.
//!
//! This module carries only the cross-library TRUST GATE: prove the
//! ark and halo2curves representations agree byte-for-byte before any
//! S4a primary-MSM gadget binds against real-fixture `comm_W` bytes.
//! Not trusted until `tests::bn256_g1_matches_halo2curves` passes on
//! the box.

/// The primary commitment group's ark curve config.
///
/// Reused, not bespoke: `ark_bn254::g1::Config` already implements
/// `SWCurveConfig` with `BaseField = ark_bn254::Fq` (non-native point
/// coords in the BN254-Fr circuit → the S4-nn `EmulatedFpVar<Fq,Fr>`
/// side) and `ScalarField = ark_bn254::Fr` (circuit-native scalars).
pub type PrimaryG1Config = ark_bn254::g1::Config;

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::short_weierstrass::SWCurveConfig;
    use ark_ff::{BigInteger, PrimeField};

    /// Independent sanity: ark BN254 G1 is `y² = x³ + 3`, A = 0, and
    /// the generator is on-curve.
    #[test]
    fn bn254_g1_params_sane() {
        let a = <PrimaryG1Config as SWCurveConfig>::COEFF_A;
        let b = <PrimaryG1Config as SWCurveConfig>::COEFF_B;
        assert_eq!(a, ark_bn254::Fq::from(0u64), "BN254 G1 COEFF_A must be 0");
        assert_eq!(b, ark_bn254::Fq::from(3u64), "BN254 G1 COEFF_B must be 3");

        let g = <PrimaryG1Config as SWCurveConfig>::GENERATOR;
        assert!(!g.infinity, "generator must not be identity");
        let rhs = g.x * g.x * g.x + b; // A = 0
        assert_eq!(g.y * g.y, rhs, "generator must satisfy y^2 = x^3 + 3");
        assert!(g.is_on_curve());
    }

    /// THE TRUST GATE — ark `ark_bn254::g1::Config` generator vs
    /// halo2curves `bn256` G1 generator, byte-for-byte. If the curve
    /// or representation convention differs in any bit, this fails
    /// and the primary group must NOT be bound against real-fixture
    /// commitment bytes downstream.
    #[test]
    fn bn256_g1_matches_halo2curves() {
        use ff::PrimeField as _;
        use halo2curves::bn256::G1Affine;
        use halo2curves::group::prime::PrimeCurveAffine;
        use halo2curves::CurveAffine;

        let h = G1Affine::generator();
        let c = h.coordinates().unwrap();
        let hx = c.x().to_repr(); // canonical little-endian
        let hy = c.y().to_repr();

        let g = <PrimaryG1Config as SWCurveConfig>::GENERATOR;
        let ax = g.x.into_bigint().to_bytes_le();
        let ay = g.y.into_bigint().to_bytes_le();

        assert_eq!(
            ax.as_slice(),
            hx.as_ref(),
            "BN254/bn256 G1 generator x: ark vs halo2curves byte mismatch"
        );
        assert_eq!(
            ay.as_slice(),
            hy.as_ref(),
            "BN254/bn256 G1 generator y: ark vs halo2curves byte mismatch"
        );
    }
}
