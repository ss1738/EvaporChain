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
