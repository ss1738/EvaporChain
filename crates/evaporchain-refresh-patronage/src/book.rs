use evaporchain_energy_kernel::refresh_pool::NamespaceId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::covenant::PatronageCovenant;

/// Registry of active Patronage Covenants, indexed by object_id.
///
/// `patronage_ns` is the global namespace credit into which all donated surplus
/// accrues. The chain governance sets this once at genesis; it never changes
/// within a covenant's lifetime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatronageBook {
    covenants: BTreeMap<Vec<u8>, PatronageCovenant>,
    pub patronage_ns: NamespaceId,
}

impl PatronageBook {
    pub fn new(patronage_ns: NamespaceId) -> Self {
        Self {
            covenants: BTreeMap::new(),
            patronage_ns,
        }
    }

    pub fn insert(&mut self, cv: PatronageCovenant) {
        self.covenants.insert(cv.object_id.clone(), cv);
    }

    pub fn get(&self, object_id: &[u8]) -> Option<&PatronageCovenant> {
        self.covenants.get(object_id)
    }

    pub fn get_mut(&mut self, object_id: &[u8]) -> Option<&mut PatronageCovenant> {
        self.covenants.get_mut(object_id)
    }

    pub fn remove(&mut self, object_id: &[u8]) -> Option<PatronageCovenant> {
        self.covenants.remove(object_id)
    }

    pub fn len(&self) -> usize {
        self.covenants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.covenants.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &PatronageCovenant> {
        self.covenants.values()
    }

    /// Expire all covenants whose `expires_epoch <= epoch`.
    /// Returns the expired covenants (patronage_score retained) without
    /// refunding pre_funded — callers should ensure `honour` was called for
    /// all epochs up to `expires_epoch - 1` before calling this.
    pub fn expire_all(&mut self, epoch: u64) -> Vec<PatronageCovenant> {
        let expired_keys: Vec<_> = self
            .covenants
            .iter()
            .filter(|(_, cv)| cv.expires_epoch <= epoch)
            .map(|(k, _)| k.clone())
            .collect();
        expired_keys
            .into_iter()
            .filter_map(|k| self.covenants.remove(&k))
            .collect()
    }

    /// Total pre_funded energy held across all active covenants.
    pub fn total_pre_funded(&self) -> u64 {
        self.covenants
            .values()
            .map(|cv| cv.pre_funded)
            .fold(0u64, |a, b| a.saturating_add(b))
    }

    /// Total patronage_score across all currently-active covenants.
    pub fn total_active_score(&self) -> u64 {
        self.covenants
            .values()
            .map(|cv| cv.patronage_score)
            .fold(0u64, |a, b| a.saturating_add(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cv(oid: u8, expires: u64, pre_funded: u64, score: u64) -> PatronageCovenant {
        PatronageCovenant {
            object_id: vec![oid],
            namespace_id: vec![],
            donation_per_epoch: 10,
            created_epoch: 0,
            expires_epoch: expires,
            pre_funded,
            patronage_score: score,
            last_honoured_epoch: None,
        }
    }

    /// T1.20 — PatronageBook len, is_empty, iter on fresh + populated.
    #[test]
    fn t1_20_book_accessors() {
        let mut b = PatronageBook::new(vec![0xFF]);
        assert!(b.is_empty());
        assert_eq!(b.len(), 0);
        assert_eq!(b.iter().count(), 0);

        b.insert(cv(1, 100, 50, 5));
        b.insert(cv(2, 200, 30, 7));
        assert!(!b.is_empty());
        assert_eq!(b.len(), 2);
        assert_eq!(b.iter().count(), 2);
    }

    /// T1.20 — expire_all removes covenants whose expires_epoch <=
    /// the cutoff and returns them.
    #[test]
    fn t1_20_book_expire_all() {
        let mut b = PatronageBook::new(vec![0xFF]);
        b.insert(cv(1, 50, 10, 1));
        b.insert(cv(2, 100, 20, 2));
        b.insert(cv(3, 200, 30, 3));

        let expired = b.expire_all(100);
        // Should expire object_ids 1 (expires=50) + 2 (expires=100, <=).
        assert_eq!(expired.len(), 2);
        assert_eq!(b.len(), 1);
        // The 200-expiry covenant remains.
        assert!(b.get(&[3]).is_some());
    }

    /// T1.20 — total_pre_funded + total_active_score sum across
    /// every covenant.
    #[test]
    fn t1_20_book_totals() {
        let mut b = PatronageBook::new(vec![0xFF]);
        b.insert(cv(1, 100, 50, 5));
        b.insert(cv(2, 200, 30, 7));
        assert_eq!(b.total_pre_funded(), 80);
        assert_eq!(b.total_active_score(), 12);
    }
}
