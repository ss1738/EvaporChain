//! `JailState` — canonical map of jailed validators.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type ValidatorId = [u8; 32];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JailReason {
    /// Validator was active during a window whose BellCertificate
    /// failed the CHSH gate.
    ChshFailedWindow { window_start: u64, window_end: u64 },
    /// Validator's energy fell below the floor.
    EnergyBelowFloor { observed: u64, floor: u64 },
    /// Operator-issued slash with a typed code.
    Slashed { code: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JailEntry {
    pub reason: JailReason,
    /// Epoch at which the jail expires (exclusive). The validator
    /// is jailed for any epoch `e < expires_at_epoch`.
    pub expires_at_epoch: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JailState {
    /// Canonical (BTreeMap) so iteration is deterministic across
    /// validators.
    inner: BTreeMap<ValidatorId, JailEntry>,
}

impl JailState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: ValidatorId, entry: JailEntry) {
        self.inner.insert(id, entry);
    }

    pub fn remove(&mut self, id: &ValidatorId) -> Option<JailEntry> {
        self.inner.remove(id)
    }

    pub fn get(&self, id: &ValidatorId) -> Option<&JailEntry> {
        self.inner.get(id)
    }

    /// Is the validator currently jailed at `current_epoch`?
    /// Expired entries are not auto-pruned by this query — call
    /// `prune_expired` to clean them up.
    pub fn is_jailed(&self, id: &ValidatorId, current_epoch: u64) -> bool {
        match self.inner.get(id) {
            Some(e) => current_epoch < e.expires_at_epoch,
            None => false,
        }
    }

    /// Remove every entry whose expiry is `≤ current_epoch`. Returns
    /// the number of entries pruned.
    pub fn prune_expired(&mut self, current_epoch: u64) -> usize {
        let to_remove: Vec<ValidatorId> = self
            .inner
            .iter()
            .filter(|(_, e)| e.expires_at_epoch <= current_epoch)
            .map(|(k, _)| *k)
            .collect();
        let n = to_remove.len();
        for k in to_remove {
            self.inner.remove(&k);
        }
        n
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Canonical iterator.
    pub fn iter(&self) -> impl Iterator<Item = (&ValidatorId, &JailEntry)> {
        self.inner.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> ValidatorId {
        [b; 32]
    }

    fn entry(expires: u64) -> JailEntry {
        JailEntry {
            reason: JailReason::Slashed { code: 0 },
            expires_at_epoch: expires,
        }
    }

    #[test]
    fn empty_state_no_one_jailed() {
        let s = JailState::new();
        assert!(!s.is_jailed(&id(1), 100));
    }

    #[test]
    fn insert_then_query() {
        let mut s = JailState::new();
        s.insert(id(1), entry(50));
        assert!(s.is_jailed(&id(1), 0));
        assert!(s.is_jailed(&id(1), 49));
        assert!(!s.is_jailed(&id(1), 50)); // exclusive
        assert!(!s.is_jailed(&id(1), 1000));
    }

    #[test]
    fn prune_expired_removes_old_entries() {
        let mut s = JailState::new();
        s.insert(id(1), entry(50));
        s.insert(id(2), entry(100));
        s.insert(id(3), entry(150));
        let pruned = s.prune_expired(100);
        // Entries with expires_at ≤ 100 → entries 1 (expires 50) and 2 (expires 100).
        assert_eq!(pruned, 2);
        assert_eq!(s.len(), 1);
        assert!(s.get(&id(3)).is_some());
    }

    #[test]
    fn iteration_is_canonical() {
        // Insert in non-sorted order; iterator yields BTreeMap (sorted) order.
        let mut s = JailState::new();
        s.insert(id(3), entry(50));
        s.insert(id(1), entry(50));
        s.insert(id(2), entry(50));
        let ids: Vec<ValidatorId> = s.iter().map(|(k, _)| *k).collect();
        assert_eq!(ids, vec![id(1), id(2), id(3)]);
    }

    #[test]
    fn jail_reason_round_trip_serde() {
        let r = JailReason::ChshFailedWindow {
            window_start: 100,
            window_end: 200,
        };
        let s = serde_json::to_string(&r).unwrap();
        let r2: JailReason = serde_json::from_str(&s).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn replace_overwrites_existing_entry() {
        let mut s = JailState::new();
        s.insert(id(1), entry(50));
        s.insert(
            id(1),
            JailEntry {
                reason: JailReason::EnergyBelowFloor {
                    observed: 0,
                    floor: 10,
                },
                expires_at_epoch: 200,
            },
        );
        assert_eq!(s.len(), 1);
        let e = s.get(&id(1)).unwrap();
        assert_eq!(e.expires_at_epoch, 200);
        assert!(matches!(e.reason, JailReason::EnergyBelowFloor { .. }));
    }
}
