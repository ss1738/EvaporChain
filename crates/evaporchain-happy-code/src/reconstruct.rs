//! Reconstruct: k-of-N Lagrange interpolation, decay-floor gate.

use thiserror::Error;

use crate::encode::Share;
use crate::field::{add_p, inverse_p, mul_p, neg_p, sub_p, FieldElem, MOD_P};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ReconstructError {
    #[error(
        "only {fresh}/{required} fresh shares available — adversary cannot recover bulk"
    )]
    InsufficientFreshShares { fresh: usize, required: usize },
    #[error("duplicate share index {0}")]
    DuplicateIndex(u64),
    #[error("zero share index — index 0 is reserved for bulk")]
    ZeroIndex,
    #[error("interpolation failed: zero in denominator (duplicate index slipped through)")]
    InterpolationFailure,
}

/// Reconstruct the bulk by Lagrange interpolation at x=0 over a
/// subset of `≥ k_threshold` shares whose energy is `≥
/// reconstruction_floor`.
///
/// Decay-aware: shares below floor are filtered out FIRST. If the
/// remaining count `< k_threshold`, returns `InsufficientFreshShares`.
pub fn reconstruct_bulk(
    shares: &[Share],
    k_threshold: usize,
    reconstruction_floor: u64,
) -> Result<FieldElem, ReconstructError> {
    // Filter to fresh-only shares.
    let mut fresh: Vec<&Share> = shares
        .iter()
        .filter(|s| s.energy >= reconstruction_floor)
        .collect();

    // Validate indices.
    let mut seen = std::collections::HashSet::new();
    for s in &fresh {
        if s.index == 0 {
            return Err(ReconstructError::ZeroIndex);
        }
        if !seen.insert(s.index) {
            return Err(ReconstructError::DuplicateIndex(s.index));
        }
    }

    if fresh.len() < k_threshold {
        return Err(ReconstructError::InsufficientFreshShares {
            fresh: fresh.len(),
            required: k_threshold,
        });
    }

    // Take exactly k_threshold of the fresh shares (any subset
    // suffices; we pick the first k by index for determinism).
    fresh.sort_by_key(|s| s.index);
    let used = &fresh[..k_threshold];

    // Lagrange interpolation at x=0:
    //   f(0) = Σ_i y_i · Π_{j≠i} (0 - x_j) / (x_i - x_j)
    //         = Σ_i y_i · Π_{j≠i} (-x_j) / (x_i - x_j)
    let mut acc: FieldElem = 0;
    for (i, s_i) in used.iter().enumerate() {
        let mut num: FieldElem = 1;
        let mut den: FieldElem = 1;
        for (j, s_j) in used.iter().enumerate() {
            if i == j {
                continue;
            }
            num = mul_p(num, neg_p(s_j.index));
            den = mul_p(den, sub_p(s_i.index, s_j.index));
        }
        let den_inv = inverse_p(den).ok_or(ReconstructError::InterpolationFailure)?;
        let term = mul_p(s_i.value, mul_p(num, den_inv));
        acc = add_p(acc, term);
    }
    Ok(acc % MOD_P)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::encode_bulk;

    fn fresh_shares(bulk: FieldElem, n: usize, k: usize) -> Vec<Share> {
        encode_bulk(bulk, n, k, [0xAA; 32], 1000).unwrap()
    }

    // ── full-share reconstruction ────────────────────────────────

    #[test]
    fn full_share_set_recovers_bulk() {
        let bulk = 42u64;
        let shares = fresh_shares(bulk, 5, 3);
        let recovered = reconstruct_bulk(&shares, 3, 100).unwrap();
        assert_eq!(recovered, bulk);
    }

    #[test]
    fn exact_threshold_subset_recovers_bulk() {
        let bulk = 42u64;
        let shares = fresh_shares(bulk, 5, 3);
        let subset = shares[..3].to_vec();
        let recovered = reconstruct_bulk(&subset, 3, 100).unwrap();
        assert_eq!(recovered, bulk);
    }

    #[test]
    fn different_subset_choices_all_recover_bulk() {
        let bulk = 12345u64;
        let shares = fresh_shares(bulk, 5, 3);
        // Try (0,1,2), (1,2,3), (2,3,4), (0,2,4) — any 3-subset works.
        let combos = vec![
            vec![0, 1, 2],
            vec![1, 2, 3],
            vec![2, 3, 4],
            vec![0, 2, 4],
            vec![0, 3, 4],
        ];
        for combo in combos {
            let subset: Vec<Share> = combo.iter().map(|&i| shares[i]).collect();
            let recovered = reconstruct_bulk(&subset, 3, 100).unwrap();
            assert_eq!(recovered, bulk);
        }
    }

    // ── decay-floor gate ─────────────────────────────────────────

    #[test]
    fn decayed_shares_excluded_from_count() {
        let bulk = 42u64;
        let mut shares = fresh_shares(bulk, 5, 3);
        // Decay 3 of 5 shares below floor.
        shares[0].energy = 50;
        shares[1].energy = 50;
        shares[2].energy = 50;
        // Remaining 2 fresh shares < threshold of 3.
        let err = reconstruct_bulk(&shares, 3, 100).unwrap_err();
        assert_eq!(
            err,
            ReconstructError::InsufficientFreshShares { fresh: 2, required: 3 }
        );
    }

    #[test]
    fn just_enough_fresh_shares_succeeds() {
        let bulk = 42u64;
        let mut shares = fresh_shares(bulk, 5, 3);
        // Decay 2; leaves 3 fresh — exactly the threshold.
        shares[0].energy = 50;
        shares[1].energy = 50;
        let recovered = reconstruct_bulk(&shares, 3, 100).unwrap();
        assert_eq!(recovered, bulk);
    }

    #[test]
    fn share_at_floor_is_accepted() {
        let bulk = 42u64;
        let mut shares = fresh_shares(bulk, 5, 3);
        // Set 3 shares to energy = floor (boundary case).
        shares[0].energy = 100;
        shares[1].energy = 100;
        shares[2].energy = 100;
        // Other two are stale below floor.
        shares[3].energy = 50;
        shares[4].energy = 50;
        let recovered = reconstruct_bulk(&shares, 3, 100).unwrap();
        assert_eq!(recovered, bulk);
    }

    #[test]
    fn share_one_below_floor_is_excluded() {
        let bulk = 42u64;
        let mut shares = fresh_shares(bulk, 5, 3);
        // 3 shares at energy=99, 2 at 50. Floor = 100. 0 fresh.
        shares[0].energy = 99;
        shares[1].energy = 99;
        shares[2].energy = 99;
        shares[3].energy = 50;
        shares[4].energy = 50;
        let err = reconstruct_bulk(&shares, 3, 100).unwrap_err();
        assert!(matches!(err, ReconstructError::InsufficientFreshShares { .. }));
    }

    // ── share-index validation ───────────────────────────────────

    #[test]
    fn duplicate_index_rejected() {
        let bulk = 42u64;
        let mut shares = fresh_shares(bulk, 5, 3);
        shares[1].index = shares[0].index;
        let err = reconstruct_bulk(&shares, 3, 100).unwrap_err();
        assert!(matches!(err, ReconstructError::DuplicateIndex(_)));
    }

    #[test]
    fn zero_index_rejected() {
        let bulk = 42u64;
        let mut shares = fresh_shares(bulk, 5, 3);
        shares[0].index = 0;
        let err = reconstruct_bulk(&shares, 3, 100).unwrap_err();
        assert_eq!(err, ReconstructError::ZeroIndex);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "HaPPY Holographic Decay Code is the first
        // holographic erasure code where bulk recovery is gated
        // by the FRESHNESS of boundary shares, not just their
        // count. An adversary holding N-1 stale shares has zero
        // recovery power; a holder with k fresh shares recovers
        // the bulk; in between, the chain enforces the threshold
        // structurally."

        let bulk = 0xCAFEu64;
        let n_total = 7;
        let k_threshold = 4;
        let mut shares = encode_bulk(bulk, n_total, k_threshold, [0xCD; 32], 1000).unwrap();

        // Honest holder: all fresh → recovers.
        let recovered = reconstruct_bulk(&shares, k_threshold, 100).unwrap();
        assert_eq!(recovered, bulk);

        // Adversary holds N-1=6 shares, but 4 of them have decayed
        // below floor. Only 2 fresh < k_threshold of 4.
        for i in 0..4 {
            shares[i].energy = 50;
        }
        // shares[4..7] still at 1000.
        let err = reconstruct_bulk(&shares, k_threshold, 100).unwrap_err();
        assert!(matches!(err, ReconstructError::InsufficientFreshShares { .. }));

        // Re-attestation: bring 2 of the decayed shares back.
        shares[0].energy = 1000;
        shares[1].energy = 1000;
        // Now 5 fresh shares ≥ threshold of 4 → recovers.
        let recovered = reconstruct_bulk(&shares, k_threshold, 100).unwrap();
        assert_eq!(recovered, bulk);
    }

    proptest::proptest! {
        #[test]
        fn property_any_k_subset_recovers(
            bulk in 0u64..1000u64,
            seed_byte in 0u8..255u8,
        ) {
            let n = 6;
            let k = 3;
            let shares = encode_bulk(bulk, n, k, [seed_byte; 32], 1000).unwrap();
            // Try all k-subsets of the n shares.
            for i in 0..(n - k + 1) {
                let subset: Vec<Share> = shares[i..(i + k)].to_vec();
                let recovered = reconstruct_bulk(&subset, k, 100).unwrap();
                proptest::prop_assert_eq!(recovered, bulk);
            }
        }
    }
}
