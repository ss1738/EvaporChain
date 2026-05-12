//! `StateSignature` — discretised account-energy histogram (the IB input X).
//!
//! A validator constructs its state signature by bucketing the current
//! account energy distribution into `N_BINS` equal-width buckets over the
//! [0, MAX_ENERGY) range.  The signature is normalised so it sums to 1_000
//! (integer parts per thousand) for easy comparison.

use serde::{Deserialize, Serialize};

/// Number of histogram bins in a StateSignature.
pub const N_BINS: usize = 16;

/// Discretised account-energy histogram.
///
/// `bins[k]` = fraction of accounts whose normalised energy falls in bucket k,
/// in parts-per-thousand (0..=1_000).  The 16 bins cover [0, MAX_ENERGY)
/// divided into equal-width intervals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSignature {
    pub bins: [u32; N_BINS],
}

impl StateSignature {
    /// Build a signature from a slice of (normalised) energy values in
    /// `[0, scale)`.  `scale` divides the range into N_BINS equal buckets.
    ///
    /// Values ≥ scale are placed in the top bin.
    pub fn from_energies(energies: &[u64], scale: u64) -> Self {
        let mut counts = [0u64; N_BINS];
        for &e in energies {
            let bin = if scale == 0 {
                0
            } else {
                ((e.min(scale - 1) as u128 * N_BINS as u128) / scale as u128) as usize
            };
            counts[bin.min(N_BINS - 1)] += 1;
        }
        let total = energies.len().max(1) as u64;
        let mut bins = [0u32; N_BINS];
        for (i, &c) in counts.iter().enumerate() {
            bins[i] = ((c * 1_000) / total) as u32;
        }
        Self { bins }
    }

    /// L1 distance between two signatures (in parts-per-thousand).
    pub fn l1_distance(&self, other: &Self) -> u32 {
        self.bins
            .iter()
            .zip(other.bins.iter())
            .map(|(&a, &b)| a.abs_diff(b))
            .sum()
    }

    /// KL divergence approximation: sum_k bins_k × log2(bins_k / other_k).
    /// Returns 0 if either distribution has a zero bin where the other doesn't,
    /// since this function is only used as a heuristic relevance measure.
    /// Result is in millibits (×1_000 to stay in integer arithmetic).
    pub fn kl_millibits(&self, other: &Self) -> u64 {
        let mut kl: u64 = 0;
        for (&p, &q) in self.bins.iter().zip(other.bins.iter()) {
            if p == 0 {
                continue;
            }
            if q == 0 {
                // Undefined — skip (treat as 0 contribution).
                continue;
            }
            // KL(p||q) ≈ p × (log2(p) − log2(q)) in millibits.
            // Use fixed-point: log2 approximated by bit_length − 1.
            let log2_p = u32::BITS - p.leading_zeros(); // floor(log2(p)) + 1
            let log2_q = u32::BITS - q.leading_zeros();
            if log2_p > log2_q {
                kl += (p as u64).saturating_mul((log2_p - log2_q) as u64 * 1_000);
            }
        }
        kl
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_distribution_fills_all_bins() {
        // 16 accounts each in a different bin.
        let energies: Vec<u64> = (0..16).map(|i| i * 100).collect();
        let sig = StateSignature::from_energies(&energies, 1600);
        // Each bin should have exactly 1/16 = 62.5 → 62 (floor after ×1000/16).
        let sum: u32 = sig.bins.iter().sum();
        // Sum should be close to 1000 (some rounding loss).
        assert!(sum <= 1_000);
        assert!(sum >= 900, "sum={sum}");
    }

    #[test]
    fn all_zero_energy_concentrates_in_bin_zero() {
        let energies = vec![0u64; 100];
        let sig = StateSignature::from_energies(&energies, 1_000_000);
        assert_eq!(sig.bins[0], 1_000);
        for &b in &sig.bins[1..] {
            assert_eq!(b, 0);
        }
    }

    #[test]
    fn l1_distance_to_self_is_zero() {
        let energies: Vec<u64> = (0..32).map(|i| i * 50_000).collect();
        let sig = StateSignature::from_energies(&energies, 1_600_000);
        assert_eq!(sig.l1_distance(&sig), 0);
    }

    #[test]
    fn l1_distance_is_symmetric() {
        let a = StateSignature::from_energies(&[0, 1, 2, 3], 4);
        let b = StateSignature::from_energies(&[3, 2, 1, 0], 4);
        assert_eq!(a.l1_distance(&b), b.l1_distance(&a));
    }

    /// T1.20 — from_energies with scale==0 routes every value to
    /// bin 0 (line 32 — defensive zero-scale fallback).
    #[test]
    fn t1_20_from_energies_zero_scale_falls_to_bin_zero() {
        let energies = vec![100, 200, 300, 400];
        let sig = StateSignature::from_energies(&energies, 0);
        // All routed to bin 0 → bin[0] = 1000 (parts-per-thousand).
        assert_eq!(sig.bins[0], 1_000);
        for &b in &sig.bins[1..] {
            assert_eq!(b, 0);
        }
    }

    /// T1.20 — kl_millibits skips bins where q==0 even when p>0
    /// (line 67). Returns kl=0 when no bin has both nonzero.
    #[test]
    fn t1_20_kl_millibits_skips_q_zero_bins() {
        // p: all weight in bin 0; q: all weight in bin 1.
        // Every bin pair has either p=0 or q=0 → kl skips all.
        let p_sig = StateSignature::from_energies(&[0u64; 10], 1_000);
        let q_sig = StateSignature::from_energies(&[500u64; 10], 1_000);
        assert_eq!(p_sig.kl_millibits(&q_sig), 0);
    }
}
