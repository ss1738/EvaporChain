//! `Region` — one sealed span + state machine.

use serde::{Deserialize, Serialize};

pub const REGION_TAG: &[u8] = b"evaporchain:sealed-region:v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegionState {
    /// Energy above floor; can be replaced by a higher-energy
    /// competitor at the same height.
    Tentative,
    /// Energy decayed below the chain's finality floor; locked
    /// forever. Cannot be replaced even by a higher-energy seal.
    Frozen { frozen_at_epoch: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    /// Inclusive lower bound of the span.
    pub span_lo: u64,
    /// Exclusive upper bound of the span. `span_hi > span_lo`.
    pub span_hi: u64,
    /// Block height the seal lives at.
    pub height: u64,
    /// 32-byte state-root over the spanned keys.
    pub state_root: [u8; 32],
    /// Initial energy of the seal. Decays toward the chain's
    /// finality floor; below floor → freeze.
    pub energy: u64,
    /// Epoch at which the seal was emitted.
    pub sealed_at_epoch: u64,
    pub state: RegionState,
}

impl Region {
    pub fn new(
        span_lo: u64,
        span_hi: u64,
        height: u64,
        state_root: [u8; 32],
        energy: u64,
        sealed_at_epoch: u64,
    ) -> Option<Self> {
        if span_hi <= span_lo {
            return None; // empty span — invalid
        }
        if energy == 0 {
            return None; // zero-energy seal — meaningless
        }
        Some(Self {
            span_lo,
            span_hi,
            height,
            state_root,
            energy,
            sealed_at_epoch,
            state: RegionState::Tentative,
        })
    }

    pub fn is_frozen(&self) -> bool {
        matches!(self.state, RegionState::Frozen { .. })
    }

    /// True iff the two regions' spans overlap (half-open).
    pub fn overlaps(&self, other: &Region) -> bool {
        self.span_lo < other.span_hi && other.span_lo < self.span_hi
    }

    /// Domain-tagged BLAKE3 commitment binding all fields except
    /// `state` (which is a derived runtime view).
    pub fn commitment(&self) -> [u8; 32] {
        region_commitment(
            self.span_lo,
            self.span_hi,
            self.height,
            &self.state_root,
            self.energy,
            self.sealed_at_epoch,
        )
    }
}

pub fn region_commitment(
    span_lo: u64,
    span_hi: u64,
    height: u64,
    state_root: &[u8; 32],
    energy: u64,
    sealed_at_epoch: u64,
) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(REGION_TAG);
    h.update(&span_lo.to_le_bytes());
    h.update(&span_hi.to_le_bytes());
    h.update(&height.to_le_bytes());
    h.update(state_root);
    h.update(&energy.to_le_bytes());
    h.update(&sealed_at_epoch.to_le_bytes());
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(lo: u64, hi: u64, h: u64, e: u64) -> Region {
        Region::new(lo, hi, h, [0xAA; 32], e, 0).unwrap()
    }

    #[test]
    fn rejects_empty_span() {
        assert!(Region::new(10, 10, 0, [0; 32], 1, 0).is_none());
        assert!(Region::new(20, 10, 0, [0; 32], 1, 0).is_none());
    }

    #[test]
    fn rejects_zero_energy() {
        assert!(Region::new(0, 10, 0, [0; 32], 0, 0).is_none());
    }

    #[test]
    fn overlaps_basic_cases() {
        let a = r(0, 10, 0, 1);
        let b = r(5, 15, 0, 1); // overlaps a
        let c = r(10, 20, 0, 1); // adjacent — does NOT overlap (half-open)
        let d = r(20, 30, 0, 1); // disjoint
        assert!(a.overlaps(&b));
        assert!(!a.overlaps(&c));
        assert!(!a.overlaps(&d));
        assert!(b.overlaps(&c)); // 5..15 ∩ 10..20 = 10..15
    }

    #[test]
    fn overlaps_is_symmetric() {
        let a = r(0, 10, 0, 1);
        let b = r(5, 15, 0, 1);
        assert_eq!(a.overlaps(&b), b.overlaps(&a));
    }

    #[test]
    fn commitment_changes_with_each_field() {
        let base = r(0, 10, 0, 1000);
        let h0 = base.commitment();

        let a = r(1, 10, 0, 1000);
        assert_ne!(a.commitment(), h0);
        let b = r(0, 11, 0, 1000);
        assert_ne!(b.commitment(), h0);
        let c = r(0, 10, 1, 1000);
        assert_ne!(c.commitment(), h0);
        let d = r(0, 10, 0, 999);
        assert_ne!(d.commitment(), h0);
    }

    #[test]
    fn commitment_uses_domain_tag() {
        let region = r(0, 10, 0, 1000);
        let with = region.commitment();
        let mut naive = blake3::Hasher::new();
        naive.update(&region.span_lo.to_le_bytes());
        naive.update(&region.span_hi.to_le_bytes());
        naive.update(&region.height.to_le_bytes());
        naive.update(&region.state_root);
        naive.update(&region.energy.to_le_bytes());
        naive.update(&region.sealed_at_epoch.to_le_bytes());
        let without: [u8; 32] = *naive.finalize().as_bytes();
        assert_ne!(with, without);
    }

    #[test]
    fn round_trip_serde() {
        let region = r(0, 10, 0, 1000);
        let s = serde_json::to_string(&region).unwrap();
        let back: Region = serde_json::from_str(&s).unwrap();
        assert_eq!(region, back);
    }
}
