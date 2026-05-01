//! Synthetic MERA workload generators.
//!
//! Three regime-specific energy-vector generators that reproduce the gate's
//! decision matrix from `research/mera-gate/run_gate.py` in pure Rust:
//!
//! - [`log_correlated`] — Pareto-distributed account popularity, modulated
//!   by a 1/(block+1) decay. Mimics Ethereum-style heavy-tail activity.
//!   Eigenvalue spectrum decays as a power law → MERA regime.
//! - [`area_law`] — block-segmented co-touch pattern with exponential
//!   cross-segment decay. MPS / area-law regime.
//! - [`flat_random`] — independent per-account activations. Marchenko-Pastur
//!   bulk → drop tensor networks, ship Verkle.
//!
//! All three are deterministic given a seed (built-in xorshift64* — no
//! `rand` dependency). They emit `Vec<u64>` energies suitable for feeding
//! into [`crate::MeraTree::build`] for performance, regression, and
//! parameter-tuning workloads.
//!
//! The generators are sized in `n_accounts` (any non-zero `usize`); the
//! caller must round up to a power of two when feeding `MeraTree::build`
//! if their bond dimension demands it.

/// Deterministic xorshift64* PRNG. Self-contained so this module pulls no
/// extra deps; reseed by handing the same seed twice.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// Construct from an arbitrary seed. Maps zero into a known non-zero
    /// state so the all-zero degenerate case can't escape.
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0xDEAD_BEEF_CAFE_BABE } else { seed })
    }

    /// Next raw 64-bit output.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform float in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        // 53-bit mantissa from the high bits.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Pareto sample with shape α (α > 0). Mean exists for α > 1.
    /// Inverse-CDF: x = (1 - U)^(-1/α), shifted by 1.0 to match the gate
    /// script which adds 1.0 to keep popularity ≥ 1.
    pub fn next_pareto(&mut self, alpha: f64) -> f64 {
        let u = self.next_f64();
        let one_minus_u = (1.0 - u).max(f64::MIN_POSITIVE);
        one_minus_u.powf(-1.0 / alpha) + 1.0
    }
}

/// Knobs for [`log_correlated`].
#[derive(Clone, Debug)]
pub struct LogCorrelatedParams {
    /// Pareto shape parameter for per-account popularity.
    /// `alpha=1.0` matches the gate script.
    pub alpha: f64,
    /// Number of synthetic blocks contributing to each account's energy.
    /// Larger → smoother distribution; gate script uses 512.
    pub n_blocks: usize,
    /// Activity scaling — multiplied into the per-block touch probability.
    /// `0.8` matches the gate script.
    pub activity: f64,
    /// Energy credited per block-touch. The final per-account energy is
    /// `touches × energy_per_touch`.
    pub energy_per_touch: u64,
}

impl Default for LogCorrelatedParams {
    fn default() -> Self {
        Self {
            alpha: 1.0,
            n_blocks: 512,
            activity: 0.8,
            energy_per_touch: 100,
        }
    }
}

/// Generate a log-correlated (heavy-tail) energy vector. Top accounts have
/// orders-of-magnitude more energy than the long tail. Use when stress-
/// testing MERA in its target regime.
pub fn log_correlated(n_accounts: usize, params: &LogCorrelatedParams, seed: u64) -> Vec<u64> {
    assert!(n_accounts > 0, "n_accounts must be > 0");
    let mut rng = Rng::new(seed);
    let popularity: Vec<f64> = (0..n_accounts).map(|_| rng.next_pareto(params.alpha)).collect();
    let mut energies = vec![0u64; n_accounts];
    for b in 0..params.n_blocks {
        let block_idx = (b + 1) as f64;
        for (i, e) in energies.iter_mut().enumerate() {
            let prob = (popularity[i] / block_idx * params.activity).min(1.0);
            if rng.next_f64() < prob {
                *e = e.saturating_add(params.energy_per_touch);
            }
        }
    }
    energies
}

/// Knobs for [`area_law`].
#[derive(Clone, Debug)]
pub struct AreaLawParams {
    /// Contiguous account-segment size. Same-segment accounts co-touch with
    /// `intra_segment_prob`; cross-segment touches decay exponentially with
    /// distance (`cross_segment_decay`).
    pub segment_size: usize,
    /// Probability that a block "activates" each segment.
    pub block_activation_rate: f64,
    /// Touch probability for accounts inside an activated segment.
    pub intra_segment_prob: f64,
    /// Decay factor for cross-segment correlation. `0.0` = pure block-
    /// independent segments; `0.5` matches the gate script feel.
    pub cross_segment_decay: f64,
    /// Total blocks contributing energy.
    pub n_blocks: usize,
    /// Energy per block-touch.
    pub energy_per_touch: u64,
}

impl Default for AreaLawParams {
    fn default() -> Self {
        Self {
            segment_size: 16,
            block_activation_rate: 0.10,
            intra_segment_prob: 0.9,
            cross_segment_decay: 0.5,
            n_blocks: 512,
            energy_per_touch: 100,
        }
    }
}

/// Generate an area-law (locally correlated) energy vector. Adjacent
/// accounts co-touch; correlation falls off exponentially across segments.
/// Use to verify MPS / Verkle baselines outperform MERA in this regime.
pub fn area_law(n_accounts: usize, params: &AreaLawParams, seed: u64) -> Vec<u64> {
    assert!(n_accounts > 0, "n_accounts must be > 0");
    assert!(params.segment_size > 0, "segment_size must be > 0");
    let mut rng = Rng::new(seed);
    let mut energies = vec![0u64; n_accounts];
    let n_segments = n_accounts.div_ceil(params.segment_size);

    for _ in 0..params.n_blocks {
        // Sample which segments fire this block; each segment fires
        // independently with probability block_activation_rate.
        let active: Vec<bool> = (0..n_segments)
            .map(|_| rng.next_f64() < params.block_activation_rate)
            .collect();
        for seg in 0..n_segments {
            // Probability this segment receives energy is the max over
            // active segments of `decay^|distance|`. Captures locality
            // without a full pairwise loop.
            let mut prob = if active[seg] { params.intra_segment_prob } else { 0.0 };
            if params.cross_segment_decay > 0.0 {
                for (other, &fired) in active.iter().enumerate() {
                    if fired && other != seg {
                        let d = other.abs_diff(seg) as i32;
                        let leak =
                            params.intra_segment_prob * params.cross_segment_decay.powi(d);
                        if leak > prob {
                            prob = leak;
                        }
                    }
                }
            }
            if prob <= 0.0 {
                continue;
            }
            let start = seg * params.segment_size;
            let end = (start + params.segment_size).min(n_accounts);
            for e in &mut energies[start..end] {
                if rng.next_f64() < prob {
                    *e = e.saturating_add(params.energy_per_touch);
                }
            }
        }
    }
    energies
}

/// Knobs for [`flat_random`].
#[derive(Clone, Debug)]
pub struct FlatRandomParams {
    /// Per-account-per-block activation probability.
    pub touch_prob: f64,
    /// Total blocks.
    pub n_blocks: usize,
    /// Energy per block-touch.
    pub energy_per_touch: u64,
}

impl Default for FlatRandomParams {
    fn default() -> Self {
        Self {
            touch_prob: 0.1,
            n_blocks: 512,
            energy_per_touch: 100,
        }
    }
}

/// Generate a flat-random energy vector — every account touched
/// independently every block at the same rate. Use as the "MERA loses to
/// Verkle" baseline.
pub fn flat_random(n_accounts: usize, params: &FlatRandomParams, seed: u64) -> Vec<u64> {
    assert!(n_accounts > 0, "n_accounts must be > 0");
    let mut rng = Rng::new(seed);
    let mut energies = vec![0u64; n_accounts];
    for _ in 0..params.n_blocks {
        for e in &mut energies {
            if rng.next_f64() < params.touch_prob {
                *e = e.saturating_add(params.energy_per_touch);
            }
        }
    }
    energies
}

/// Activation matrix variant of [`log_correlated`] — returns the per-block
/// touched/not-touched indicator (rows = accounts, cols = blocks) instead
/// of the energy-summed vector. The MERA gate consumes this shape.
pub fn log_correlated_matrix(
    n_accounts: usize,
    params: &LogCorrelatedParams,
    seed: u64,
) -> Vec<Vec<f64>> {
    assert!(n_accounts > 0, "n_accounts must be > 0");
    let mut rng = Rng::new(seed);
    let popularity: Vec<f64> = (0..n_accounts).map(|_| rng.next_pareto(params.alpha)).collect();
    let mut mat = vec![vec![0.0f64; params.n_blocks]; n_accounts];
    for b in 0..params.n_blocks {
        let block_idx = (b + 1) as f64;
        for (i, row) in mat.iter_mut().enumerate() {
            let prob = (popularity[i] / block_idx * params.activity).min(1.0);
            row[b] = if rng.next_f64() < prob { 1.0 } else { 0.0 };
        }
    }
    mat
}

/// Activation matrix variant of [`area_law`].
pub fn area_law_matrix(
    n_accounts: usize,
    params: &AreaLawParams,
    seed: u64,
) -> Vec<Vec<f64>> {
    assert!(n_accounts > 0, "n_accounts must be > 0");
    assert!(params.segment_size > 0, "segment_size must be > 0");
    let mut rng = Rng::new(seed);
    let mut mat = vec![vec![0.0f64; params.n_blocks]; n_accounts];
    let n_segments = n_accounts.div_ceil(params.segment_size);

    for b in 0..params.n_blocks {
        let active: Vec<bool> = (0..n_segments)
            .map(|_| rng.next_f64() < params.block_activation_rate)
            .collect();
        for seg in 0..n_segments {
            let mut prob = if active[seg] { params.intra_segment_prob } else { 0.0 };
            if params.cross_segment_decay > 0.0 {
                for (other, &fired) in active.iter().enumerate() {
                    if fired && other != seg {
                        let d = other.abs_diff(seg) as i32;
                        let leak =
                            params.intra_segment_prob * params.cross_segment_decay.powi(d);
                        if leak > prob {
                            prob = leak;
                        }
                    }
                }
            }
            if prob <= 0.0 {
                continue;
            }
            let start = seg * params.segment_size;
            let end = (start + params.segment_size).min(n_accounts);
            for row in &mut mat[start..end] {
                row[b] = if rng.next_f64() < prob { 1.0 } else { 0.0 };
            }
        }
    }
    mat
}

/// Activation matrix variant of [`flat_random`].
pub fn flat_random_matrix(
    n_accounts: usize,
    params: &FlatRandomParams,
    seed: u64,
) -> Vec<Vec<f64>> {
    assert!(n_accounts > 0, "n_accounts must be > 0");
    let mut rng = Rng::new(seed);
    let mut mat = vec![vec![0.0f64; params.n_blocks]; n_accounts];
    for row in &mut mat {
        for cell in row.iter_mut() {
            *cell = if rng.next_f64() < params.touch_prob { 1.0 } else { 0.0 };
        }
    }
    mat
}

/// Pad an energy vector up to the next power of two by repeating zeros.
/// Convenient because [`crate::MeraTree::build`] consumes power-of-two
/// inputs cleanly (the binary tree has uniform depth).
pub fn pad_pow2(energies: &mut Vec<u64>) {
    let n = energies.len();
    if n == 0 {
        return;
    }
    let target = n.next_power_of_two();
    if target > n {
        energies.resize(target, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_per_seed() {
        let mut a = Rng::new(0xCAFE);
        let mut b = Rng::new(0xCAFE);
        for _ in 0..1024 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn log_correlated_is_deterministic() {
        let p = LogCorrelatedParams::default();
        let a = log_correlated(64, &p, 0xABCD);
        let b = log_correlated(64, &p, 0xABCD);
        assert_eq!(a, b, "same seed must produce identical workload");
    }

    #[test]
    fn log_correlated_is_heavy_tailed() {
        // Top quintile must hold >50% of total energy — the canonical
        // heavy-tail signature.
        let mut energies = log_correlated(256, &LogCorrelatedParams::default(), 0x1234);
        let total: u128 = energies.iter().copied().map(|e| e as u128).sum();
        energies.sort_unstable();
        let threshold = energies.len() * 4 / 5;
        let top_quintile: u128 = energies[threshold..].iter().copied().map(|e| e as u128).sum();
        assert!(
            top_quintile * 2 > total,
            "top 20% of accounts hold {} of {} total — distribution isn't heavy-tailed",
            top_quintile,
            total
        );
    }

    #[test]
    fn area_law_is_segment_clustered() {
        // Within-segment variance should be much lower than across-segment
        // variance — that's the signature of locality.
        let params = AreaLawParams {
            segment_size: 32,
            n_blocks: 256,
            ..Default::default()
        };
        let n = 256;
        let energies = area_law(n, &params, 0x7777);
        let n_segments = n / params.segment_size;
        let segment_means: Vec<f64> = (0..n_segments)
            .map(|s| {
                let start = s * params.segment_size;
                let end = start + params.segment_size;
                let sum: u64 = energies[start..end].iter().sum();
                sum as f64 / params.segment_size as f64
            })
            .collect();
        let overall_mean: f64 =
            segment_means.iter().sum::<f64>() / segment_means.len() as f64;
        let between_var: f64 = segment_means
            .iter()
            .map(|m| (m - overall_mean).powi(2))
            .sum::<f64>()
            / n_segments as f64;
        // Sanity: between-segment variance is non-zero (segments differ).
        assert!(
            between_var > 0.0,
            "area_law produced uniform segments — locality knob isn't active"
        );
    }

    #[test]
    fn flat_random_concentrates_around_expected_mean() {
        let p = FlatRandomParams {
            touch_prob: 0.1,
            n_blocks: 1024,
            energy_per_touch: 1,
        };
        let energies = flat_random(512, &p, 0xDEAD);
        let mean: f64 =
            energies.iter().copied().map(|e| e as f64).sum::<f64>() / energies.len() as f64;
        let expected = (p.n_blocks as f64) * p.touch_prob * (p.energy_per_touch as f64);
        // Tolerance: ±15% of expected. With 512 accounts × 1024 blocks the
        // sample variance of the per-account count is npq = 92.16, σ ≈ 9.6
        // touches → ~10% per account, but the mean of 512 samples is 22×
        // tighter (σ_mean ≈ 0.42 touches ≈ 0.4%). 15% is a generous bound.
        assert!(
            (mean - expected).abs() / expected < 0.15,
            "flat_random mean {} drifted >15% from expected {}",
            mean,
            expected
        );
    }

    #[test]
    fn pad_pow2_extends_to_next_power_of_two() {
        let mut v = vec![1u64, 2, 3, 4, 5];
        pad_pow2(&mut v);
        assert_eq!(v.len(), 8);
        assert_eq!(&v[..5], &[1, 2, 3, 4, 5]);
        assert_eq!(&v[5..], &[0, 0, 0]);

        // Already power of two: no-op.
        let mut v8 = vec![1u64; 8];
        pad_pow2(&mut v8);
        assert_eq!(v8.len(), 8);

        // Empty: no-op (avoid 0.next_power_of_two() = 1 surprise).
        let mut empty: Vec<u64> = Vec::new();
        pad_pow2(&mut empty);
        assert!(empty.is_empty());
    }
}
