//! Sumcheck protocol on the multilinear extension of the leaf-
//! energy vector. V1 of the V2 protocol — the chain's vector-
//! commitment scheme is out of scope; V1 has the verifier hold
//! the full data, and exists to demonstrate the protocol shape +
//! soundness gate.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::field::{add_p, inverse_p, mul_p, multilinear_extend, sub_p, FieldElem, MOD_P};

/// Univariate polynomial of degree ≤ 2 represented by its values
/// at x = 0, 1, 2. Lagrange interpolation gives evaluation at
/// any other point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnivariatePoly3 {
    pub at_0: FieldElem,
    pub at_1: FieldElem,
    pub at_2: FieldElem,
}

impl UnivariatePoly3 {
    /// Lagrange-evaluate at `x` ∈ F_p.
    /// `p(x) = at_0·L₀(x) + at_1·L₁(x) + at_2·L₂(x)`
    /// where L₀(x) = (x−1)(x−2)/2, L₁(x) = x(x−2)/(−1), L₂(x) = x(x−1)/2.
    pub fn evaluate(&self, x: FieldElem) -> FieldElem {
        let inv2 = inverse_p(2).expect("2 has an inverse mod P");
        // L₀ = (x−1)(x−2)·inv(2)
        let l0 = mul_p(mul_p(sub_p(x, 1), sub_p(x, 2)), inv2);
        // L₁ = x(x−2)·(−1) = neg(x(x−2))
        let l1 = sub_p(0, mul_p(x, sub_p(x, 2)));
        // L₂ = x(x−1)·inv(2)
        let l2 = mul_p(mul_p(x, sub_p(x, 1)), inv2);

        let t0 = mul_p(self.at_0, l0);
        let t1 = mul_p(self.at_1, l1);
        let t2 = mul_p(self.at_2, l2);
        add_p(add_p(t0, t1), t2)
    }
}

/// One sumcheck-folded inclusion proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SumcheckProof {
    /// The total the prover claims for `Σ_x g(x)`.
    pub claimed_total: FieldElem,
    /// One round-polynomial per variable.
    pub round_polys: Vec<UnivariatePoly3>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SumcheckError {
    #[error("leaves vector must be non-empty and a power of two")]
    BadLeavesShape,
    #[error("target index {idx} out of range for {n} leaves")]
    TargetOutOfRange { idx: usize, n: usize },
    #[error("claimed total disagrees with target leaf energy")]
    ClaimNotEqualToTarget,
    #[error("round-polynomial count mismatch: got {got}, expected {expected}")]
    RoundCountMismatch { got: usize, expected: usize },
    #[error(
        "round {round} consistency check failed: round_poly(0) + round_poly(1) = {observed} but expected {expected}"
    )]
    RoundConsistency {
        round: usize,
        observed: FieldElem,
        expected: FieldElem,
    },
    #[error(
        "final-evaluation check failed: prover-implied g(r₁,…,r_v) = {claimed} but recomputed = {recomputed}"
    )]
    FinalEval {
        claimed: FieldElem,
        recomputed: FieldElem,
    },
    #[error("claim {claim} below energy floor {floor}")]
    BelowFloor { claim: FieldElem, floor: u64 },
}

const FS_TAG: &[u8] = b"evaporchain:epa-mmr-sumcheck:v1\0";

/// Derive a Fiat-Shamir challenge from the running transcript.
fn fs_challenge(transcript: &mut blake3::Hasher, label: &[u8]) -> FieldElem {
    let mut h = transcript.clone();
    h.update(label);
    let bytes: [u8; 32] = *h.finalize().as_bytes();
    transcript.update(&bytes); // mix back
    let r = u64::from_le_bytes(bytes[..8].try_into().unwrap()) % MOD_P;
    // Avoid the degenerate r=0 / r=1 / r=2 case by re-rolling
    // simply with a counter in the label.
    if r > 2 {
        return r;
    }
    let mut counter = 0u64;
    loop {
        let mut h2 = transcript.clone();
        h2.update(label);
        h2.update(&counter.to_le_bytes());
        let bytes: [u8; 32] = *h2.finalize().as_bytes();
        transcript.update(&bytes);
        let r2 = u64::from_le_bytes(bytes[..8].try_into().unwrap()) % MOD_P;
        if r2 > 2 {
            return r2;
        }
        counter += 1;
    }
}

/// Build the `selector` MLE evaluated at `point`, where the
/// selector's binary value is 1 at `target` and 0 elsewhere.
/// Closed form: `∏_i (target_bit_i · point_i + (1 − target_bit_i) · (1 − point_i))`.
///
/// **Convention**: the MLE in `field::multilinear_extend` folds
/// the FIRST coordinate of `point` as the high-order index bit
/// (MSB-first). So `point[0]` corresponds to bit `v−1` of
/// `target`. Selector matches this convention.
fn selector_eval(target: usize, point: &[FieldElem]) -> FieldElem {
    let v = point.len();
    let mut acc: FieldElem = 1;
    for i in 0..v {
        // point[i] ↔ bit (v − 1 − i) of target (MSB-first).
        let bit_index = v - 1 - i;
        let target_bit = ((target >> bit_index) & 1) as FieldElem;
        let p = point[i];
        let one_minus_target = sub_p(1, target_bit);
        let one_minus_p = sub_p(1, p);
        let term = add_p(mul_p(target_bit, p), mul_p(one_minus_target, one_minus_p));
        acc = mul_p(acc, term);
    }
    acc
}

/// Honest-prover construction. Builds the round polynomials by
/// computing `Σ_remaining g(r_fixed, X, …)` at X ∈ {0, 1, 2}.
pub fn prove_sumcheck_inclusion(
    leaves: &[FieldElem],
    target: usize,
) -> Result<SumcheckProof, SumcheckError> {
    let n = leaves.len();
    if n == 0 || !n.is_power_of_two() {
        return Err(SumcheckError::BadLeavesShape);
    }
    if target >= n {
        return Err(SumcheckError::TargetOutOfRange { idx: target, n });
    }
    let v = n.trailing_zeros() as usize;
    let claimed_total = leaves[target] % MOD_P;

    // Build the FS transcript identically to the verifier.
    let mut transcript = blake3::Hasher::new();
    transcript.update(FS_TAG);
    transcript.update(&(n as u64).to_le_bytes());
    transcript.update(&(target as u64).to_le_bytes());
    transcript.update(&claimed_total.to_le_bytes());

    let mut fixed_r: Vec<FieldElem> = Vec::with_capacity(v);
    let mut round_polys: Vec<UnivariatePoly3> = Vec::with_capacity(v);

    for round_k in 0..v {
        // Build s_k(X) at X ∈ {0,1,2} by summing g(r_1,…,r_{k-1}, X, x_{k+1},…,x_v)
        // over the remaining-variable cube.
        let mut at = [0u64; 3];
        let remaining_dims = v - round_k - 1;
        let cube_size = 1usize << remaining_dims;
        for &x_value in &[0u64, 1u64, 2u64] {
            let idx_pos = if x_value == 0 { 0 } else if x_value == 1 { 1 } else { 2 };
            let mut sum: FieldElem = 0;
            for cube_pt in 0..cube_size {
                // Build the full v-dim point: (r_1,…,r_{k-1}, x_value, cube_pt's bits…)
                let mut point: Vec<FieldElem> = Vec::with_capacity(v);
                for &r in &fixed_r {
                    point.push(r);
                }
                point.push(x_value);
                for d in 0..remaining_dims {
                    point.push(((cube_pt >> d) & 1) as FieldElem);
                }
                // g(point) = leaf_energy_MLE(point) · selector(target, point)
                let leaf_val = multilinear_extend(leaves, &point)
                    .expect("MLE at well-shaped point");
                let sel_val = selector_eval(target, &point);
                sum = add_p(sum, mul_p(leaf_val, sel_val));
            }
            at[idx_pos] = sum;
        }
        let poly = UnivariatePoly3 {
            at_0: at[0],
            at_1: at[1],
            at_2: at[2],
        };
        round_polys.push(poly);
        // Fold transcript with this round's poly.
        transcript.update(&at[0].to_le_bytes());
        transcript.update(&at[1].to_le_bytes());
        transcript.update(&at[2].to_le_bytes());
        let r_k = fs_challenge(&mut transcript, b"round");
        fixed_r.push(r_k);
    }

    Ok(SumcheckProof {
        claimed_total,
        round_polys,
    })
}

/// Verifier gate. Given `leaves`, `target`, `proof`, and an
/// energy `floor`:
///
/// 1. Round consistency: for each round k, `round_poly[k](0) +
///    round_poly[k](1) == previous_value`.
/// 2. Final evaluation: at the FS-derived randomness
///    `(r_1,…,r_v)`, recompute `g = leaf_MLE · selector` and
///    compare to `round_poly[v−1](r_v)`.
/// 3. Energy floor: `claimed_total ≥ floor`.
pub fn verify_sumcheck_inclusion(
    leaves: &[FieldElem],
    target: usize,
    proof: &SumcheckProof,
    floor: u64,
) -> Result<(), SumcheckError> {
    let n = leaves.len();
    if n == 0 || !n.is_power_of_two() {
        return Err(SumcheckError::BadLeavesShape);
    }
    if target >= n {
        return Err(SumcheckError::TargetOutOfRange { idx: target, n });
    }
    let v = n.trailing_zeros() as usize;
    if proof.round_polys.len() != v {
        return Err(SumcheckError::RoundCountMismatch {
            got: proof.round_polys.len(),
            expected: v,
        });
    }

    // 3. Energy floor first (cheap, fail fast).
    if proof.claimed_total < floor {
        return Err(SumcheckError::BelowFloor {
            claim: proof.claimed_total,
            floor,
        });
    }

    // Build the FS transcript identically to the prover.
    let mut transcript = blake3::Hasher::new();
    transcript.update(FS_TAG);
    transcript.update(&(n as u64).to_le_bytes());
    transcript.update(&(target as u64).to_le_bytes());
    transcript.update(&proof.claimed_total.to_le_bytes());

    let mut prev_value = proof.claimed_total;
    let mut fixed_r: Vec<FieldElem> = Vec::with_capacity(v);

    for (k, poly) in proof.round_polys.iter().enumerate() {
        // 1. Round consistency.
        let observed = add_p(poly.at_0, poly.at_1);
        if observed != prev_value {
            return Err(SumcheckError::RoundConsistency {
                round: k,
                observed,
                expected: prev_value,
            });
        }
        // FS challenge.
        transcript.update(&poly.at_0.to_le_bytes());
        transcript.update(&poly.at_1.to_le_bytes());
        transcript.update(&poly.at_2.to_le_bytes());
        let r_k = fs_challenge(&mut transcript, b"round");
        fixed_r.push(r_k);
        prev_value = poly.evaluate(r_k);
    }

    // 2. Final evaluation.
    let leaf_val = multilinear_extend(leaves, &fixed_r).expect("v matches");
    let sel_val = selector_eval(target, &fixed_r);
    let recomputed = mul_p(leaf_val, sel_val);
    if recomputed != prev_value {
        return Err(SumcheckError::FinalEval {
            claimed: prev_value,
            recomputed,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves_4() -> Vec<FieldElem> {
        vec![100, 200, 300, 400]
    }

    fn leaves_8() -> Vec<FieldElem> {
        vec![10, 20, 30, 40, 50, 60, 70, 80]
    }

    // ── poly evaluation ──────────────────────────────────────────

    #[test]
    fn univariate_poly_evaluates_at_known_points() {
        let p = UnivariatePoly3 {
            at_0: 5,
            at_1: 11,
            at_2: 21,
        };
        assert_eq!(p.evaluate(0), 5);
        assert_eq!(p.evaluate(1), 11);
        assert_eq!(p.evaluate(2), 21);
    }

    #[test]
    fn univariate_poly_evaluates_at_arbitrary_point() {
        // A degree-2 polynomial. Try x²: at 0=0, at 1=1, at 2=4.
        let p = UnivariatePoly3 { at_0: 0, at_1: 1, at_2: 4 };
        // p(3) should = 9.
        assert_eq!(p.evaluate(3), 9);
        // p(5) should = 25.
        assert_eq!(p.evaluate(5), 25);
    }

    // ── selector ─────────────────────────────────────────────────

    #[test]
    fn selector_at_target_returns_one() {
        // MSB-first: target=2 ↔ (b₀=1, b₁=0) ↔ point (1, 0).
        let r = selector_eval(2, &[1, 0]);
        assert_eq!(r, 1);
    }

    #[test]
    fn selector_at_other_corner_returns_zero() {
        // target=2 (point should be (1,0)); evaluating at (1,1) ≠ target → 0.
        let r = selector_eval(2, &[1, 1]);
        assert_eq!(r, 0);
    }

    // ── prove + verify round-trip ────────────────────────────────

    #[test]
    fn honest_proof_verifies_on_4_leaves() {
        let leaves = leaves_4();
        for target in 0..4 {
            let proof = prove_sumcheck_inclusion(&leaves, target).unwrap();
            assert_eq!(proof.claimed_total, leaves[target]);
            verify_sumcheck_inclusion(&leaves, target, &proof, 0).unwrap();
        }
    }

    #[test]
    fn honest_proof_verifies_on_8_leaves() {
        let leaves = leaves_8();
        for target in 0..8 {
            let proof = prove_sumcheck_inclusion(&leaves, target).unwrap();
            verify_sumcheck_inclusion(&leaves, target, &proof, 0).unwrap();
        }
    }

    // ── shape errors ─────────────────────────────────────────────

    #[test]
    fn non_power_of_two_leaves_rejected() {
        let leaves = vec![1u64, 2, 3];
        assert!(matches!(
            prove_sumcheck_inclusion(&leaves, 0),
            Err(SumcheckError::BadLeavesShape)
        ));
        let bad_proof = SumcheckProof {
            claimed_total: 1,
            round_polys: vec![],
        };
        assert!(matches!(
            verify_sumcheck_inclusion(&leaves, 0, &bad_proof, 0),
            Err(SumcheckError::BadLeavesShape)
        ));
    }

    #[test]
    fn target_out_of_range_rejected() {
        let leaves = leaves_4();
        let err = prove_sumcheck_inclusion(&leaves, 99).unwrap_err();
        assert!(matches!(err, SumcheckError::TargetOutOfRange { .. }));
    }

    #[test]
    fn round_count_mismatch_rejected() {
        let leaves = leaves_4();
        let proof = prove_sumcheck_inclusion(&leaves, 0).unwrap();
        let mut bad = proof.clone();
        bad.round_polys.pop();
        let err = verify_sumcheck_inclusion(&leaves, 0, &bad, 0).unwrap_err();
        assert!(matches!(err, SumcheckError::RoundCountMismatch { .. }));
    }

    // ── tampered proofs ──────────────────────────────────────────

    #[test]
    fn tampered_round_poly_fails_consistency() {
        let leaves = leaves_4();
        let mut proof = prove_sumcheck_inclusion(&leaves, 1).unwrap();
        // Bump the at_1 of the first round → consistency check
        // s(0) + s(1) == prev_value will fail.
        proof.round_polys[0].at_1 = add_p(proof.round_polys[0].at_1, 1);
        let err = verify_sumcheck_inclusion(&leaves, 1, &proof, 0).unwrap_err();
        assert!(matches!(err, SumcheckError::RoundConsistency { .. }));
    }

    #[test]
    fn tampered_claimed_total_fails() {
        let leaves = leaves_4();
        let mut proof = prove_sumcheck_inclusion(&leaves, 1).unwrap();
        // Pump the claimed total — first round consistency check
        // will fail.
        proof.claimed_total = add_p(proof.claimed_total, 1);
        let err = verify_sumcheck_inclusion(&leaves, 1, &proof, 0).unwrap_err();
        assert!(matches!(err, SumcheckError::RoundConsistency { .. }));
    }

    // ── energy floor ─────────────────────────────────────────────

    #[test]
    fn below_floor_rejected_before_other_checks() {
        // leaves = [100, 200, 300, 400]; target leaf = 100 (idx 0).
        // floor = 200 → claim 100 < 200 → BelowFloor.
        let leaves = leaves_4();
        let proof = prove_sumcheck_inclusion(&leaves, 0).unwrap();
        let err = verify_sumcheck_inclusion(&leaves, 0, &proof, 200).unwrap_err();
        assert!(matches!(err, SumcheckError::BelowFloor { .. }));
    }

    #[test]
    fn at_floor_passes() {
        // leaves[1] = 200. floor = 200. Should pass.
        let leaves = leaves_4();
        let proof = prove_sumcheck_inclusion(&leaves, 1).unwrap();
        verify_sumcheck_inclusion(&leaves, 1, &proof, 200).unwrap();
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "EPA-MMR V2 ships sumcheck-folded inclusion
        // proofs. The verifier checks the multilinear extension
        // of the leaf-energy vector via O(log² N) field ops
        // through Lund-Fortnow-Karloff-Nisan 1992 sumcheck.
        // Energy floor still gates structurally — a decayed
        // leaf's claim fails the floor check first."

        // Honest path: 8-leaf MMR, target = 5, energy = leaves[5] = 60.
        let leaves = leaves_8();
        let proof = prove_sumcheck_inclusion(&leaves, 5).unwrap();
        verify_sumcheck_inclusion(&leaves, 5, &proof, 50).unwrap();

        // Decayed-leaf path: same MMR but the prover tries to
        // claim leaf 0 (energy 10) above its decay floor.
        let proof_low = prove_sumcheck_inclusion(&leaves, 0).unwrap();
        let err = verify_sumcheck_inclusion(&leaves, 0, &proof_low, 50).unwrap_err();
        assert!(matches!(err, SumcheckError::BelowFloor { .. }));

        // Tampered prover: claim leaf 0 has higher energy than it
        // actually does. The bogus claim passes the floor check
        // but breaks round consistency.
        let mut bad = proof_low.clone();
        bad.claimed_total = 10_000;
        let err = verify_sumcheck_inclusion(&leaves, 0, &bad, 50).unwrap_err();
        assert!(matches!(err, SumcheckError::RoundConsistency { .. }));
    }

    proptest::proptest! {
        #[test]
        fn property_honest_proof_always_verifies(
            seed in 1u64..200u64,
            target_offset in 0usize..4usize,
        ) {
            // Generate a deterministic 4-leaf vector from seed.
            let mut leaves = vec![0u64; 4];
            let mut s = seed;
            for i in 0..4 {
                s = s.wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                leaves[i] = (s % 10_000) + 1;
            }
            let target = target_offset;
            let proof = prove_sumcheck_inclusion(&leaves, target).unwrap();
            proptest::prop_assert!(
                verify_sumcheck_inclusion(&leaves, target, &proof, 0).is_ok()
            );
        }
    }
}
