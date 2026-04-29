//! `EulogyTrie` — append-only memorial keyed by the original
//! account address. The chain's deliberate exception to the
//! anti-immutability rule.
//!
//! Order-independent root via `BTreeMap`-backed iteration:
//! `root = blake3("evaporchain-eulogy-trie" || count || sorted (addr ||
//! commitment) pairs)`. Two nodes that have observed the same set of
//! tombstones (in any order) compute the same root.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

use evaporchain_types::AccountAddress;

use crate::tombstone::Tombstone;

const ROOT_TAG: &[u8] = b"evaporchain-eulogy-trie";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EulogyTrie {
    /// Address → tombstone. Append-only by enforcement (see `insert`).
    entries: BTreeMap<AccountAddress, Tombstone>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EulogyError {
    #[error("address {0:?} already has a tombstone — re-evaporation forbidden")]
    AlreadyMemorialised(AccountAddress),
}

impl EulogyTrie {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a tombstone for `addr`. Rejects re-insertion: an
    /// account can only evaporate once. The chain's death is final.
    pub fn insert(&mut self, addr: AccountAddress, tombstone: Tombstone) -> Result<(), EulogyError> {
        if self.entries.contains_key(&addr) {
            return Err(EulogyError::AlreadyMemorialised(addr));
        }
        self.entries.insert(addr, tombstone);
        Ok(())
    }

    pub fn contains(&self, addr: &AccountAddress) -> bool {
        self.entries.contains_key(addr)
    }

    pub fn get(&self, addr: &AccountAddress) -> Option<&Tombstone> {
        self.entries.get(addr)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate (addr, tombstone) pairs in deterministic (sorted-addr) order.
    pub fn iter(&self) -> impl Iterator<Item = (&AccountAddress, &Tombstone)> {
        self.entries.iter()
    }

    /// Domain-separated blake3 root commitment of the entire eulogy
    /// trie. Order-independent: two tries built by inserting the same
    /// (addr, tombstone) pairs in any order produce the same root.
    pub fn root(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(ROOT_TAG);
        h.update(&(self.entries.len() as u64).to_le_bytes());
        for (addr, t) in &self.entries {
            h.update(addr);
            h.update(&t.commitment);
        }
        *h.finalize().as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cause::CauseOfDeath;
    use crate::tombstone::mint;

    fn addr(b: u8) -> AccountAddress {
        [b; 32]
    }

    fn ts(addr_byte: u8, epoch: u64) -> Tombstone {
        mint(addr(addr_byte), 0, epoch, CauseOfDeath::Evaporated)
    }

    #[test]
    fn empty_trie_root_is_canonical() {
        let t = EulogyTrie::new();
        let r = t.root();
        // Sanity: empty trie has a deterministic non-zero root
        // (it commits to "count = 0" via the domain tag).
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        // Two empty tries agree.
        let t2 = EulogyTrie::new();
        assert_eq!(r, t2.root());
    }

    #[test]
    fn insert_records_and_changes_root() {
        let mut t = EulogyTrie::new();
        let r0 = t.root();
        t.insert(addr(1), ts(1, 100)).unwrap();
        assert!(t.contains(&addr(1)));
        let r1 = t.root();
        assert_ne!(r0, r1);
    }

    #[test]
    fn re_insertion_rejected() {
        let mut t = EulogyTrie::new();
        t.insert(addr(1), ts(1, 100)).unwrap();
        let err = t.insert(addr(1), ts(1, 200)).unwrap_err();
        assert!(matches!(err, EulogyError::AlreadyMemorialised(_)));
    }

    #[test]
    fn root_is_order_independent() {
        let mut t1 = EulogyTrie::new();
        t1.insert(addr(1), ts(1, 100)).unwrap();
        t1.insert(addr(2), ts(2, 100)).unwrap();
        t1.insert(addr(3), ts(3, 100)).unwrap();

        let mut t2 = EulogyTrie::new();
        t2.insert(addr(3), ts(3, 100)).unwrap();
        t2.insert(addr(1), ts(1, 100)).unwrap();
        t2.insert(addr(2), ts(2, 100)).unwrap();

        assert_eq!(t1.root(), t2.root());
    }

    #[test]
    fn distinct_tombstone_sets_distinct_roots() {
        let mut t1 = EulogyTrie::new();
        t1.insert(addr(1), ts(1, 100)).unwrap();

        let mut t2 = EulogyTrie::new();
        t2.insert(addr(1), ts(1, 200)).unwrap(); // different epoch

        assert_ne!(t1.root(), t2.root());
    }

    #[test]
    fn get_returns_recorded_tombstone() {
        let mut t = EulogyTrie::new();
        let stone = ts(1, 100);
        t.insert(addr(1), stone).unwrap();
        assert_eq!(t.get(&addr(1)), Some(&stone));
        assert_eq!(t.get(&addr(99)), None);
    }
}
