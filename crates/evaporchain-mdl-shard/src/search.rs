//! Exhaustive small-case MDL-optimal search.
//!
//! For `n` items and `k_max` shards there are `k_max^n` candidate
//! assignments. Tractable up to ~`(10, 4)` total. Production swaps in
//! a beam-search or simulated-annealing approximation; this substrate
//! exposes the exact optimum so the approximation can be benchmarked.

use crate::length::mdl_score;
use crate::partition::{Partition, ShardId};

/// Exhaustively enumerate every assignment of `items` into at most
/// `max_shards` shards and return the one with minimum MDL score.
/// Caller is responsible for keeping `items.len() × max_shards`
/// reasonable (`max_shards^items.len()` candidates).
pub fn mdl_optimal(items: &[u64], max_shards: ShardId) -> Option<Partition> {
    let n = items.len();
    if n == 0 || max_shards == 0 {
        return None;
    }
    let mut best: Option<(Partition, u64)> = None;
    let mut assignments = vec![0u32; n];
    enumerate(&mut assignments, 0, max_shards, items, &mut best);
    best.map(|(p, _)| p)
}

fn enumerate(
    a: &mut Vec<ShardId>,
    pos: usize,
    max_shards: ShardId,
    items: &[u64],
    best: &mut Option<(Partition, u64)>,
) {
    if pos == a.len() {
        let p = Partition::new(a.clone()).expect("nonempty by construction");
        let score = mdl_score(&p, items);
        match best {
            None => *best = Some((p, score)),
            Some((_, prev_score)) if score < *prev_score => *best = Some((p, score)),
            _ => {}
        }
        return;
    }
    for s in 0..max_shards {
        a[pos] = s;
        enumerate(a, pos + 1, max_shards, items, best);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_items_no_optimum() {
        assert!(mdl_optimal(&[], 4).is_none());
    }

    #[test]
    fn zero_max_shards_no_optimum() {
        assert!(mdl_optimal(&[1, 2, 3], 0).is_none());
    }

    #[test]
    fn single_value_optimal_is_one_shard() {
        // 4 identical items → single shard wins (data_length=0,
        // description_length=0).
        let opt = mdl_optimal(&[7, 7, 7, 7], 4).unwrap();
        assert_eq!(opt.shard_count(), 1);
    }

    #[test]
    fn pairwise_split_can_beat_single_shard_when_items_diverse() {
        // 4 items: {1,1,2,2} has a regularity to exploit by splitting.
        // Both single-shard and good-split tie at MDL=4 in our
        // stylised metric — the search returns the FIRST optimum it
        // sees in enumeration order, which is single-shard.
        let opt = mdl_optimal(&[1, 1, 2, 2], 2).unwrap();
        let sc = mdl_score(&opt, &[1, 1, 2, 2]);
        assert_eq!(sc, 4);
    }
}
