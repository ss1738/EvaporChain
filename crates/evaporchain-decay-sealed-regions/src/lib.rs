//! Decay-Sealed Regions — sub-block finality primitive.
//!
//! ## What this is
//!
//! A "region" is a contiguous half-open span `[lo, hi)` of state
//! keys (or equivalent index space) that the chain commits to as
//! **sealed** mid-block. The seal carries:
//! 1. The state-root hash over the region.
//! 2. An energy budget that decays from creation forward.
//! 3. A finality floor: when the seal's energy crosses below,
//!    the region transitions from `Tentative` to `Frozen` —
//!    cryptographically locked.
//!
//! Once `Frozen`, no validator can produce a competing seal over
//! the same span (or any span overlapping it). This gives the
//! chain a structural sub-block finality knob: hot regions of
//! state can be finalized faster than the block-level finality
//! by attaching extra-energy seals at production time.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Span-disjoint at any height.** Two seals at the same
//!    height with overlapping spans: the *higher-energy* seal
//!    wins; the registry refuses to register the lower-energy
//!    overlap. Pure thermal priority — same rule as Thermal-STM.
//!
//! 2. **Frozen seals are immutable.** Once energy decays below
//!    the finality floor and the chain calls `freeze_below_floor`,
//!    the seal cannot be replaced even by a higher-energy seal.
//!    Frozen is a one-way transition — sub-block finality is
//!    real finality.
//!
//! 3. **Domain-separated commitment.** Tag is
//!    `b"evaporchain:sealed-region:v1\0"`; binds (span_lo,
//!    span_hi, height, state_root, energy, sealed_at_epoch).
//!    Anti-replay: changing any field changes the commitment.
//!
//! ## Module map
//!
//! - [`region`] — [`Region`] descriptor + commitment hash.
//! - [`registry`] — [`SealRegistry`] with overlap-check + freeze.

pub mod region;
pub mod registry;

pub use region::{region_commitment, Region, RegionState, REGION_TAG};
pub use registry::{RegistryError, SealRegistry};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Decay-Sealed Regions provides sub-block finality:
    /// at any height, two overlapping seals resolve by thermal
    /// priority (higher energy wins; lower-energy tentative is
    /// evicted). Frozen seals are IMMUTABLE — cannot be replaced
    /// even by a higher-energy seal. The freeze transition is
    /// one-way: sub-block finality is real finality."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let mut reg = SealRegistry::new();

        // First seal at height 1, span [0, 100), energy 1000.
        let r_low = Region::new(0, 100, 1, [0xAA; 32], 1_000, 0).unwrap();
        reg.register(r_low.clone()).unwrap();

        // Overlapping seal with HIGHER energy → loser-evicted, winner registered.
        let r_high = Region::new(50, 200, 1, [0xBB; 32], 5_000, 0).unwrap();
        reg.register(r_high.clone()).unwrap();
        // r_low must be gone (lower energy got evicted).
        assert!(reg.get(&r_low.commitment()).is_none());
        assert!(reg.get(&r_high.commitment()).is_some());

        // Overlapping seal with EQUAL-OR-LOWER energy → rejected.
        let r_equal = Region::new(60, 80, 1, [0xCC; 32], 5_000, 0).unwrap();
        assert!(matches!(
            reg.register(r_equal),
            Err(RegistryError::OverlappingSealHigherEnergy { .. })
        ));

        // Freeze sweep: drop r_high's energy, then sweep below floor.
        reg.set_energy(&r_high.commitment(), 100).unwrap();
        let frozen_count = reg.freeze_below_floor(500, 50);
        assert_eq!(frozen_count, 1);

        // Frozen seal cannot be replaced by a higher-energy seal.
        let r_super = Region::new(70, 90, 1, [0xDD; 32], 99_999, 0).unwrap();
        assert!(matches!(
            reg.register(r_super),
            Err(RegistryError::OverlappingFrozenSeal)
        ));

        // set_energy on a frozen seal is rejected.
        assert!(matches!(
            reg.set_energy(&r_high.commitment(), 9_999),
            Err(RegistryError::AlreadyFrozen)
        ));

        // Construction guards: empty span and zero energy → None.
        assert!(Region::new(10, 10, 1, [0; 32], 100, 0).is_none());
        assert!(Region::new(0, 100, 1, [0; 32], 0, 0).is_none());

        // Domain-tagged commitment: changing any input field changes
        // the commitment.
        let c1 = region_commitment(0, 100, 1, &[0; 32], 1_000, 0);
        let c2 = region_commitment(0, 100, 2, &[0; 32], 1_000, 0);
        assert_ne!(c1, c2);
    }
}
