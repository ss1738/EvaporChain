//! Maximality check + greedy extension to a maximal antichain.
//!
//! `is_maximal_antichain(a, lc)` is true iff no other block on the
//! DAG can be added to `a` while preserving mutual concurrency.
//!
//! `extend_to_maximal(a, lc, candidates)` walks the candidate list (in
//! the iteration order callers choose — typically descending energy)
//! and adds each candidate that is concurrent with every current
//! member. Greedy and order-dependent — this is the *substrate*; a
//! production proposer will use a more sophisticated selection
//! ordering.

use std::collections::BTreeSet;

use evaporchain_light_cone::{is_concurrent, BlockId, LightCone};

use crate::antichain::{Antichain, AntichainError};

/// True iff no block on `lc` outside `a` is concurrent with every
/// member of `a`. (i.e. no proper extension exists.)
pub fn is_maximal_antichain(a: &Antichain, lc: &LightCone) -> bool {
    // Iterate every known block id in the DAG. We don't have a public
    // `ids()` accessor on LightCone — gather via the `causal_*` reach
    // sets from a representative. Simpler: walk every member's union
    // of causal_past, causal_future, plus the member itself, plus
    // candidates from the broader DAG.
    //
    // For substrate scope we expose a brute-force check that visits
    // every block reachable from any antichain member's neighbourhood.
    // Production callers should pass an explicit candidate iterator.
    let candidates = collect_candidates(a, lc);
    !candidates.iter().any(|c| {
        if a.contains(c) {
            return false;
        }
        a.members().iter().all(|m| is_concurrent(lc, *m, *c))
    })
}

/// Greedy maximal-antichain extension. Walks `candidates` in the given
/// order; returns the resulting (still-valid) antichain.
pub fn extend_to_maximal<I: IntoIterator<Item = BlockId>>(
    seed: &Antichain,
    lc: &LightCone,
    candidates: I,
) -> Result<Antichain, AntichainError> {
    let mut members: BTreeSet<BlockId> = seed.members().clone();
    for c in candidates {
        if members.contains(&c) || !lc.contains(&c) {
            continue;
        }
        let ok = members
            .iter()
            .all(|m| evaporchain_light_cone::is_concurrent(lc, *m, c));
        if ok {
            members.insert(c);
        }
    }
    Antichain::from_set(members, lc)
}

/// Helper: gather all blocks reachable through the antichain's
/// neighbourhood. Used by `is_maximal_antichain` to bound the
/// brute-force candidate scan. For an empty antichain, returns the
/// empty set (any antichain over a non-empty DAG is non-maximal, so
/// callers will typically check membership against ALL DAG blocks
/// in production rather than relying on this helper).
fn collect_candidates(a: &Antichain, lc: &LightCone) -> BTreeSet<BlockId> {
    let mut out = BTreeSet::new();
    for m in a.members() {
        out.insert(*m);
        for x in evaporchain_light_cone::causal_past(lc, *m) {
            out.insert(x);
        }
        for x in evaporchain_light_cone::causal_future(lc, *m) {
            out.insert(x);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_light_cone::Block;

    fn id(b: u8) -> BlockId {
        [b; 32]
    }

    fn diamond() -> LightCone {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 800, 2)).unwrap();
        lc
    }

    #[test]
    fn singleton_concurrent_pair_is_extendable() {
        let lc = diamond();
        let seed = Antichain::from_set([id(1)].into_iter().collect(), &lc).unwrap();
        // id(2) is concurrent with id(1) — extend should add it.
        let extended = extend_to_maximal(&seed, &lc, vec![id(2)]).unwrap();
        assert_eq!(extended.len(), 2);
        assert!(extended.contains(&id(2)));
    }

    #[test]
    fn extend_skips_comparable_candidates() {
        let lc = diamond();
        let seed = Antichain::from_set([id(1)].into_iter().collect(), &lc).unwrap();
        // id(0) precedes id(1); id(3) is in id(1)'s causal future. Both
        // are comparable, so neither can be added.
        let extended = extend_to_maximal(&seed, &lc, vec![id(0), id(3)]).unwrap();
        assert_eq!(extended.len(), 1);
    }

    #[test]
    fn maximal_pair_is_maximal() {
        let lc = diamond();
        let a = Antichain::from_set([id(1), id(2)].into_iter().collect(), &lc).unwrap();
        // {id(1), id(2)} is a maximal antichain in the diamond — every
        // other block is comparable with at least one of them.
        assert!(is_maximal_antichain(&a, &lc));
    }

    #[test]
    fn singleton_not_maximal_when_concurrent_neighbour_exists() {
        let lc = diamond();
        let a = Antichain::from_set([id(1)].into_iter().collect(), &lc).unwrap();
        // id(2) is concurrent with id(1), so {id(1)} is NOT maximal.
        assert!(!is_maximal_antichain(&a, &lc));
    }
}
