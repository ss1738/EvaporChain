//! `embd_micros` — earth-mover block-diff.

use thiserror::Error;

/// Fixed-point unit: 1.0 = 1_000_000.
pub const MICROS: u128 = 1_000_000;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BlockDiffError {
    #[error("arrival and final rank vectors have different lengths: {arrival} vs {final_}")]
    LengthMismatch { arrival: usize, final_: usize },
    #[error("rank vectors must contain all of 0..N-1 — duplicate or out-of-range rank")]
    InvalidPermutation,
    #[error("empty block")]
    EmptyBlock,
    #[error("EMBD {observed_micros} above floor {floor_micros} — MEV-suspect block")]
    AboveFloor {
        observed_micros: u64,
        floor_micros: u64,
    },
    #[error("arithmetic overflow")]
    Overflow,
}

/// EMBD between two rank permutations of the same set of
/// transactions. Both inputs are length-N vectors mapping
/// transaction index → rank in their respective ordering. Returns
/// the per-tx average rank-displacement, scaled by MICROS.
///
/// `arrival_rank[i]` = position of tx `i` in arrival order.
/// `final_rank[i]`   = position of tx `i` in the block.
pub fn embd_micros(
    arrival_rank: &[u32],
    final_rank: &[u32],
) -> Result<u64, BlockDiffError> {
    if arrival_rank.len() != final_rank.len() {
        return Err(BlockDiffError::LengthMismatch {
            arrival: arrival_rank.len(),
            final_: final_rank.len(),
        });
    }
    let n = arrival_rank.len();
    if n == 0 {
        return Err(BlockDiffError::EmptyBlock);
    }
    validate_permutation(arrival_rank)?;
    validate_permutation(final_rank)?;

    // Σ |arrival_rank[i] - final_rank[i]|, in u128.
    let mut sum_displacement: u128 = 0;
    for (a, f) in arrival_rank.iter().zip(final_rank.iter()) {
        let d = if a >= f { *a - *f } else { *f - *a } as u128;
        sum_displacement = sum_displacement
            .checked_add(d)
            .ok_or(BlockDiffError::Overflow)?;
    }

    // Average per tx: sum / n. Scale by MICROS.
    let scaled = sum_displacement
        .checked_mul(MICROS)
        .ok_or(BlockDiffError::Overflow)?;
    let avg = scaled / (n as u128);
    Ok(avg.try_into().unwrap_or(u64::MAX))
}

/// Floor gate: alarm if EMBD > floor (MEV-suspect block).
pub fn embd_floor_check(
    arrival_rank: &[u32],
    final_rank: &[u32],
    floor_micros: u64,
) -> Result<u64, BlockDiffError> {
    let observed = embd_micros(arrival_rank, final_rank)?;
    if observed > floor_micros {
        return Err(BlockDiffError::AboveFloor {
            observed_micros: observed,
            floor_micros,
        });
    }
    Ok(observed)
}

/// Maximum possible EMBD for a block of size `n`. Achieved when
/// the first tx in arrival order goes to the last position in the
/// block (and vice-versa for several txs in a sweeping reversal).
///
/// For the *fully reversed* permutation: each tx at arrival rank
/// `i` goes to final rank `N-1-i`, so displacement is
/// `|i - (N-1-i)| = |2i - (N-1)|`. Sum over i gives roughly
/// N(N-1)/2 / 2 for large N — but we want a usable cap. Below we
/// compute the exact max-displacement sum for the reversal case.
pub fn embd_max_for_block_size(n: usize) -> u64 {
    if n <= 1 {
        return 0;
    }
    let mut sum: u128 = 0;
    for i in 0..n {
        let target = (n - 1) - i;
        let d = if i >= target { i - target } else { target - i };
        sum += d as u128;
    }
    let scaled = sum.saturating_mul(MICROS) / (n as u128);
    scaled.try_into().unwrap_or(u64::MAX)
}

fn validate_permutation(v: &[u32]) -> Result<(), BlockDiffError> {
    let n = v.len();
    let mut seen = vec![false; n];
    for &r in v {
        if (r as usize) >= n || seen[r as usize] {
            return Err(BlockDiffError::InvalidPermutation);
        }
        seen[r as usize] = true;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── identity / symmetry ───────────────────────────────────────

    #[test]
    fn identity_permutation_is_zero() {
        // arrival[i] = final[i] = i — no reordering.
        let v: Vec<u32> = (0..5).collect();
        assert_eq!(embd_micros(&v, &v).unwrap(), 0);
    }

    #[test]
    fn symmetric() {
        let arrival = vec![0u32, 1, 2, 3, 4];
        let final_ = vec![2u32, 0, 3, 1, 4];
        let a = embd_micros(&arrival, &final_).unwrap();
        let b = embd_micros(&final_, &arrival).unwrap();
        assert_eq!(a, b);
    }

    // ── concrete examples ─────────────────────────────────────────

    #[test]
    fn one_swap_neighboring() {
        // Two adjacent txs swap. Each displaced by 1 rank;
        // total displacement = 2; per-tx = 2/5 = 0.4 = 400_000.
        let arrival = vec![0u32, 1, 2, 3, 4];
        let final_  = vec![1u32, 0, 2, 3, 4];
        assert_eq!(embd_micros(&arrival, &final_).unwrap(), 400_000);
    }

    #[test]
    fn full_reversal() {
        // 5 txs, fully reversed. Displacements: |0-4|, |1-3|,
        // |2-2|, |3-1|, |4-0| = 4+2+0+2+4 = 12. Per-tx = 12/5 = 2.4
        // → 2_400_000.
        let arrival = vec![0u32, 1, 2, 3, 4];
        let final_  = vec![4u32, 3, 2, 1, 0];
        assert_eq!(embd_micros(&arrival, &final_).unwrap(), 2_400_000);
    }

    #[test]
    fn frontrun_pattern() {
        // Adversary frontruns: original order [victim, A, B];
        // attacker's tx (call it I) was arrival-rank 3 (after them
        // all). Final order: [I, victim, A, B] — I jumps from
        // rank 3 to rank 0. Wait — we have 4 txs total then.
        let arrival = vec![1u32, 2, 3, 0]; // I at index 3, arrival-rank 0
        // Hmm let me re-think. Use 4 txs where one inserted late
        // jumps to the front.
        // Tx indices 0..3, where 3 is the inserted attacker.
        // Arrival order (timeline): victim(0)→A(1)→B(2)→I(3)
        //   so arrival_rank[i] = i.
        // Final order (block): I(3)→victim(0)→A(1)→B(2)
        //   so final_rank[0]=1, final_rank[1]=2, final_rank[2]=3, final_rank[3]=0.
        let arrival = vec![0u32, 1, 2, 3];
        let final_  = vec![1u32, 2, 3, 0];
        // Displacements: 1, 1, 1, 3 → sum 6 → per-tx 6/4 = 1.5 = 1_500_000.
        assert_eq!(embd_micros(&arrival, &final_).unwrap(), 1_500_000);
    }

    // ── error paths ──────────────────────────────────────────────

    #[test]
    fn length_mismatch_rejected() {
        let a = vec![0u32, 1, 2];
        let f = vec![0u32, 1];
        let err = embd_micros(&a, &f).unwrap_err();
        assert!(matches!(err, BlockDiffError::LengthMismatch { .. }));
    }

    #[test]
    fn empty_block_rejected() {
        let v: Vec<u32> = vec![];
        let err = embd_micros(&v, &v).unwrap_err();
        assert_eq!(err, BlockDiffError::EmptyBlock);
    }

    #[test]
    fn invalid_permutation_with_duplicate_rank_rejected() {
        let a = vec![0u32, 1, 2];
        let f = vec![0u32, 0, 1]; // rank 0 appears twice
        let err = embd_micros(&a, &f).unwrap_err();
        assert_eq!(err, BlockDiffError::InvalidPermutation);
    }

    #[test]
    fn invalid_permutation_with_out_of_range_rank_rejected() {
        let a = vec![0u32, 1, 2];
        let f = vec![0u32, 1, 5]; // rank 5 is out of range for n=3
        let err = embd_micros(&a, &f).unwrap_err();
        assert_eq!(err, BlockDiffError::InvalidPermutation);
    }

    // ── floor gate ────────────────────────────────────────────────

    #[test]
    fn within_floor_passes() {
        let a = vec![0u32, 1, 2, 3, 4];
        let f = vec![1u32, 0, 2, 3, 4];
        let observed = embd_floor_check(&a, &f, 500_000).unwrap();
        assert_eq!(observed, 400_000);
    }

    #[test]
    fn above_floor_alarms() {
        let a = vec![0u32, 1, 2, 3, 4];
        let f = vec![4u32, 3, 2, 1, 0];
        let err = embd_floor_check(&a, &f, 500_000).unwrap_err();
        assert!(matches!(err, BlockDiffError::AboveFloor { .. }));
    }

    // ── max for block size ────────────────────────────────────────

    #[test]
    fn max_for_size_one_is_zero() {
        assert_eq!(embd_max_for_block_size(1), 0);
    }

    #[test]
    fn max_for_size_zero_is_zero() {
        assert_eq!(embd_max_for_block_size(0), 0);
    }

    #[test]
    fn max_for_size_five_matches_full_reversal() {
        // Exactly the result of `full_reversal` test.
        assert_eq!(embd_max_for_block_size(5), 2_400_000);
    }

    #[test]
    fn embd_never_exceeds_max() {
        // Spot-check: for any permutation, EMBD ≤ embd_max.
        let a = vec![0u32, 1, 2, 3, 4, 5, 6, 7];
        let f = vec![3u32, 7, 1, 0, 5, 2, 6, 4];
        let observed = embd_micros(&a, &f).unwrap();
        let max = embd_max_for_block_size(8);
        assert!(observed <= max);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Earth-Mover Block Diff is EvaporChain's
        // structural MEV detector. A block's transaction order
        // is compared against arrival order; large displacement
        // = signal of reordering attack. Honest blocks have
        // small EMBD; sandwiched / frontrun blocks have large
        // EMBD."
        //
        // Demonstrate: an honest block (small reorder) vs a
        // sandwich-pattern block.
        let n = 8usize;
        let arrival: Vec<u32> = (0..n as u32).collect();
        // Honest: single neighbouring swap. EMBD ≈ 1/8 → ~250_000 micros.
        let honest_final = vec![1u32, 0, 2, 3, 4, 5, 6, 7];
        let honest = embd_micros(&arrival, &honest_final).unwrap();

        // Sandwich pattern: attacker insert at the front + back of
        // the victim. txs 6 and 7 are attacker; tx 0 is victim;
        // attacker reorders to [6, 0, 1, 2, 3, 4, 5, 7] - so 6 jumps
        // from 6→0, 0..5 each shift by +1, 7 stays.
        let sandwich_final = vec![1u32, 2, 3, 4, 5, 6, 0, 7];
        let sandwich = embd_micros(&arrival, &sandwich_final).unwrap();

        assert!(honest < sandwich);

        // Floor between the two: chain alarms on sandwich, not honest.
        let floor = (honest + sandwich) / 2;
        assert!(embd_floor_check(&arrival, &honest_final, floor).is_ok());
        assert!(embd_floor_check(&arrival, &sandwich_final, floor).is_err());
    }

    proptest::proptest! {
        #[test]
        fn property_identity_is_zero(n in 1usize..20usize) {
            // For any block size, the identity permutation has EMBD = 0.
            let v: Vec<u32> = (0..n as u32).collect();
            proptest::prop_assert_eq!(embd_micros(&v, &v).unwrap(), 0);
        }

        #[test]
        fn property_symmetric(n in 2usize..10usize, seed in 0u64..1000u64) {
            // For any pair of permutations, EMBD(a,f) = EMBD(f,a).
            let arrival: Vec<u32> = (0..n as u32).collect();
            // Permute deterministically using seed.
            let mut final_: Vec<u32> = arrival.clone();
            let mut s = seed;
            for i in 0..n {
                let j = (s as usize) % n;
                final_.swap(i, j);
                s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            }
            let a = embd_micros(&arrival, &final_).unwrap();
            let b = embd_micros(&final_, &arrival).unwrap();
            proptest::prop_assert_eq!(a, b);
        }

        #[test]
        fn property_bounded_by_max(n in 1usize..20usize, seed in 0u64..1000u64) {
            let arrival: Vec<u32> = (0..n as u32).collect();
            let mut final_: Vec<u32> = arrival.clone();
            let mut s = seed;
            for i in 0..n {
                let j = (s as usize) % n;
                final_.swap(i, j);
                s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            }
            let observed = embd_micros(&arrival, &final_).unwrap();
            let max = embd_max_for_block_size(n);
            proptest::prop_assert!(observed <= max);
        }
    }
}
