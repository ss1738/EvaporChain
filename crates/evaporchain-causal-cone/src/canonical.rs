//! Canonical hash of a causal cone.
//!
//! Sorted, domain-separated blake3 over the cone's block ids — so two
//! validators who observe the same set of ancestors (in any order)
//! produce identical hashes.

use std::collections::BTreeSet;

use evaporchain_light_cone::BlockId;

const DOMAIN_TAG: &[u8] = b"evaporchain-causal-cone";

pub fn canonical_cone_hash(cone: &BTreeSet<BlockId>) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(DOMAIN_TAG);
    h.update(&(cone.len() as u64).to_le_bytes());
    for id in cone {
        h.update(id);
    }
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> BlockId {
        [b; 32]
    }

    #[test]
    fn empty_cone_has_a_well_defined_hash() {
        let _ = canonical_cone_hash(&BTreeSet::new());
    }

    #[test]
    fn deterministic_under_same_input() {
        let cone: BTreeSet<BlockId> = [id(1), id(2), id(3)].into_iter().collect();
        assert_eq!(canonical_cone_hash(&cone), canonical_cone_hash(&cone));
    }

    #[test]
    fn order_independence_via_btreeset() {
        // BTreeSet always iterates sorted; both inputs collect to the
        // same set so produce the same hash.
        let a: BTreeSet<BlockId> = [id(1), id(2), id(3)].into_iter().collect();
        let b: BTreeSet<BlockId> = [id(3), id(2), id(1)].into_iter().collect();
        assert_eq!(canonical_cone_hash(&a), canonical_cone_hash(&b));
    }

    #[test]
    fn distinct_cones_distinct_hashes() {
        let a: BTreeSet<BlockId> = [id(1), id(2)].into_iter().collect();
        let b: BTreeSet<BlockId> = [id(1), id(3)].into_iter().collect();
        assert_ne!(canonical_cone_hash(&a), canonical_cone_hash(&b));
    }
}
