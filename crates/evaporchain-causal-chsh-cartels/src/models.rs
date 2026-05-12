//! Three deterministic cartel-injection models.
//!
//! All three accept a `ConcurrentPairSamples` (the honest sample, in
//! the same shape as Lane O.2's runner output) plus a `[u8; 32]` seed
//! plus a rational parameter, and return a fresh
//! `ConcurrentPairSamples` of the same length per bucket.
//!
//! ## Cartel pattern
//!
//! Every model targets the same algebraic-max violation pattern that
//! the V1 synthetic gate tests use:
//!
//! - setting-pair (A,B)        → +1
//! - setting-pair (A,B')       → +1
//! - setting-pair (A',B)       → +1
//! - setting-pair (A',B')      → −1
//!
//! Plug all four into `compute_chsh_s` and you get S = 4 (algebraic
//! maximum over ±1 observables; well past Tsirelson's 2√2).

use evaporchain_causal_chsh::chsh::ConcurrentPairSamples;
use thiserror::Error;

use crate::rng::Blake3Rng;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CartelError {
    #[error("honest sample contains a non-binary observable: {0}")]
    NonBinaryHonestSample(i8),
    #[error("a setting-pair bucket is empty — cartel injection requires non-empty samples in all four buckets")]
    EmptyBucket,
    #[error("invalid rational parameter: numerator ({n}) > denominator ({d})")]
    NumeratorTooLarge { n: u64, d: u64 },
    #[error("invalid rational parameter: denominator must be > 0")]
    ZeroDenominator,
}

/// The cartel target value for each setting-pair (algebraic-max
/// violation pattern, S = 4).
fn cartel_value_ab() -> i8 {
    1
}
fn cartel_value_ab_prime() -> i8 {
    1
}
fn cartel_value_a_prime_b() -> i8 {
    1
}
fn cartel_value_a_prime_b_prime() -> i8 {
    -1
}

fn check_binary(v: i8) -> Result<(), CartelError> {
    if v == 1 || v == -1 {
        Ok(())
    } else {
        Err(CartelError::NonBinaryHonestSample(v))
    }
}

fn check_rational(n: u64, d: u64) -> Result<(), CartelError> {
    if d == 0 {
        return Err(CartelError::ZeroDenominator);
    }
    if n > d {
        return Err(CartelError::NumeratorTooLarge { n, d });
    }
    Ok(())
}

fn check_non_empty(s: &ConcurrentPairSamples) -> Result<(), CartelError> {
    if s.samples_ab.is_empty()
        || s.samples_ab_prime.is_empty()
        || s.samples_a_prime_b.is_empty()
        || s.samples_a_prime_b_prime.is_empty()
    {
        return Err(CartelError::EmptyBucket);
    }
    Ok(())
}

fn check_all_binary(s: &ConcurrentPairSamples) -> Result<(), CartelError> {
    for v in s
        .samples_ab
        .iter()
        .chain(s.samples_ab_prime.iter())
        .chain(s.samples_a_prime_b.iter())
        .chain(s.samples_a_prime_b_prime.iter())
    {
        check_binary(*v)?;
    }
    Ok(())
}

// ── Model 1 — Coordinated subset ─────────────────────────────────────

/// In each of the four buckets independently, replace
/// `floor(num · len / den)` randomly-chosen positions with the cartel
/// value for that bucket. The remaining positions keep the honest
/// sample. Random positions are drawn deterministically from the seed.
///
/// - `num=0`  → output = honest (no replacement).
/// - `num=den` → output = full cartel (every position replaced).
pub fn coordinated_subset_cartel(
    honest: &ConcurrentPairSamples,
    num: u64,
    den: u64,
    seed: [u8; 32],
) -> Result<ConcurrentPairSamples, CartelError> {
    check_rational(num, den)?;
    check_non_empty(honest)?;
    check_all_binary(honest)?;

    let mut rng = Blake3Rng::new(b"evaporchain:cartel:coordinated-subset:v1", &seed);

    let inject = |bucket_tag: &[u8], samples: &[i8], cartel_v: i8, rng: &mut Blake3Rng| {
        let len = samples.len() as u64;
        let k = (num.saturating_mul(len)) / den;
        let mut out: Vec<i8> = samples.to_vec();
        // Fisher-Yates partial shuffle: pick k distinct indices.
        let mut indices: Vec<usize> = (0..samples.len()).collect();
        // Domain-separate per bucket so the same seed produces
        // different position sets across the four buckets.
        let mut sub = Blake3Rng::new(bucket_tag, &derive_seed(rng));
        let n = indices.len();
        let k = k as usize;
        for i in 0..k.min(n) {
            let swap_idx = i + sub.next_in_range((n - i) as u64) as usize;
            indices.swap(i, swap_idx);
            out[indices[i]] = cartel_v;
        }
        out
    };

    let samples_ab = inject(
        b"bucket:ab",
        &honest.samples_ab,
        cartel_value_ab(),
        &mut rng,
    );
    let samples_ab_prime = inject(
        b"bucket:ab_prime",
        &honest.samples_ab_prime,
        cartel_value_ab_prime(),
        &mut rng,
    );
    let samples_a_prime_b = inject(
        b"bucket:a_prime_b",
        &honest.samples_a_prime_b,
        cartel_value_a_prime_b(),
        &mut rng,
    );
    let samples_a_prime_b_prime = inject(
        b"bucket:a_prime_b_prime",
        &honest.samples_a_prime_b_prime,
        cartel_value_a_prime_b_prime(),
        &mut rng,
    );

    Ok(ConcurrentPairSamples {
        samples_ab,
        samples_ab_prime,
        samples_a_prime_b,
        samples_a_prime_b_prime,
    })
}

fn derive_seed(rng: &mut Blake3Rng) -> [u8; 32] {
    let mut s = [0u8; 32];
    for byte in &mut s {
        *byte = rng.next_u8();
    }
    s
}

// ── Model 2 — Biased coin ───────────────────────────────────────────

/// For every sample independently, with probability `num/den` use the
/// cartel value; otherwise keep the honest sample.
///
/// - `num=0`  → output = honest (probability 0 of replacement).
/// - `num=den` → output = full cartel (probability 1 of replacement).
///
/// The honest distribution leaks through at rate `1 − num/den`. The
/// cartel value never appears in the unrigged samples; if the honest
/// sample happens to equal the cartel value, no replacement is
/// observable.
pub fn biased_coin_cartel(
    honest: &ConcurrentPairSamples,
    num: u64,
    den: u64,
    seed: [u8; 32],
) -> Result<ConcurrentPairSamples, CartelError> {
    check_rational(num, den)?;
    check_non_empty(honest)?;
    check_all_binary(honest)?;

    let mut rng = Blake3Rng::new(b"evaporchain:cartel:biased-coin:v1", &seed);

    let mut flip = |samples: &[i8], cartel_v: i8| -> Vec<i8> {
        samples
            .iter()
            .map(|&h| if rng.bernoulli(num, den) { cartel_v } else { h })
            .collect()
    };

    let samples_ab = flip(&honest.samples_ab, cartel_value_ab());
    let samples_ab_prime = flip(&honest.samples_ab_prime, cartel_value_ab_prime());
    let samples_a_prime_b = flip(&honest.samples_a_prime_b, cartel_value_a_prime_b());
    let samples_a_prime_b_prime = flip(
        &honest.samples_a_prime_b_prime,
        cartel_value_a_prime_b_prime(),
    );

    Ok(ConcurrentPairSamples {
        samples_ab,
        samples_ab_prime,
        samples_a_prime_b,
        samples_a_prime_b_prime,
    })
}

// ── Model 3 — PR-box approximation ──────────────────────────────────

/// White-noise mixture with the Popescu-Rohrlich box at visibility
/// `v = num/den`.
///
/// For every sample independently:
/// - with probability `v`, output the PR-box (cartel) value;
/// - with probability `1 − v`, output a uniformly-random ±1 sample
///   (white noise; *not* the honest sample).
///
/// Theoretical S as a function of v: S = 4·v.
/// - v=1.0 → S=4 (algebraic max).
/// - v ≈ 0.707 → S ≈ 2√2 (Tsirelson bound; quantum max).
/// - v=0.5 → S=2 (classical bound; gate tie at honest_ceiling+δ).
/// - v=0 → S ≈ 0 (pure white noise).
pub fn pr_box_cartel(
    honest: &ConcurrentPairSamples,
    visibility_num: u64,
    visibility_den: u64,
    seed: [u8; 32],
) -> Result<ConcurrentPairSamples, CartelError> {
    check_rational(visibility_num, visibility_den)?;
    check_non_empty(honest)?;
    // No need to check honest binary — PR-box doesn't read honest values.

    let mut rng = Blake3Rng::new(b"evaporchain:cartel:pr-box:v1", &seed);

    let mut sample = |len: usize, cartel_v: i8| -> Vec<i8> {
        (0..len)
            .map(|_| {
                if rng.bernoulli(visibility_num, visibility_den) {
                    cartel_v
                } else if rng.bernoulli(1, 2) {
                    1
                } else {
                    -1
                }
            })
            .collect()
    };

    let samples_ab = sample(honest.samples_ab.len(), cartel_value_ab());
    let samples_ab_prime = sample(honest.samples_ab_prime.len(), cartel_value_ab_prime());
    let samples_a_prime_b = sample(honest.samples_a_prime_b.len(), cartel_value_a_prime_b());
    let samples_a_prime_b_prime = sample(
        honest.samples_a_prime_b_prime.len(),
        cartel_value_a_prime_b_prime(),
    );

    Ok(ConcurrentPairSamples {
        samples_ab,
        samples_ab_prime,
        samples_a_prime_b,
        samples_a_prime_b_prime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_causal_chsh::chsh::compute_chsh_s;

    fn balanced_honest(n_each: usize) -> ConcurrentPairSamples {
        // Balanced ±1 in each bucket → every E(·,·) = 0 → S = 0.
        let bal: Vec<i8> = (0..n_each)
            .map(|i| if i % 2 == 0 { 1 } else { -1 })
            .collect();
        ConcurrentPairSamples {
            samples_ab: bal.clone(),
            samples_ab_prime: bal.clone(),
            samples_a_prime_b: bal.clone(),
            samples_a_prime_b_prime: bal,
        }
    }

    fn seed(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    // ── invariants common to all three models ────────────────────

    #[test]
    fn all_models_preserve_per_bucket_lengths() {
        let h = balanced_honest(64);
        let c1 = coordinated_subset_cartel(&h, 1, 4, seed(1)).unwrap();
        let c2 = biased_coin_cartel(&h, 1, 4, seed(1)).unwrap();
        let c3 = pr_box_cartel(&h, 1, 4, seed(1)).unwrap();
        for c in [&c1, &c2, &c3] {
            assert_eq!(c.samples_ab.len(), h.samples_ab.len());
            assert_eq!(c.samples_ab_prime.len(), h.samples_ab_prime.len());
            assert_eq!(c.samples_a_prime_b.len(), h.samples_a_prime_b.len());
            assert_eq!(
                c.samples_a_prime_b_prime.len(),
                h.samples_a_prime_b_prime.len()
            );
        }
    }

    #[test]
    fn all_models_emit_only_pm1() {
        let h = balanced_honest(48);
        let c1 = coordinated_subset_cartel(&h, 3, 7, seed(2)).unwrap();
        let c2 = biased_coin_cartel(&h, 3, 7, seed(2)).unwrap();
        let c3 = pr_box_cartel(&h, 3, 7, seed(2)).unwrap();
        for c in [&c1, &c2, &c3] {
            for v in c
                .samples_ab
                .iter()
                .chain(c.samples_ab_prime.iter())
                .chain(c.samples_a_prime_b.iter())
                .chain(c.samples_a_prime_b_prime.iter())
            {
                assert!(*v == 1 || *v == -1);
            }
        }
    }

    #[test]
    fn all_models_validator_deterministic() {
        let h = balanced_honest(32);
        let s = seed(99);
        let a1 = coordinated_subset_cartel(&h, 5, 9, s).unwrap();
        let a2 = coordinated_subset_cartel(&h, 5, 9, s).unwrap();
        let b1 = biased_coin_cartel(&h, 5, 9, s).unwrap();
        let b2 = biased_coin_cartel(&h, 5, 9, s).unwrap();
        let c1 = pr_box_cartel(&h, 5, 9, s).unwrap();
        let c2 = pr_box_cartel(&h, 5, 9, s).unwrap();
        assert_eq!(a1.samples_ab, a2.samples_ab);
        assert_eq!(b1.samples_ab, b2.samples_ab);
        assert_eq!(c1.samples_ab, c2.samples_ab);
    }

    #[test]
    fn rejects_invalid_rational() {
        let h = balanced_honest(8);
        assert_eq!(
            coordinated_subset_cartel(&h, 5, 4, seed(0)).unwrap_err(),
            CartelError::NumeratorTooLarge { n: 5, d: 4 }
        );
        assert_eq!(
            biased_coin_cartel(&h, 1, 0, seed(0)).unwrap_err(),
            CartelError::ZeroDenominator
        );
        assert_eq!(
            pr_box_cartel(&h, 7, 5, seed(0)).unwrap_err(),
            CartelError::NumeratorTooLarge { n: 7, d: 5 }
        );
    }

    #[test]
    fn rejects_empty_bucket() {
        let mut h = balanced_honest(4);
        h.samples_ab.clear();
        assert_eq!(
            coordinated_subset_cartel(&h, 1, 2, seed(0)).unwrap_err(),
            CartelError::EmptyBucket
        );
        assert_eq!(
            biased_coin_cartel(&h, 1, 2, seed(0)).unwrap_err(),
            CartelError::EmptyBucket
        );
        assert_eq!(
            pr_box_cartel(&h, 1, 2, seed(0)).unwrap_err(),
            CartelError::EmptyBucket
        );
    }

    // ── coordinated_subset specifics ─────────────────────────────

    #[test]
    fn coordinated_zero_returns_honest() {
        let h = balanced_honest(20);
        let c = coordinated_subset_cartel(&h, 0, 1, seed(1)).unwrap();
        assert_eq!(c.samples_ab, h.samples_ab);
        assert_eq!(c.samples_ab_prime, h.samples_ab_prime);
        assert_eq!(c.samples_a_prime_b, h.samples_a_prime_b);
        assert_eq!(c.samples_a_prime_b_prime, h.samples_a_prime_b_prime);
    }

    #[test]
    fn coordinated_full_yields_max_violation() {
        let h = balanced_honest(20);
        let c = coordinated_subset_cartel(&h, 1, 1, seed(1)).unwrap();
        let s = compute_chsh_s(&c).unwrap();
        assert!((s - 4.0).abs() < 1e-9);
    }

    #[test]
    fn coordinated_replacement_count_matches_floor() {
        // 3/4 of 20 = 15 replacements per bucket.
        let h = balanced_honest(20);
        let c = coordinated_subset_cartel(&h, 3, 4, seed(7)).unwrap();
        let cartel_v = cartel_value_ab(); // = +1
        let count_replaced = c.samples_ab.iter().filter(|&&v| v == cartel_v).count();
        // Honest sample has 10 +1s already; replacements turn some −1s
        // into +1s. With k=15 replacements drawn over the 20 positions,
        // at least the 10 that were −1 in honest must be flipped, so
        // we expect ≥15 +1s in the output (could be all 20 if every
        // chosen index happened to be a −1, which is not guaranteed).
        assert!(count_replaced >= 15);
        // Also: count of −1 in output = 20 − (replaced + (honest +1
        // not replaced)). With 15 replacements drawn deterministically,
        // there are exactly 5 untouched positions; among those, half
        // are +1 and half −1 in expectation. We assert there is at
        // least one untouched −1 left to confirm partial mode.
        let count_minus = c.samples_ab.iter().filter(|&&v| v == -1).count();
        assert!(count_minus <= 5);
    }

    // ── biased_coin specifics ────────────────────────────────────

    #[test]
    fn biased_coin_zero_returns_honest() {
        let h = balanced_honest(40);
        let c = biased_coin_cartel(&h, 0, 1, seed(1)).unwrap();
        assert_eq!(c.samples_ab, h.samples_ab);
    }

    #[test]
    fn biased_coin_full_yields_max_violation() {
        let h = balanced_honest(40);
        let c = biased_coin_cartel(&h, 1, 1, seed(1)).unwrap();
        let s = compute_chsh_s(&c).unwrap();
        assert!((s - 4.0).abs() < 1e-9);
    }

    #[test]
    fn biased_coin_partial_increases_s_monotonically() {
        // Honest balanced → S=0. Increasing bias should increase S.
        let h = balanced_honest(2_000);
        let s_low = compute_chsh_s(&biased_coin_cartel(&h, 1, 4, seed(11)).unwrap()).unwrap();
        let s_mid = compute_chsh_s(&biased_coin_cartel(&h, 1, 2, seed(11)).unwrap()).unwrap();
        let s_high = compute_chsh_s(&biased_coin_cartel(&h, 3, 4, seed(11)).unwrap()).unwrap();
        assert!(s_low < s_mid);
        assert!(s_mid < s_high);
        assert!(s_high < 4.0);
    }

    // ── pr_box specifics ─────────────────────────────────────────

    #[test]
    fn pr_box_visibility_one_yields_max_violation() {
        let h = balanced_honest(40);
        let c = pr_box_cartel(&h, 1, 1, seed(1)).unwrap();
        let s = compute_chsh_s(&c).unwrap();
        assert!((s - 4.0).abs() < 1e-9);
    }

    #[test]
    fn pr_box_visibility_zero_yields_classical() {
        // Pure white noise: every sample uniformly ±1. Expected S ≈ 0;
        // for n=8000 samples per bucket, |S| < 0.2 is comfortable.
        let h = balanced_honest(8_000);
        let c = pr_box_cartel(&h, 0, 1, seed(33)).unwrap();
        let s = compute_chsh_s(&c).unwrap();
        assert!(s.abs() < 0.2, "S = {s}");
    }

    #[test]
    fn pr_box_s_tracks_4v_within_tolerance() {
        // S = 4·v in expectation. v = 0.5 → S ≈ 2.0.
        let h = balanced_honest(8_000);
        let c = pr_box_cartel(&h, 1, 2, seed(55)).unwrap();
        let s = compute_chsh_s(&c).unwrap();
        assert!((s - 2.0).abs() < 0.2, "S = {s}, expected ~2.0");
    }

    #[test]
    fn pr_box_at_tsirelson_visibility() {
        // 707/1000 ≈ 1/√2. Expected S ≈ 4·0.707 = 2.828 (Tsirelson).
        let h = balanced_honest(8_000);
        let c = pr_box_cartel(&h, 707, 1000, seed(77)).unwrap();
        let s = compute_chsh_s(&c).unwrap();
        assert!((s - 2.828).abs() < 0.2, "S = {s}, expected ~2.828");
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Lane O.3 ships three deterministic cartel-injection
        // models. Coordinated-subset rigs k of n pairs; biased-coin
        // flips each sample with probability p; PR-box approximates
        // the Popescu-Rohrlich box at visibility v with white-noise
        // mixture. All three are validator-deterministic via
        // BLAKE3-keyed XOF — same seed, same output on every node.
        // At full strength (k=n, p=1, v=1) all three reach S = 4."
        let h = balanced_honest(64);
        let s_coord =
            compute_chsh_s(&coordinated_subset_cartel(&h, 1, 1, seed(0)).unwrap()).unwrap();
        let s_bias = compute_chsh_s(&biased_coin_cartel(&h, 1, 1, seed(0)).unwrap()).unwrap();
        let s_pr = compute_chsh_s(&pr_box_cartel(&h, 1, 1, seed(0)).unwrap()).unwrap();
        for s in [s_coord, s_bias, s_pr] {
            assert!((s - 4.0).abs() < 1e-9);
        }

        // Determinism: same seed → byte-identical output.
        let a = biased_coin_cartel(&h, 3, 7, seed(123)).unwrap();
        let b = biased_coin_cartel(&h, 3, 7, seed(123)).unwrap();
        assert_eq!(a.samples_ab, b.samples_ab);
    }
}
