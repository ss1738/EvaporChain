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
/// member of `a` (i.e. no proper extension exists).
///
/// Brute-force candidate sweep over `LightCone::ids()` — O(|DAG| × |a|)
/// in the number of `is_concurrent` checks. Acceptable at substrate
/// scope; a production proposer will keep its own candidate index.
pub fn is_maximal_antichain(a: &Antichain, lc: &LightCone) -> bool {
    !lc.ids().any(|c| {
        if a.contains(&c) {
            return false;
        }
        a.members().iter().all(|m| is_concurrent(lc, *m, c))
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
