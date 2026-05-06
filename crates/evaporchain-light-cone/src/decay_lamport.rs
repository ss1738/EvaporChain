//! Decay-Lamport time over the Light-Cone DAG.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 3:
//!
//! > **Decay-Lamport Time** — Clock ticks by energy spent, not wall
//! > clock. Decentralised; no NTP, no PoH leader.
//!
//! This module wires the standalone
//! [`evaporchain_decay_lamport::LamportClock`] math into the DAG.
//! It computes, for any block in the DAG, that block's logical
//! Decay-Lamport clock — derived from the merge of all parent clocks
//! plus a tick by the block's own energy expenditure.
//!
//! The integration was deferred per the comment at
//! `evaporchain-light-cone::block::Block.observed_epoch`
//! ("separate crate, weeks 20-24"). Until this module shipped,
//! `Block.observed_epoch` was a raw `u64` with no logical-time
//! semantics. With this module, every DAG block gets a derivable
//! `LamportClock` keyed by its causal past — a pure function of the
//! DAG topology + per-block energies + the chain-set tick quantum.
//!
//! ## Recurrence
//!
//! For a block `b` with parents `p_1..p_k` in the DAG, the
//! Decay-Lamport clock at `b` is:
//!
//! 1. Start with `merged = LamportClock::new(quantum)` (tick 0).
//! 2. For each parent `p_i` (in sorted order — validator-determinism),
//!    `merged = merged.merge(clock_at(p_i))`. Lamport's merge takes
//!    the max tick.
//! 3. Tick by `b.energy`: `clock_at(b) = merged.tick(b.energy)`.
//!
//! Genesis (`b.parents.is_empty()`): step 1 only, then step 3.
//!
//! ## Validator-determinism
//!
//! All iteration is over sorted parent slices and `BTreeMap` keys; no
//! `HashMap` order leaks. Two validators with identical DAG state +
//! identical `tick_quantum` produce identical clocks for every block.
//! Tested by [`tests::clocks_converge_across_validators`] (proptest
//! 64 random DAGs).
//!
//! ## Cost
//!
//! `block_lamport_clock` is `O(n)` worst-case (it walks the block's
//! causal past via memoized DFS). `all_block_clocks` is `O(n)` total
//! via topological sweep. Caching is per-call — the substrate doesn't
//! pin a clock map in `LightCone` itself, so the storage cost is paid
//! by the caller only when needed.

use std::collections::BTreeMap;

use evaporchain_decay_lamport::{LamportClock, TickError};
use evaporchain_types::Energy;

use crate::block::BlockId;
use crate::dag::LightCone;

/// Errors returned by clock derivation.
#[derive(Debug, PartialEq, Eq)]
pub enum ClockDerivationError {
    /// Requested block isn't in the DAG.
    BlockNotFound(BlockId),
    /// `tick_quantum` is 0; LamportClock can't advance.
    ZeroQuantum,
}

impl From<TickError> for ClockDerivationError {
    fn from(e: TickError) -> Self {
        match e {
            TickError::ZeroQuantum => ClockDerivationError::ZeroQuantum,
        }
    }
}

/// Compute the Decay-Lamport clock at `block_id`, given the chain's
/// `tick_quantum`. Returns `Err(BlockNotFound)` if the block isn't in
/// the DAG, or `Err(ZeroQuantum)` if `tick_quantum == 0`.
///
/// The clock is the merge of every parent clock (via the recurrence
/// in the module docs) ticked by `block.energy`. Pure function of
/// `(LightCone, block_id, tick_quantum)`.
pub fn block_lamport_clock(
    lc: &LightCone,
    block_id: BlockId,
    tick_quantum: Energy,
) -> Result<LamportClock, ClockDerivationError> {
    if tick_quantum == 0 {
        return Err(ClockDerivationError::ZeroQuantum);
    }
    if lc.get(&block_id).is_none() {
        return Err(ClockDerivationError::BlockNotFound(block_id));
    }

    // Iterative topological derivation via post-order DFS with a
    // memo cache. Recursion would blow the stack on long chains.
    let mut cache: BTreeMap<BlockId, LamportClock> = BTreeMap::new();
    let mut stack: Vec<(BlockId, bool)> = vec![(block_id, false)];

    while let Some((id, expanded)) = stack.pop() {
        if cache.contains_key(&id) {
            continue;
        }
        let block = lc.get(&id).ok_or(ClockDerivationError::BlockNotFound(id))?;

        if !expanded {
            // First visit: schedule self-after-children and push
            // children. Children = parents in the *causal past*
            // direction; we want to compute parents first.
            stack.push((id, true));
            // BTreeSet-sorted parent iteration would be ideal, but
            // Block.parents is a Vec (proposer-emitted antichain
            // order). Sort here for deterministic walk.
            let mut sorted_parents = block.parents.clone();
            sorted_parents.sort();
            for parent in sorted_parents {
                if !cache.contains_key(&parent) {
                    stack.push((parent, false));
                }
            }
            continue;
        }

        // Second visit: all parents are now in cache. Build
        // merged clock + tick.
        let mut merged = LamportClock::new(tick_quantum);
        // Sorted iteration over parents for validator-determinism.
        let mut sorted_parents = block.parents.clone();
        sorted_parents.sort();
        for parent in &sorted_parents {
            let parent_clock = cache
                .get(parent)
                .copied()
                .ok_or(ClockDerivationError::BlockNotFound(*parent))?;
            merged = merged.merge(parent_clock);
        }
        // Restore tick_quantum if merge picked up a parent's
        // (it's a self.tick_quantum-keep operation today, but
        // be explicit).
        merged.tick_quantum = tick_quantum;
        let clock = merged.tick(block.energy)?;
        cache.insert(id, clock);
    }

    cache
        .get(&block_id)
        .copied()
        .ok_or(ClockDerivationError::BlockNotFound(block_id))
}

/// Compute Decay-Lamport clocks for every block in the DAG. Returns
/// a `BTreeMap<BlockId, LamportClock>` (validator-deterministic
/// iteration order). `O(n)` total via topological sweep with a memo
/// cache — much cheaper than calling `block_lamport_clock` per block,
/// which would be `O(n * causal_past_size)`.
pub fn all_block_clocks(
    lc: &LightCone,
    tick_quantum: Energy,
) -> Result<BTreeMap<BlockId, LamportClock>, ClockDerivationError> {
    if tick_quantum == 0 {
        return Err(ClockDerivationError::ZeroQuantum);
    }

    let mut cache: BTreeMap<BlockId, LamportClock> = BTreeMap::new();
    // Drive each block via the same iterative DFS — the cache makes
    // this O(n) total because parents already in cache short-circuit.
    let block_ids: Vec<BlockId> = lc.ids().collect();
    for id in block_ids {
        // Short-circuit if we already cached this id (and all of
        // its causal past) via an earlier sweep.
        if cache.contains_key(&id) {
            continue;
        }
        let mut stack: Vec<(BlockId, bool)> = vec![(id, false)];
        while let Some((cur, expanded)) = stack.pop() {
            if cache.contains_key(&cur) {
                continue;
            }
            let block = lc.get(&cur).ok_or(ClockDerivationError::BlockNotFound(cur))?;
            if !expanded {
                stack.push((cur, true));
                let mut sorted_parents = block.parents.clone();
                sorted_parents.sort();
                for parent in sorted_parents {
                    if !cache.contains_key(&parent) {
                        stack.push((parent, false));
                    }
                }
                continue;
            }
            let mut merged = LamportClock::new(tick_quantum);
            let mut sorted_parents = block.parents.clone();
            sorted_parents.sort();
            for parent in &sorted_parents {
                let parent_clock = cache
                    .get(parent)
                    .copied()
                    .ok_or(ClockDerivationError::BlockNotFound(*parent))?;
                merged = merged.merge(parent_clock);
            }
            merged.tick_quantum = tick_quantum;
            cache.insert(cur, merged.tick(block.energy)?);
        }
    }
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;

    fn id(b: u8) -> BlockId {
        [b; 32]
    }

    #[test]
    fn genesis_clock_is_one_tick_when_energy_equals_quantum() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 100, 0)).unwrap();
        let clock = block_lamport_clock(&lc, id(0), 100).unwrap();
        assert_eq!(clock.current_tick, 1);
        assert_eq!(clock.accumulated_energy, 0);
    }

    #[test]
    fn genesis_clock_under_quantum_no_advance() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 50, 0)).unwrap();
        let clock = block_lamport_clock(&lc, id(0), 100).unwrap();
        assert_eq!(clock.current_tick, 0);
        assert_eq!(clock.accumulated_energy, 50);
    }

    #[test]
    fn linear_chain_accumulates_ticks() {
        // Genesis → b1 → b2 → b3, each with energy=100, quantum=100.
        // Tick advances by 1 per block: 1, 2, 3, 4.
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 100, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 100, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(1)], 100, 2)).unwrap();
        lc.insert(Block::new(id(3), vec![id(2)], 100, 3)).unwrap();

        for (i, block_id) in [id(0), id(1), id(2), id(3)].iter().enumerate() {
            let clock = block_lamport_clock(&lc, *block_id, 100).unwrap();
            assert_eq!(
                clock.current_tick,
                (i + 1) as u64,
                "block {} tick mismatch",
                i
            );
        }
    }

    #[test]
    fn diamond_merge_takes_max_parent_tick() {
        // Genesis (energy 100, tick→1)
        //   ├─ A (energy 100, parent_tick=1, ticked once → 2)
        //   ├─ B (energy 300, parent_tick=1, ticked thrice → 4)
        //   └─ M (parents=[A,B], energy 100,
        //        merge takes max(2, 4) = 4, ticked once → 5)
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 100, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 100, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 300, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 100, 2)).unwrap();

        assert_eq!(block_lamport_clock(&lc, id(0), 100).unwrap().current_tick, 1);
        assert_eq!(block_lamport_clock(&lc, id(1), 100).unwrap().current_tick, 2);
        assert_eq!(block_lamport_clock(&lc, id(2), 100).unwrap().current_tick, 4);
        assert_eq!(block_lamport_clock(&lc, id(3), 100).unwrap().current_tick, 5);
    }

    #[test]
    fn missing_block_reports_not_found() {
        let lc = LightCone::new();
        let err = block_lamport_clock(&lc, id(0), 100).unwrap_err();
        assert_eq!(err, ClockDerivationError::BlockNotFound(id(0)));
    }

    #[test]
    fn zero_quantum_reports_error() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 100, 0)).unwrap();
        let err = block_lamport_clock(&lc, id(0), 0).unwrap_err();
        assert_eq!(err, ClockDerivationError::ZeroQuantum);
    }

    #[test]
    fn all_block_clocks_matches_per_block_derivation() {
        // Build a 5-block diamond. Sweep all clocks at once and
        // assert every entry matches the per-block derivation.
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 100, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 200, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 300, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 100, 2)).unwrap();
        lc.insert(Block::new(id(4), vec![id(3)], 100, 3)).unwrap();

        let all = all_block_clocks(&lc, 100).unwrap();
        assert_eq!(all.len(), 5);
        for &bid in &[id(0), id(1), id(2), id(3), id(4)] {
            let one_off = block_lamport_clock(&lc, bid, 100).unwrap();
            assert_eq!(
                all.get(&bid).copied().unwrap(),
                one_off,
                "all_block_clocks[{:?}] disagrees with single-block derivation",
                bid
            );
        }
    }

    #[test]
    fn all_block_clocks_empty_dag_returns_empty_map() {
        let lc = LightCone::new();
        let all = all_block_clocks(&lc, 100).unwrap();
        assert!(all.is_empty());
    }

    #[test]
    fn lamport_clock_arrow_aligns_with_dag_partial_order() {
        // If a precedes b in the DAG, then a.current_tick <= b.current_tick.
        // Equality is allowed (a is a parent that contributes max but
        // b's energy was 0), but never strict ordering reversal.
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 100, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 200, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 50, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 100, 2)).unwrap();

        let all = all_block_clocks(&lc, 100).unwrap();
        // Genesis precedes every descendant.
        let g = all.get(&id(0)).unwrap().current_tick;
        for desc in [id(1), id(2), id(3)] {
            assert!(
                all.get(&desc).unwrap().current_tick >= g,
                "DAG-precedence must imply tick non-decrease"
            );
        }
        // Diamond merge tick >= max(parents).
        let p1 = all.get(&id(1)).unwrap().current_tick;
        let p2 = all.get(&id(2)).unwrap().current_tick;
        let m = all.get(&id(3)).unwrap().current_tick;
        assert!(m >= p1.max(p2), "merge tick must be >= max parent tick");
    }

    #[test]
    fn long_chain_doesnt_blow_stack() {
        // 1000-block linear chain. Recursive derivation would blow
        // a debug-build stack at this depth; iterative DFS handles
        // it.
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 100, 0)).unwrap();
        let mut last = id(0);
        for i in 1u32..1_000 {
            // Use unique ids by varying both ends of the [u8; 32].
            let mut bid = [0u8; 32];
            bid[..4].copy_from_slice(&i.to_le_bytes());
            bid[28..].copy_from_slice(&i.to_le_bytes());
            lc.insert(Block::new(bid, vec![last], 100, i as u64)).unwrap();
            last = bid;
        }
        let clock = block_lamport_clock(&lc, last, 100).unwrap();
        assert_eq!(clock.current_tick, 1000);
    }

    /// Validator-determinism property: two `LightCone` instances
    /// fed the same insertion sequence produce identical clock maps.
    /// 64 random DAG topologies × 1-12 blocks each.
    use proptest::prelude::*;

    fn arb_dag() -> impl Strategy<Value = Vec<(u8, Vec<u8>, u64, u64)>> {
        // Each entry: (id_seed, parent_id_seeds, energy, observed_epoch).
        // The first entry is genesis (parents=[]); subsequent entries
        // pick parents from earlier-seen seeds.
        proptest::collection::vec(
            (any::<u8>(), 1u64..1000, 0u64..100),
            1..=12,
        )
        .prop_map(|raws: Vec<(u8, u64, u64)>| {
            let mut seen: Vec<u8> = Vec::new();
            let mut out: Vec<(u8, Vec<u8>, u64, u64)> = Vec::new();
            for (seed, energy, epoch) in raws {
                let unique_seed = if seen.contains(&seed) {
                    seen.iter()
                        .max()
                        .copied()
                        .unwrap_or(0)
                        .wrapping_add(1)
                        .max(1)
                } else {
                    seed
                };
                if seen.contains(&unique_seed) {
                    continue; // skip duplicates after collision
                }
                let parents = if seen.is_empty() {
                    vec![]
                } else {
                    // Pick 1-2 parents from seen.
                    let n = (energy as usize % 2) + 1;
                    seen.iter().rev().take(n).copied().collect()
                };
                out.push((unique_seed, parents, energy, epoch));
                seen.push(unique_seed);
            }
            out
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn clocks_converge_across_validators(spec in arb_dag()) {
            let mut a = LightCone::new();
            let mut b = LightCone::new();
            for (seed, parents, energy, epoch) in &spec {
                let bid = id(*seed);
                let parent_ids: Vec<BlockId> = parents.iter().map(|p| id(*p)).collect();
                let _ = a.insert(Block::new(bid, parent_ids.clone(), *energy, *epoch));
                let _ = b.insert(Block::new(bid, parent_ids, *energy, *epoch));
            }
            let clocks_a = all_block_clocks(&a, 100).unwrap();
            let clocks_b = all_block_clocks(&b, 100).unwrap();
            prop_assert_eq!(clocks_a, clocks_b);
        }

        /// Doctrine §4.1 time-arrow claim, locked over random
        /// DAG topologies: if `a` precedes `b` in the DAG (i.e.
        /// `a` is in `b`'s causal past), then
        /// `clock(a).current_tick <= clock(b).current_tick`.
        ///
        /// This is the substrate-level statement of "the chain
        /// has a time arrow" — Decay-Lamport time agrees with
        /// causal precedence, regardless of which fork or
        /// merge-history `b` was born on.
        ///
        /// 64 random DAG topologies; for each, walk every
        /// (ancestor, descendant) pair via `causal_past` and
        /// assert tick non-decrease.
        #[test]
        fn dag_arrow_implies_tick_non_decrease(spec in arb_dag()) {
            use crate::dag::causal_past;

            let mut lc = LightCone::new();
            for (seed, parents, energy, epoch) in &spec {
                let bid = id(*seed);
                let parent_ids: Vec<BlockId> = parents.iter().map(|p| id(*p)).collect();
                let _ = lc.insert(Block::new(bid, parent_ids, *energy, *epoch));
            }
            let clocks = all_block_clocks(&lc, 100).unwrap();

            // For every block in the DAG, every member of its
            // causal past must have a tick <= its tick.
            let block_ids: Vec<BlockId> = lc.ids().collect();
            for descendant in &block_ids {
                let desc_tick = clocks
                    .get(descendant)
                    .map(|c| c.current_tick)
                    .unwrap_or(0);
                let past = causal_past(&lc, *descendant);
                for ancestor in past {
                    let anc_tick = clocks
                        .get(&ancestor)
                        .map(|c| c.current_tick)
                        .unwrap_or(0);
                    prop_assert!(
                        anc_tick <= desc_tick,
                        "DAG arrow violation: ancestor {:?} (tick {}) > descendant {:?} (tick {})",
                        ancestor,
                        anc_tick,
                        descendant,
                        desc_tick
                    );
                }
            }
        }
    }
}
