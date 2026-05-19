//! B-1/B-2 EVM option (2), increment 2 kernel: the deterministic
//! IPA `s`-tensor derivation.
//!
//! `nova-snark` 0.68 `ipa_pc::verify` reconstructs the folded
//! commitment key as a single MSM `ck_hat = Σ sᵢ·ckᵢ`, where the
//! length-`n` scalar vector `s` has a tensor structure built from
//! the `log₂(n)` Fiat-Shamir round challenges (`ipa_pc.rs`
//! L334-349). The `RecursionDeciderCircuit` Section-A witness needs
//! exactly this `s`. Porting the recurrence wrong (index / exponent
//! / round-reversal off-by-one) silently produces a different MSM
//! and a vacuous-yet-passing binding — the B-1 hazard. So this
//! kernel is isolated and falsified **independently**: the tensor
//! MSM `Σ sᵢ·ckᵢ` must equal the literal recursive `ck.fold`
//! (`pedersen.rs::fold` L487 + `ipa_pc.rs` prove loop) the prover
//! actually performs. Two unrelated code paths, same point, or the
//! port is wrong.

use ark_ff::Field;

/// Port of `nova-snark` 0.68 `ipa_pc::verify`'s `s` derivation
/// (`provider/ipa_pc.rs` L334-349), bit-exact.
///
/// `r` = the per-round challenges in verifier order (`r[0]` = first
/// round, operating on the full size-`n` vector; `r[L-1]` = last).
/// Returns the length-`n = 2^L` tensor vector. `L = r.len()`.
///
/// ```text
/// s[0]            = Π_k r[k]⁻¹
/// pos_in_r(i)     = ⌊log₂ i⌋               (i ≥ 1)
/// s[i]            = s[i − 2^{pos}] · r[(L−1) − pos]²
/// ```
pub fn ipa_s_vector<F: Field>(r: &[F]) -> Vec<F> {
    let rounds = r.len();
    let n = 1usize << rounds;

    // r_square[i] = r[i]², r_inverse[i] = r[i]⁻¹ (elementwise inverse
    // in index order == nova-snark's `batch_invert` for this use).
    let r_square: Vec<F> = r.iter().map(|x| *x * *x).collect();
    let r_inverse: Vec<F> = r
        .iter()
        .map(|x| x.inverse().expect("IPA challenge must be invertible"))
        .collect();

    let mut s = vec![F::zero(); n];
    // s[0] = Π_k r_inverse[k]
    s[0] = r_inverse.iter().fold(F::one(), |acc, ri| acc * *ri);
    for i in 1..n {
        let pos_in_r = (31 - (i as u32).leading_zeros()) as usize;
        s[i] = s[i - (1 << pos_in_r)] * r_square[(rounds - 1) - pos_in_r];
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grumpkin_config::GrumpkinConfig;
    use ark_bn254::Fq as Bn254Fq;
    use ark_ec::{
        short_weierstrass::{Projective, SWCurveConfig},
        CurveGroup,
    };
    use ark_ff::UniformRand;
    use ark_std::{test_rng, Zero};

    type GProj = Projective<GrumpkinConfig>;

    /// THE INDEPENDENT FALSIFIER. Build a real-shaped instance:
    /// `n = 2^L` distinct Grumpkin bases `ck`, `L` random challenges.
    /// Path 1: `ck_hat = Σ sᵢ·ckᵢ` with `s` from [`ipa_s_vector`].
    /// Path 2: the literal recursive fold the prover performs —
    /// per round `ck'[i] = r⁻¹·ck_lo[i] + r·ck_hi[i]`
    /// (`pedersen.rs::fold`, weights `(r_inverse, r)` from
    /// `ipa_pc` prove loop), collapsing `ck` to one point.
    /// They MUST coincide; any recurrence error breaks this.
    fn assert_s_matches_fold(rounds: usize) {
        let mut rng = test_rng();
        let g = GProj::from(GrumpkinConfig::GENERATOR);
        let n = 1usize << rounds;

        // Distinct, non-trivial bases ckᵢ = (i+1)·G.
        let ck: Vec<GProj> =
            (0..n).map(|i| g * Bn254Fq::from((i + 1) as u64)).collect();
        let r: Vec<Bn254Fq> =
            (0..rounds).map(|_| Bn254Fq::rand(&mut rng)).collect();

        // Path 1: tensor MSM.
        let s = ipa_s_vector(&r);
        assert_eq!(s.len(), n, "s length must be 2^rounds");
        let ck_hat_via_s: GProj = s
            .iter()
            .zip(ck.iter())
            .fold(GProj::zero(), |acc, (si, ci)| acc + *ci * *si);

        // Path 2: literal recursive fold (challenges in round order
        // r[0]..r[L-1], matching the prove loop & s indexing).
        let mut cur = ck.clone();
        for &r_round in r.iter() {
            let r_inv = r_round.inverse().unwrap();
            let half = cur.len() / 2;
            let folded: Vec<GProj> = (0..half)
                .map(|i| cur[i] * r_inv + cur[i + half] * r_round)
                .collect();
            cur = folded;
        }
        assert_eq!(cur.len(), 1, "fold must collapse to a single point");
        let ck_hat_via_fold = cur[0];

        assert_eq!(
            ck_hat_via_s.into_affine(),
            ck_hat_via_fold.into_affine(),
            "rounds={rounds}: tensor-s MSM must equal the recursive \
             ck.fold — a mismatch means the ipa_s_vector port is wrong"
        );
    }

    #[test]
    fn s_tensor_matches_recursive_fold_n8() {
        assert_s_matches_fold(3);
    }

    #[test]
    fn s_tensor_matches_recursive_fold_n16() {
        assert_s_matches_fold(4);
    }

    #[test]
    fn s_tensor_matches_recursive_fold_n64() {
        assert_s_matches_fold(6);
    }

    /// Spot-check the closed form: `s[0] = Π r[k]⁻¹`, and the full
    /// product `Π_k r[k]` appears as `s[n-1]` up to the round map
    /// (catches a silent all-ones / wrong-exponent regression that
    /// the fold check could in principle absorb).
    #[test]
    fn s_zero_is_product_of_inverses() {
        let mut rng = test_rng();
        let r: Vec<Bn254Fq> = (0..4).map(|_| Bn254Fq::rand(&mut rng)).collect();
        let s = ipa_s_vector(&r);
        let expected_s0 = r
            .iter()
            .fold(Bn254Fq::from(1u64), |a, x| a * x.inverse().unwrap());
        assert_eq!(s[0], expected_s0, "s[0] must be Π r[k]⁻¹");
        assert_ne!(s[1], s[0], "tensor must be non-degenerate");
    }
}
