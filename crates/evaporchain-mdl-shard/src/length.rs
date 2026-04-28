//! Description-length functions.
//!
//! Both reported in *bits* under integer approximations.
//!
//! ## `description_length(Π)`
//!
//! `n × log_2(k)` bits where `k` = `shard_count(Π)`. Each of the `n`
//! items needs `⌈log_2 k⌉` bits to name its shard.
//!
//! ## `data_length(Π, items)`
//!
//! Per-shard cost: for each shard `S_i`, `|S_i| × log_2(unique_values_in_S_i)`
//! bits. This rewards partitions that group items with similar
//! "values" (substrate uses `u64` items as a stand-in for any item
//! type that exposes a discrete identity).
//!
//! Together: `mdl_score = description_length + data_length`. The MDL
//! optimum minimises this sum.

use crate::partition::Partition;

/// `description_length(Π)` = `n × ⌈log_2(shard_count)⌉` bits.
/// Single-shard partitions take 0 bits to describe (every item
/// trivially in shard 0).
pub fn description_length(partition: &Partition) -> u64 {
    let k = partition.shard_count();
    if k <= 1 {
        return 0;
    }
    let bits_per_assignment = bit_length(k as u64);
    (partition.n() as u64).saturating_mul(bits_per_assignment)
}

/// `data_length(Π, items)` = `Σ_i |S_i| × ⌈log_2(uniques_in_S_i)⌉`.
/// Per-item bit cost decreases when the items in a shard share more
/// of their value range — this is the "regularity exploitation"
/// MDL penalises ignorance of.
pub fn data_length(partition: &Partition, items: &[u64]) -> u64 {
    let shards = partition.shards();
    let mut total: u64 = 0;
    for (_id, indices) in shards {
        let mut values: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
        for &i in &indices {
            if let Some(&v) = items.get(i) {
                values.insert(v);
            }
        }
        let uniques = values.len() as u64;
        if uniques <= 1 {
            // Zero-information shard: all items identical (or the shard is empty/one item).
            continue;
        }
        let per_item_bits = bit_length(uniques);
        total = total.saturating_add((indices.len() as u64).saturating_mul(per_item_bits));
    }
    total
}

/// MDL total = description + data lengths.
pub fn mdl_score(partition: &Partition, items: &[u64]) -> u64 {
    description_length(partition).saturating_add(data_length(partition, items))
}

fn bit_length(n: u64) -> u64 {
    if n <= 1 {
        0
    } else {
        64 - (n - 1).leading_zeros() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_length_single_shard_zero() {
        let p = Partition::new(vec![0; 8]).unwrap();
        assert_eq!(description_length(&p), 0);
    }

    #[test]
    fn description_length_two_shards() {
        // 4 items, 2 shards → 4 × ⌈log_2 2⌉ = 4 bits.
        let p = Partition::new(vec![0, 1, 0, 1]).unwrap();
        assert_eq!(description_length(&p), 4);
    }

    #[test]
    fn data_length_uniform_shard_zero_bits() {
        // All items identical inside the single shard.
        let p = Partition::new(vec![0, 0, 0, 0]).unwrap();
        let items = vec![7u64; 4];
        assert_eq!(data_length(&p, &items), 0);
    }

    #[test]
    fn data_length_diverse_shard_costs_bits() {
        let p = Partition::new(vec![0, 0, 0, 0]).unwrap();
        let items = vec![1, 2, 3, 4]; // 4 uniques → ⌈log_2 4⌉ = 2 bits per item
        assert_eq!(data_length(&p, &items), 4 * 2);
    }

    #[test]
    fn mdl_prefers_grouping_similar_items() {
        // Items: [1, 1, 2, 2].
        // Single shard: desc=0, data=4×log_2(2)=4. Total=4.
        // Two-shard split into {1,1}{2,2}: desc=4×1=4, data=0 (each shard uniform). Total=4.
        // Two-shard split into {1,2}{1,2}: desc=4, data=2×log_2(2) + 2×log_2(2) = 4. Total=8.
        let items = vec![1, 1, 2, 2];
        let p_one = Partition::new(vec![0, 0, 0, 0]).unwrap();
        let p_good = Partition::new(vec![0, 0, 1, 1]).unwrap();
        let p_bad = Partition::new(vec![0, 1, 0, 1]).unwrap();
        // good is at most as costly as bad.
        assert!(mdl_score(&p_good, &items) <= mdl_score(&p_bad, &items));
        // single-shard ties with good in this stylised example (both = 4).
        assert_eq!(mdl_score(&p_one, &items), 4);
        assert_eq!(mdl_score(&p_good, &items), 4);
    }
}
