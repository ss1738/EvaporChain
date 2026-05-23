//! `SealRegistry` — overlap-aware seal table + freeze gate.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::region::{Region, RegionState};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error(
        "incoming seal at height {height} (energy {incoming}) overlaps a higher-energy seal (energy {existing}); rejected by thermal priority"
    )]
    OverlappingSealHigherEnergy {
        height: u64,
        incoming: u64,
        existing: u64,
    },
    #[error(
        "incoming seal overlaps a FROZEN seal — sub-block finality is real, replacement forbidden"
    )]
    OverlappingFrozenSeal,
    #[error("region commitment {commitment:?} not found")]
    NotFound { commitment: [u8; 32] },
    #[error("seal already frozen — cannot decay-update further")]
    AlreadyFrozen,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SealRegistry {
    /// Map commitment -> region. Pure runtime state.
    seals: HashMap<[u8; 32], Region>,
}

impl SealRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.seals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seals.is_empty()
    }

    pub fn get(&self, commitment: &[u8; 32]) -> Option<&Region> {
        self.seals.get(commitment)
    }

    /// All seals at a given height. Iteration order is stable
    /// only within a single process; for validator-determinism the
    /// chain should sort by `commitment` after enumerating.
    pub fn at_height(&self, height: u64) -> impl Iterator<Item = &Region> {
        self.seals.values().filter(move |r| r.height == height)
    }

    /// Try to register a new seal. Three outcomes:
    /// - **Disjoint** at height ⇒ accept.
    /// - **Overlap with FROZEN** seal ⇒ `OverlappingFrozenSeal`.
    /// - **Overlap with TENTATIVE** seal of higher-or-equal energy
    ///   ⇒ `OverlappingSealHigherEnergy`.
    /// - **Overlap with TENTATIVE** seal of strictly lower energy
    ///   ⇒ remove the loser, accept the new one.
    ///
    /// Idempotent on commitment: re-registering the *same* region
    /// is a no-op success (but does NOT replace itself with a
    /// duplicate entry).
    pub fn register(&mut self, region: Region) -> Result<[u8; 32], RegistryError> {
        let commitment = region.commitment();

        if self.seals.contains_key(&commitment) {
            return Ok(commitment);
        }

        // Find any overlap at the same height.
        let mut to_remove: Vec<[u8; 32]> = Vec::new();
        for (cmt, existing) in &self.seals {
            if existing.height != region.height || !existing.overlaps(&region) {
                continue;
            }
            // Overlap.
            if existing.is_frozen() {
                return Err(RegistryError::OverlappingFrozenSeal);
            }
            // Both tentative: thermal priority.
            if existing.energy >= region.energy {
                return Err(RegistryError::OverlappingSealHigherEnergy {
                    height: region.height,
                    incoming: region.energy,
                    existing: existing.energy,
                });
            } else {
                // Existing has strictly less energy → loser, evicted.
                to_remove.push(*cmt);
            }
        }
        for cmt in to_remove {
            self.seals.remove(&cmt);
        }
        self.seals.insert(commitment, region);
        Ok(commitment)
    }

    /// Decay tick: the chain calls this with the post-decay
    /// energy for one seal. Cannot lower a frozen seal.
    pub fn set_energy(
        &mut self,
        commitment: &[u8; 32],
        new_energy: u64,
    ) -> Result<(), RegistryError> {
        let r = self
            .seals
            .get_mut(commitment)
            .ok_or(RegistryError::NotFound {
                commitment: *commitment,
            })?;
        if r.is_frozen() {
            return Err(RegistryError::AlreadyFrozen);
        }
        r.energy = new_energy;
        Ok(())
    }

    /// Sweep: any tentative seal whose energy is `<= floor` gets
    /// frozen at the given epoch. Returns count of newly-frozen
    /// seals.
    pub fn freeze_below_floor(&mut self, floor: u64, at_epoch: u64) -> usize {
        let mut frozen = 0usize;
        for r in self.seals.values_mut() {
            if matches!(r.state, RegionState::Tentative) && r.energy <= floor {
                r.state = RegionState::Frozen {
                    frozen_at_epoch: at_epoch,
                };
                frozen += 1;
            }
        }
        frozen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::region::Region;

    fn r(lo: u64, hi: u64, h: u64, e: u64) -> Region {
        Region::new(lo, hi, h, [0xAA; 32], e, 0).unwrap()
    }

    fn r_with_root(lo: u64, hi: u64, h: u64, e: u64, root: u8) -> Region {
        Region::new(lo, hi, h, [root; 32], e, 0).unwrap()
    }

    // ── disjoint and idempotent ─────────────────────────────────

    #[test]
    fn disjoint_seals_at_same_height_accept() {
        let mut reg = SealRegistry::new();
        reg.register(r(0, 10, 0, 100)).unwrap();
        reg.register(r(20, 30, 0, 100)).unwrap();
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn seals_at_different_heights_never_conflict() {
        let mut reg = SealRegistry::new();
        reg.register(r(0, 10, 0, 100)).unwrap();
        reg.register(r(0, 10, 1, 1)).unwrap(); // same span, different height
        reg.register(r(0, 10, 2, 1)).unwrap();
        assert_eq!(reg.len(), 3);
    }

    #[test]
    fn re_registering_same_seal_is_idempotent() {
        let mut reg = SealRegistry::new();
        let region = r(0, 10, 0, 100);
        let c1 = reg.register(region.clone()).unwrap();
        let c2 = reg.register(region).unwrap();
        assert_eq!(c1, c2);
        assert_eq!(reg.len(), 1);
    }

    // ── overlap + thermal priority ───────────────────────────────

    #[test]
    fn overlap_higher_energy_wins() {
        let mut reg = SealRegistry::new();
        let weak = r_with_root(0, 10, 0, 50, 0xAA);
        reg.register(weak.clone()).unwrap();

        let strong = r_with_root(5, 15, 0, 100, 0xBB);
        reg.register(strong).unwrap();

        // Weak should be evicted.
        assert!(reg.get(&weak.commitment()).is_none());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn overlap_lower_energy_rejected() {
        let mut reg = SealRegistry::new();
        let strong = r_with_root(0, 10, 0, 100, 0xAA);
        reg.register(strong.clone()).unwrap();

        let weak = r_with_root(5, 15, 0, 50, 0xBB);
        let err = reg.register(weak).unwrap_err();
        assert!(matches!(
            err,
            RegistryError::OverlappingSealHigherEnergy {
                existing: 100,
                incoming: 50,
                ..
            }
        ));
        assert!(reg.get(&strong.commitment()).is_some());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn overlap_equal_energy_existing_wins() {
        // Tie-breaker: existing wins on equal-energy (existing >= incoming).
        let mut reg = SealRegistry::new();
        let first = r_with_root(0, 10, 0, 100, 0xAA);
        reg.register(first.clone()).unwrap();
        let same_energy = r_with_root(5, 15, 0, 100, 0xBB);
        let err = reg.register(same_energy).unwrap_err();
        assert!(matches!(
            err,
            RegistryError::OverlappingSealHigherEnergy { .. }
        ));
    }

    // ── freeze + immutability ────────────────────────────────────

    #[test]
    fn freeze_below_floor_marks_eligible_seals() {
        let mut reg = SealRegistry::new();
        let r1 = r_with_root(0, 10, 0, 100, 0xAA);
        let r2 = r_with_root(20, 30, 0, 5, 0xBB);
        reg.register(r1.clone()).unwrap();
        reg.register(r2.clone()).unwrap();

        let frozen = reg.freeze_below_floor(10, 50);
        assert_eq!(frozen, 1);
        assert!(!reg.get(&r1.commitment()).unwrap().is_frozen());
        assert!(reg.get(&r2.commitment()).unwrap().is_frozen());
    }

    #[test]
    fn cannot_replace_frozen_seal_even_with_higher_energy() {
        let mut reg = SealRegistry::new();
        let weak = r_with_root(0, 10, 0, 5, 0xAA);
        reg.register(weak.clone()).unwrap();
        reg.freeze_below_floor(10, 50);
        assert!(reg.get(&weak.commitment()).unwrap().is_frozen());

        // Try to overlap with a much higher energy seal.
        let strong = r_with_root(5, 15, 0, u64::MAX, 0xBB);
        let err = reg.register(strong).unwrap_err();
        assert_eq!(err, RegistryError::OverlappingFrozenSeal);
    }

    #[test]
    fn cannot_set_energy_on_frozen_seal() {
        let mut reg = SealRegistry::new();
        let r1 = r_with_root(0, 10, 0, 5, 0xAA);
        let cmt = r1.commitment();
        reg.register(r1).unwrap();
        reg.freeze_below_floor(10, 50);
        let err = reg.set_energy(&cmt, 1000).unwrap_err();
        assert_eq!(err, RegistryError::AlreadyFrozen);
    }

    #[test]
    fn set_energy_unknown_seal_errors() {
        let mut reg = SealRegistry::new();
        let err = reg.set_energy(&[0xFF; 32], 100).unwrap_err();
        assert!(matches!(err, RegistryError::NotFound { .. }));
    }

    // ── decay path ───────────────────────────────────────────────

    #[test]
    fn decay_then_freeze_locks() {
        let mut reg = SealRegistry::new();
        let r1 = r_with_root(0, 10, 0, 1000, 0xAA);
        let cmt = r1.commitment();
        reg.register(r1).unwrap();
        // Pre-freeze: replaceable.
        assert!(!reg.get(&cmt).unwrap().is_frozen());
        // Decay below floor.
        reg.set_energy(&cmt, 5).unwrap();
        let frozen = reg.freeze_below_floor(10, 50);
        assert_eq!(frozen, 1);
        // Now immutable.
        assert!(reg.set_energy(&cmt, 1000).is_err());
    }

    // ── doctrine claim ───────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Decay-Sealed Regions are EvaporChain's
        // sub-block finality. Hot regions can be sealed mid-block
        // with extra energy; once their energy decays past
        // finality floor, the region is cryptographically locked
        // and no validator — even a higher-energy one — can
        // replace it. Sub-block finality is real finality."

        let mut reg = SealRegistry::new();

        // Hot region: a high-energy mid-block seal over span 0..100.
        let hot = r_with_root(0, 100, 5, 1000, 0xAA);
        let hot_cmt = hot.commitment();
        reg.register(hot).unwrap();

        // Time passes; hot region's energy decays.
        reg.set_energy(&hot_cmt, 5).unwrap();
        let frozen = reg.freeze_below_floor(10, 100);
        assert_eq!(frozen, 1);
        assert!(reg.get(&hot_cmt).unwrap().is_frozen());

        // Adversary submits a competing seal over an overlapping
        // span at the same height with WAY more energy.
        let usurper = r_with_root(50, 150, 5, u64::MAX, 0xBB);
        let err = reg.register(usurper).unwrap_err();
        assert_eq!(err, RegistryError::OverlappingFrozenSeal);
    }

    proptest::proptest! {
        #[test]
        fn property_disjoint_spans_always_coexist(
            n in 2usize..10usize,
            stride in 10u64..100u64,
        ) {
            // n strictly-disjoint spans at the same height,
            // arbitrary energies, all admit.
            let mut reg = SealRegistry::new();
            for i in 0..n {
                let lo = (i as u64) * stride;
                let hi = lo + stride / 2;
                let region = r_with_root(lo, hi, 0, (i as u64 + 1) * 10, i as u8 + 1);
                proptest::prop_assert!(reg.register(region).is_ok());
            }
            proptest::prop_assert_eq!(reg.len(), n);
        }
    }
}
