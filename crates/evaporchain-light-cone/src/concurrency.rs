//! Concurrency / partial-order relations on a `LightCone`.
//!
//! - `precedes(a, b)` — true iff `a ∈ causal_past(b)`.
//! - `comparable(a, b)` — true iff one precedes the other, OR a = b.
//! - `is_concurrent(a, b)` — true iff neither precedes the other AND
//!   they are distinct. The mempool's *antichain* operation in the
//!   sister crate `evaporchain-antichain-mempool` rests on this.

use crate::block::BlockId;
use crate::dag::{causal_past, LightCone};

/// `a` precedes `b` iff `a` is in `b`'s causal past.
pub fn precedes(lc: &LightCone, a: BlockId, b: BlockId) -> bool {
    if a == b {
        return false;
    }
    causal_past(lc, b).contains(&a)
}

/// `a` and `b` are comparable iff one precedes the other or they're
/// equal. This is reflexive/transitive on the partial order.
pub fn comparable(lc: &LightCone, a: BlockId, b: BlockId) -> bool {
    if a == b {
        return true;
    }
    precedes(lc, a, b) || precedes(lc, b, a)
}

/// `a` and `b` are concurrent iff neither precedes the other AND
/// they are distinct.
pub fn is_concurrent(lc: &LightCone, a: BlockId, b: BlockId) -> bool {
    if a == b {
        return false;
    }
    !precedes(lc, a, b) && !precedes(lc, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;

    fn id(b: u8) -> BlockId {
        [b; 32]
    }

    fn diamond() -> LightCone {
        // A → B, A → C, B → D, C → D
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
        lc.insert(Block::new(id(3), vec![id(1), id(2)], 800, 2))
            .unwrap();
        lc
    }

    #[test]
    fn precedes_examples() {
        let lc = diamond();
        assert!(precedes(&lc, id(0), id(3)));
        assert!(precedes(&lc, id(1), id(3)));
        assert!(!precedes(&lc, id(1), id(2))); // concurrent, neither precedes
        assert!(!precedes(&lc, id(0), id(0))); // not reflexive
    }

    #[test]
    fn concurrent_pair_in_diamond() {
        let lc = diamond();
        assert!(is_concurrent(&lc, id(1), id(2)));
        assert!(is_concurrent(&lc, id(2), id(1))); // symmetric
        assert!(!is_concurrent(&lc, id(0), id(1))); // ordered
        assert!(!is_concurrent(&lc, id(1), id(1))); // not reflexive
    }

    #[test]
    fn comparable_is_reflexive() {
        let lc = diamond();
        for b in [0, 1, 2, 3] {
            assert!(comparable(&lc, id(b), id(b)));
        }
    }

    #[test]
    fn comparable_via_precedes() {
        let lc = diamond();
        assert!(comparable(&lc, id(0), id(3)));
        assert!(comparable(&lc, id(3), id(0))); // either direction
        assert!(!comparable(&lc, id(1), id(2))); // concurrent
    }
}
