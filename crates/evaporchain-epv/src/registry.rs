//! `EpvRegistry` — the active set of protocol versions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};
use evaporchain_types::Energy;

/// Protocol-version identifier — opaque `u32` index. Concrete version
/// strings (semver, etc.) are mapped to / from this `u32` outside the
/// crate; EPV's job is the energy/decay accounting only.
pub type VersionId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub id: VersionId,
    /// Seed energy credited at `activated_epoch`.
    pub seed_energy: Energy,
    /// Epoch at which this version was activated (its native "now").
    pub activated_epoch: u64,
}

impl ProtocolVersion {
    pub const fn new(id: VersionId, seed_energy: Energy, activated_epoch: u64) -> Self {
        Self {
            id,
            seed_energy,
            activated_epoch,
        }
    }

    /// Remaining energy after λ-decay from `activated_epoch` to `t`.
    /// Future-`t` (relative to activation) just clamps to seed.
    pub fn remaining_at(&self, chain_lambda: ChainLambda, t: u64) -> Energy {
        if t <= self.activated_epoch {
            return self.seed_energy;
        }
        let elapsed = t - self.activated_epoch;
        energy_at_epoch(self.seed_energy, chain_lambda.half_life(), elapsed)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EpvRegistry {
    versions: BTreeMap<VersionId, ProtocolVersion>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("protocol version {0} already registered")]
    AlreadyRegistered(VersionId),
    #[error("protocol version {0} not registered")]
    UnknownVersion(VersionId),
}

impl EpvRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.versions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.versions.is_empty()
    }

    pub fn register(&mut self, v: ProtocolVersion) -> Result<(), RegistryError> {
        if self.versions.contains_key(&v.id) {
            return Err(RegistryError::AlreadyRegistered(v.id));
        }
        self.versions.insert(v.id, v);
        Ok(())
    }

    pub fn get(&self, id: VersionId) -> Option<&ProtocolVersion> {
        self.versions.get(&id)
    }

    pub fn contains(&self, id: VersionId) -> bool {
        self.versions.contains_key(&id)
    }

    /// Iterate every registered version (sorted by `VersionId`).
    pub fn iter(&self) -> impl Iterator<Item = &ProtocolVersion> {
        self.versions.values()
    }

    /// Iterate `(id, version)` pairs.
    pub fn entries(&self) -> impl Iterator<Item = (&VersionId, &ProtocolVersion)> {
        self.versions.iter()
    }

    /// Mutable iter — used by `prune_evaporated` to drop entries.
    pub(crate) fn versions_mut(&mut self) -> &mut BTreeMap<VersionId, ProtocolVersion> {
        &mut self.versions
    }

    /// **The consensus gate.** True iff version `v` is still runnable
    /// at chain time `t` — i.e. registered AND has remaining energy
    /// strictly above `e_min`.
    pub fn is_runnable(&self, v: VersionId, chain_lambda: ChainLambda, t: u64, e_min: Energy) -> bool {
        match self.versions.get(&v) {
            Some(version) => version.remaining_at(chain_lambda, t) > e_min,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn empty_registry_no_versions_runnable() {
        let r = EpvRegistry::new();
        assert!(!r.is_runnable(0, lambda(), 0, 0));
    }

    #[test]
    fn fresh_version_is_runnable() {
        let mut r = EpvRegistry::new();
        r.register(ProtocolVersion::new(1, 1000, 0)).unwrap();
        assert!(r.is_runnable(1, lambda(), 0, 100));
    }

    #[test]
    fn double_registration_rejected() {
        let mut r = EpvRegistry::new();
        r.register(ProtocolVersion::new(1, 1000, 0)).unwrap();
        let err = r.register(ProtocolVersion::new(1, 999, 5)).unwrap_err();
        assert_eq!(err, RegistryError::AlreadyRegistered(1));
    }

    #[test]
    fn version_below_e_min_not_runnable() {
        let mut r = EpvRegistry::new();
        r.register(ProtocolVersion::new(1, 1000, 0)).unwrap();
        // After lots of half-lives the remaining is well below 999.
        assert!(!r.is_runnable(1, lambda(), 1_000, 999));
    }

    #[test]
    fn remaining_at_pre_activation_returns_seed() {
        let v = ProtocolVersion::new(1, 1000, 50);
        assert_eq!(v.remaining_at(lambda(), 0), 1000);
        assert_eq!(v.remaining_at(lambda(), 50), 1000);
    }

    #[test]
    fn remaining_at_one_halflife_is_half() {
        let v = ProtocolVersion::new(1, 1000, 0);
        assert_eq!(v.remaining_at(lambda(), 100), 500);
    }
}
