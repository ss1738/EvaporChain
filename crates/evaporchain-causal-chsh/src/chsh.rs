//! CHSH-style four-term correlation S statistic over concurrent block
//! pairs.
//!
//! See crate root for the full theorem statement. This module computes
//! `S = |E(A,B) + E(A,B') + E(A',B) − E(A',B')|` where each `E(·,·)` is
//! the empirical mean of the product of two ±1 observables across a
//! sample drawn under that specific (setting₁, setting₂) condition.
//!
//! ## Why four separate samples, not one quadruple per pair
//!
//! In a Bell experiment, only ONE pair of settings can be measured per
//! pair — counterfactual definiteness (assigning all four observables
//! simultaneously to the same pair) is exactly what Bell's theorem
//! forbids. Translated to blockchain: the chain *picks* which
//! observable to compute on each concurrent pair (e.g., "for this
//! pair I'll bin by total energy"; "for that pair I'll bin by
//! tx-count"). Each setting-pair `(setting_left, setting_right) ∈
//! {(A,B), (A,B'), (A',B), (A',B')}` produces its own sample of ±1
//! products.
//!
//! This is what makes a cartel detectable: a classical hidden-variable
//! source must respect `|S| ≤ 2` no matter how it rigs the four
//! samples. A *communicating* cartel that sees the chain's setting
//! choice can rig each sample independently to push S toward the
//! algebraic max of 4.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Four samples of ±1 products, one per CHSH setting-pair, drawn from
/// concurrent-block pairs in the LightCone DAG.
///
/// Each `samples_*` field is a vector where each entry is the product
/// `obs_left * obs_right` of two binary observables on a single
/// concurrent pair, measured under that setting-pair condition. ±1
/// only. Empty vectors are rejected.
///
/// In quantum-CHSH terms: the chain picks one of two "setting" choices
/// per side (A vs A' on the first block, B vs B' on the second). For
/// each pair sampled under setting-pair (X, Y), record the product of
/// the ±1 readings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcurrentPairSamples {
    /// Sample of `obs_A(first) * obs_B(second)` over concurrent pairs.
    pub samples_ab: Vec<i8>,
    /// Sample of `obs_A(first) * obs_B'(second)`.
    pub samples_ab_prime: Vec<i8>,
    /// Sample of `obs_A'(first) * obs_B(second)`.
    pub samples_a_prime_b: Vec<i8>,
    /// Sample of `obs_A'(first) * obs_B'(second)`.
    pub samples_a_prime_b_prime: Vec<i8>,
}

/// Legacy alias retained for callers that accept "one quadruple per
/// pair" — the per-pair shape can still be useful for *honest-source*
/// analysis where the chain can compute all four observables (classical
/// LHV regime). The `compute_chsh_s_classical` helper at the bottom of
/// this module accepts that shape and is bounded by `S ≤ 2`
/// unconditionally.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ConcurrentPair {
    pub a: i8,
    pub a_prime: i8,
    pub b: i8,
    pub b_prime: i8,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChshError {
    #[error("empty sample for setting-pair {pair_label} — at least one observation required")]
    EmptySample { pair_label: &'static str },
    #[error(
        "observable out of range — must be ±1; got value {value} at index {index} of \
         setting-pair {pair_label}"
    )]
    NonBinaryObservable {
        pair_label: &'static str,
        index: usize,
        value: i8,
    },
}

/// Compute the CHSH-style S statistic over four samples of ±1 products,
/// one per setting-pair. Returns `S ∈ [0, 4]`.
///
/// `|S| ≤ 2` under honest validators + LightCone causality (proposed
/// theorem). `S > 2` is the cartel-detection signal.
///
/// **Off-consensus only.** This function returns `f64` and is used by
/// off-chain operator tools (renderer, sweep CLI, off-chain detection).
/// **For any consensus-bearing path (Bell-Beacon V2 certificate, fork
/// rules), use [`compute_chsh_s_milli`] instead** — f64 isn't bitwise
/// deterministic across architectures.
pub fn compute_chsh_s(samples: &ConcurrentPairSamples) -> Result<f64, ChshError> {
    let e_ab = mean_of_pm1(&samples.samples_ab, "AB")?;
    let e_abp = mean_of_pm1(&samples.samples_ab_prime, "AB'")?;
    let e_apb = mean_of_pm1(&samples.samples_a_prime_b, "A'B")?;
    let e_apbp = mean_of_pm1(&samples.samples_a_prime_b_prime, "A'B'")?;

    Ok((e_ab + e_abp + e_apb - e_apbp).abs())
}

/// Pure-integer CHSH S statistic in milli-units (×1000). Returns
/// `|S * 1000|` as `i64`, range `[0, 4000]`.
///
/// **Validator-deterministic.** All arithmetic is i64/i128; samples are
/// `i8 ∈ {-1, +1}`; per-bucket means use truncating integer division
/// (matches the Rust `f64 as i64` cast for the bounded values produced
/// here). Use this on every consensus path that needs CHSH S — Bell-
/// Beacon V2 certificate derivation, fork-choice rules, slashing.
pub fn compute_chsh_s_milli(samples: &ConcurrentPairSamples) -> Result<i64, ChshError> {
    let e_ab = milli_mean_of_pm1(&samples.samples_ab, "AB")?;
    let e_abp = milli_mean_of_pm1(&samples.samples_ab_prime, "AB'")?;
    let e_apb = milli_mean_of_pm1(&samples.samples_a_prime_b, "A'B")?;
    let e_apbp = milli_mean_of_pm1(&samples.samples_a_prime_b_prime, "A'B'")?;
    // Sum is bounded by 4 * 1000 = 4000 → comfortably fits i64.
    let s_signed = (e_ab as i128) + (e_abp as i128) + (e_apb as i128) - (e_apbp as i128);
    Ok(s_signed.unsigned_abs() as i64)
}

/// Compute the CHSH S statistic for a CLASSICAL local-hidden-variable
/// source where the chain has access to all four observables per pair.
/// Provably bounded by 2 (Bell). Used for honest-source analysis +
/// regression checks on the math impl.
pub fn compute_chsh_s_classical(pairs: &[ConcurrentPair]) -> Result<f64, ChshError> {
    if pairs.is_empty() {
        return Err(ChshError::EmptySample { pair_label: "all" });
    }
    for (i, p) in pairs.iter().enumerate() {
        for (lbl, v) in [
            ("AB", p.a),
            ("AB", p.b),
            ("AB'", p.b_prime),
            ("A'B", p.a_prime),
        ] {
            if !is_binary(v) {
                return Err(ChshError::NonBinaryObservable {
                    pair_label: lbl,
                    index: i,
                    value: v,
                });
            }
        }
    }
    let n = pairs.len() as f64;
    let mut sum_ab: i64 = 0;
    let mut sum_abp: i64 = 0;
    let mut sum_apb: i64 = 0;
    let mut sum_apbp: i64 = 0;
    for p in pairs {
        sum_ab += (p.a as i64) * (p.b as i64);
        sum_abp += (p.a as i64) * (p.b_prime as i64);
        sum_apb += (p.a_prime as i64) * (p.b as i64);
        sum_apbp += (p.a_prime as i64) * (p.b_prime as i64);
    }
    let s =
        (sum_ab as f64 / n + sum_abp as f64 / n + sum_apb as f64 / n - sum_apbp as f64 / n).abs();
    Ok(s)
}

fn mean_of_pm1(samples: &[i8], pair_label: &'static str) -> Result<f64, ChshError> {
    if samples.is_empty() {
        return Err(ChshError::EmptySample { pair_label });
    }
    let mut sum: i64 = 0;
    for (i, v) in samples.iter().enumerate() {
        if !is_binary(*v) {
            return Err(ChshError::NonBinaryObservable {
                pair_label,
                index: i,
                value: *v,
            });
        }
        sum += *v as i64;
    }
    Ok(sum as f64 / samples.len() as f64)
}

/// Pure-integer per-bucket mean × 1000. Validates ±1 inputs, bails on
/// empty bucket. Result is in `[-1000, 1000]` for valid inputs since
/// `|sum| ≤ len`.
fn milli_mean_of_pm1(samples: &[i8], pair_label: &'static str) -> Result<i64, ChshError> {
    if samples.is_empty() {
        return Err(ChshError::EmptySample { pair_label });
    }
    let mut sum: i64 = 0;
    for (i, v) in samples.iter().enumerate() {
        if !is_binary(*v) {
            return Err(ChshError::NonBinaryObservable {
                pair_label,
                index: i,
                value: *v,
            });
        }
        sum += *v as i64;
    }
    // Truncating integer division. Matches Rust's `f64 as i64` cast for
    // the bounded values produced by f64-path mean_of_pm1 when the
    // float sum is exactly representable — but never silently rounds.
    Ok(sum.saturating_mul(1000) / samples.len() as i64)
}

#[inline]
fn is_binary(v: i8) -> bool {
    v == 1 || v == -1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(ab: Vec<i8>, abp: Vec<i8>, apb: Vec<i8>, apbp: Vec<i8>) -> ConcurrentPairSamples {
        ConcurrentPairSamples {
            samples_ab: ab,
            samples_ab_prime: abp,
            samples_a_prime_b: apb,
            samples_a_prime_b_prime: apbp,
        }
    }

    #[test]
    fn empty_sample_errs() {
        let s = samples(vec![], vec![1], vec![1], vec![1]);
        assert!(matches!(
            compute_chsh_s(&s),
            Err(ChshError::EmptySample { pair_label: "AB" })
        ));
    }

    #[test]
    fn non_binary_observable_rejected() {
        let s = samples(vec![1, 0, 1], vec![1], vec![1], vec![1]); // 0 invalid
        assert!(matches!(
            compute_chsh_s(&s),
            Err(ChshError::NonBinaryObservable { .. })
        ));
    }

    #[test]
    fn classical_bound_holds_for_lhv_source() {
        // Classical local hidden-variable source: assign a, a', b, b'
        // independently per pair, then compute the four products
        // PER PAIR and aggregate via the classical helper.
        // S ≤ 2 by Bell's theorem.
        let lhv_pairs = vec![
            ConcurrentPair {
                a: 1,
                a_prime: 1,
                b: 1,
                b_prime: 1,
            },
            ConcurrentPair {
                a: 1,
                a_prime: -1,
                b: 1,
                b_prime: -1,
            },
            ConcurrentPair {
                a: -1,
                a_prime: 1,
                b: -1,
                b_prime: 1,
            },
            ConcurrentPair {
                a: -1,
                a_prime: -1,
                b: -1,
                b_prime: -1,
            },
        ];
        let s = compute_chsh_s_classical(&lhv_pairs).unwrap();
        assert!(s <= 2.0 + 1e-9, "Bell bound: S ≤ 2, got {}", s);
    }

    #[test]
    fn cartel_can_violate_classical_bound_via_independent_samples() {
        // Cartel model: each setting-pair sample is rigged
        // independently. Specifically, cartel forces:
        //   E(A,B) = +1, E(A,B') = +1, E(A',B) = +1, E(A',B') = −1
        // → S = |1 + 1 + 1 − (−1)| = 4 (algebraic max for ±1).
        //
        // This is impossible under LHV (Bell) because LHV requires the
        // four samples to share an underlying joint distribution — a
        // classical source can't independently rig the four marginals.
        // A communicating cartel CAN.
        let s_samples = samples(
            vec![1, 1, 1, 1],     // E(A,B) = 1
            vec![1, 1, 1, 1],     // E(A,B') = 1
            vec![1, 1, 1, 1],     // E(A',B) = 1
            vec![-1, -1, -1, -1], // E(A',B') = -1
        );
        let s = compute_chsh_s(&s_samples).unwrap();
        assert!(s > 2.0, "cartel can violate, got S={}", s);
        assert!((s - 4.0).abs() < 1e-9, "expected S=4, got {}", s);
    }

    #[test]
    fn honest_independent_samples_yield_s_near_zero() {
        // Each setting-pair sample is balanced: equal +1 / −1 entries.
        // All four E(·,·) → 0 in the limit; S → 0.
        let bal = vec![1, -1, 1, -1, 1, -1, 1, -1];
        let s = compute_chsh_s(&samples(bal.clone(), bal.clone(), bal.clone(), bal)).unwrap();
        assert!(s.abs() < 1e-9, "expected S≈0, got {}", s);
    }

    // ── milli (integer, validator-deterministic) ───────────────────

    #[test]
    fn milli_max_violation_is_4000() {
        // Cartel pattern: S = 4 → S_milli = 4000 exactly.
        let s = samples(
            vec![1, 1, 1, 1],
            vec![1, 1, 1, 1],
            vec![1, 1, 1, 1],
            vec![-1, -1, -1, -1],
        );
        assert_eq!(compute_chsh_s_milli(&s).unwrap(), 4000);
    }

    #[test]
    fn milli_balanced_honest_is_zero() {
        let bal = vec![1, -1, 1, -1, 1, -1, 1, -1];
        let s = compute_chsh_s_milli(&samples(bal.clone(), bal.clone(), bal.clone(), bal)).unwrap();
        assert_eq!(s, 0);
    }

    #[test]
    fn milli_empty_bucket_errs() {
        let s = samples(vec![], vec![1], vec![1], vec![1]);
        assert!(matches!(
            compute_chsh_s_milli(&s),
            Err(ChshError::EmptySample { pair_label: "AB" })
        ));
    }

    #[test]
    fn milli_non_binary_rejected() {
        let s = samples(vec![1, 2, 1], vec![1], vec![1], vec![1]);
        assert!(matches!(
            compute_chsh_s_milli(&s),
            Err(ChshError::NonBinaryObservable { .. })
        ));
    }

    #[test]
    fn milli_is_validator_deterministic() {
        // Same inputs → byte-identical output across calls.
        let s = samples(
            vec![1, -1, 1, -1, 1],
            vec![1, 1, -1, -1, 1],
            vec![-1, 1, 1, -1, 1],
            vec![1, 1, 1, -1, -1],
        );
        let a = compute_chsh_s_milli(&s).unwrap();
        let b = compute_chsh_s_milli(&s).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn milli_truncates_toward_zero_on_uneven_buckets() {
        // 7 samples summing to 1 → mean = 1/7 ≈ 0.1428571. milli = 142
        // (truncated). The f64 path would give 142 also (1000/7 = 142.857
        // → as i64 = 142), so this round-trips with f64 for this case.
        // Build a single-bucket scenario by zeroing the other three
        // buckets via balanced ±1 (each contributes 0).
        let bal = vec![1, -1, 1, -1];
        let s = samples(
            vec![1, 1, 1, 1, -1, -1, -1], // sum = 1, len = 7 → milli = 142
            bal.clone(),
            bal.clone(),
            bal,
        );
        // S_milli = |142 + 0 + 0 - 0| = 142
        assert_eq!(compute_chsh_s_milli(&s).unwrap(), 142);
    }

    use proptest::prelude::*;

    proptest! {
        /// Algebraic max: any four samples of ±1 products give
        /// `S ∈ [0, 4]`. Proof of the math impl over arbitrary inputs.
        #[test]
        fn s_is_in_range_0_to_4(
            ab_signs in proptest::collection::vec(any::<bool>(), 1..32),
            abp_signs in proptest::collection::vec(any::<bool>(), 1..32),
            apb_signs in proptest::collection::vec(any::<bool>(), 1..32),
            apbp_signs in proptest::collection::vec(any::<bool>(), 1..32),
        ) {
            let to_pm1 = |v: Vec<bool>| v.into_iter().map(|b| if b { 1i8 } else { -1 }).collect::<Vec<_>>();
            let s_samples = ConcurrentPairSamples {
                samples_ab: to_pm1(ab_signs),
                samples_ab_prime: to_pm1(abp_signs),
                samples_a_prime_b: to_pm1(apb_signs),
                samples_a_prime_b_prime: to_pm1(apbp_signs),
            };
            let s = compute_chsh_s(&s_samples).unwrap();
            prop_assert!(s >= 0.0);
            prop_assert!(s <= 4.0 + 1e-9);
        }

        /// Bell's theorem: classical LHV sources (where all four
        /// observables can be assigned per pair simultaneously)
        /// satisfy `S ≤ 2`. This is the load-bearing theorem the
        /// crate's gate test relies on as the "honest baseline."
        #[test]
        fn lhv_source_satisfies_bell_bound(
            pairs in proptest::collection::vec(
                (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()),
                1..32,
            ),
        ) {
            let pairs: Vec<ConcurrentPair> = pairs
                .into_iter()
                .map(|(a, ap, b, bp)| ConcurrentPair {
                    a: if a { 1 } else { -1 },
                    a_prime: if ap { 1 } else { -1 },
                    b: if b { 1 } else { -1 },
                    b_prime: if bp { 1 } else { -1 },
                })
                .collect();
            let s = compute_chsh_s_classical(&pairs).unwrap();
            prop_assert!(s <= 2.0 + 1e-9, "Bell bound violated by LHV source: {}", s);
        }

        /// Lane O.10 cross-validation: the per-pair LHV API
        /// (`compute_chsh_s_classical`) and the four-sample API
        /// (`compute_chsh_s`) must agree on classical inputs.
        ///
        /// Given LHV pairs `[(a_i, a'_i, b_i, b'_i)]`, expand to four
        /// samples by computing the four products PER PAIR:
        ///   samples_ab[i]      = a_i · b_i
        ///   samples_ab_prime[i] = a_i · b'_i
        ///   samples_a_prime_b[i] = a'_i · b_i
        ///   samples_a_prime_b_prime[i] = a'_i · b'_i
        /// Then `compute_chsh_s(expanded) == compute_chsh_s_classical(pairs)`
        /// because the per-bucket means are identical (each bucket
        /// averages N samples, where N is the original pair count).
        ///
        /// This is the algebraic bridge between the two APIs: the
        /// classical helper is a special case of the four-sample
        /// helper where the four samples are PERFECTLY CORRELATED via
        /// a shared underlying LHV — which is exactly what Bell's
        /// theorem says forces `S ≤ 2`.
        #[test]
        fn classical_helper_matches_four_sample_helper_on_lhv_input(
            pairs in proptest::collection::vec(
                (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()),
                1..32,
            ),
        ) {
            let pairs: Vec<ConcurrentPair> = pairs
                .into_iter()
                .map(|(a, ap, b, bp)| ConcurrentPair {
                    a: if a { 1 } else { -1 },
                    a_prime: if ap { 1 } else { -1 },
                    b: if b { 1 } else { -1 },
                    b_prime: if bp { 1 } else { -1 },
                })
                .collect();

            // Reference: classical helper.
            let s_ref = compute_chsh_s_classical(&pairs).unwrap();

            // Cross-validate: expand to four samples + run four-sample
            // helper. Result MUST equal the classical reference.
            let expanded = ConcurrentPairSamples {
                samples_ab: pairs.iter().map(|p| p.a * p.b).collect(),
                samples_ab_prime: pairs.iter().map(|p| p.a * p.b_prime).collect(),
                samples_a_prime_b: pairs.iter().map(|p| p.a_prime * p.b).collect(),
                samples_a_prime_b_prime: pairs.iter().map(|p| p.a_prime * p.b_prime).collect(),
            };
            let s_xval = compute_chsh_s(&expanded).unwrap();

            // Bit-equality not possible across two sums-then-divide
            // paths (FP rounding); 1e-12 tolerance is generous.
            prop_assert!(
                (s_ref - s_xval).abs() < 1e-12,
                "classical and four-sample helpers must agree: ref={} xval={}",
                s_ref,
                s_xval
            );

            // Bonus: as a corollary, the four-sample helper on
            // expanded LHV input also satisfies Bell's bound.
            prop_assert!(s_xval <= 2.0 + 1e-9, "expanded LHV must satisfy Bell: {}", s_xval);
        }
    }
}
