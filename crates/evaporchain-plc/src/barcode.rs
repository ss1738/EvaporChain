//! `Barcode` — a multiset of `(birth, death)` intervals.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const BARCODE_TAG: &[u8] = b"evaporchain:plc:barcode:v1\0";

/// Sentinel for an "infinite-death" bar — a feature that survives
/// to the end of the filtration. We use `u64::MAX` rather than a
/// separate enum to keep the canonical-bytes path purely
/// integer-valued.
pub const INF_DEATH: u64 = u64::MAX;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BarcodeError {
    #[error("bar {birth}→{death} is invalid (death must be > birth, except for ∞ bars)")]
    InvalidBar { birth: u64, death: u64 },
}

/// One persistence interval. `death == INF_DEATH` denotes an
/// infinite bar (feature surviving to the end of the filtration).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Bar {
    pub birth: u64,
    pub death: u64,
}

impl Bar {
    pub fn new(birth: u64, death: u64) -> Result<Self, BarcodeError> {
        if death == INF_DEATH || death > birth {
            Ok(Self { birth, death })
        } else {
            Err(BarcodeError::InvalidBar { birth, death })
        }
    }

    pub fn is_infinite(&self) -> bool {
        self.death == INF_DEATH
    }

    /// Persistence (lifetime) of the bar. Infinite for ∞-bars
    /// (returns `INF_DEATH`).
    pub fn persistence(&self) -> u64 {
        if self.is_infinite() {
            INF_DEATH
        } else {
            self.death.saturating_sub(self.birth)
        }
    }
}

/// A canonically-ordered multiset of bars.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Barcode {
    /// Bars sorted by `(birth, death)` lex. Validators agree
    /// byte-for-byte on the canonical encoding.
    bars: Vec<Bar>,
}

impl Barcode {
    /// Build a canonical barcode from any iterable of bars.
    /// Sorts and stores; duplicates allowed (it's a multiset).
    pub fn from_bars<I: IntoIterator<Item = Bar>>(it: I) -> Self {
        let mut bars: Vec<Bar> = it.into_iter().collect();
        bars.sort_by_key(|b| (b.birth, b.death));
        Self { bars }
    }

    pub fn len(&self) -> usize {
        self.bars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }

    pub fn bars(&self) -> &[Bar] {
        &self.bars
    }

    /// Domain-tagged BLAKE3 of the canonical barcode encoding.
    pub fn hash(&self) -> [u8; 32] {
        let mut h = blake3::Hasher::new();
        h.update(BARCODE_TAG);
        h.update(&(self.bars.len() as u64).to_le_bytes());
        for b in &self.bars {
            h.update(&b.birth.to_le_bytes());
            h.update(&b.death.to_le_bytes());
        }
        *h.finalize().as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_validation_rejects_zero_persistence() {
        // death == birth is degenerate.
        let err = Bar::new(10, 10).unwrap_err();
        assert!(matches!(err, BarcodeError::InvalidBar { .. }));
    }

    #[test]
    fn bar_validation_rejects_negative_persistence() {
        let err = Bar::new(10, 5).unwrap_err();
        assert!(matches!(err, BarcodeError::InvalidBar { .. }));
    }

    #[test]
    fn bar_with_infinite_death_is_valid() {
        Bar::new(10, INF_DEATH).unwrap();
    }

    #[test]
    fn persistence_is_difference_for_finite() {
        let b = Bar::new(5, 17).unwrap();
        assert_eq!(b.persistence(), 12);
    }

    #[test]
    fn persistence_is_inf_for_infinite() {
        let b = Bar::new(5, INF_DEATH).unwrap();
        assert!(b.is_infinite());
        assert_eq!(b.persistence(), INF_DEATH);
    }

    #[test]
    fn barcode_canonicalises_bar_order() {
        let bars = vec![
            Bar::new(5, 10).unwrap(),
            Bar::new(1, 3).unwrap(),
            Bar::new(2, 7).unwrap(),
        ];
        let bc = Barcode::from_bars(bars);
        let births: Vec<u64> = bc.bars().iter().map(|b| b.birth).collect();
        assert_eq!(births, vec![1, 2, 5]);
    }

    #[test]
    fn barcode_hash_is_deterministic() {
        let a = Barcode::from_bars(vec![Bar::new(1, 5).unwrap(), Bar::new(2, 3).unwrap()]);
        let b = Barcode::from_bars(vec![Bar::new(2, 3).unwrap(), Bar::new(1, 5).unwrap()]);
        // Same multiset, different insertion order → same hash.
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn barcode_hash_changes_with_bar_change() {
        let a = Barcode::from_bars(vec![Bar::new(1, 5).unwrap()]);
        let b = Barcode::from_bars(vec![Bar::new(1, 6).unwrap()]);
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn empty_barcode_hash_is_well_defined() {
        let a = Barcode::default();
        let b = Barcode::from_bars(Vec::new());
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn barcode_uses_domain_tag() {
        // Hashing the same content WITHOUT the tag must differ.
        let bc = Barcode::from_bars(vec![Bar::new(1, 5).unwrap()]);
        let with = bc.hash();
        let mut naive = blake3::Hasher::new();
        naive.update(&(1u64).to_le_bytes());
        naive.update(&1u64.to_le_bytes());
        naive.update(&5u64.to_le_bytes());
        let without: [u8; 32] = *naive.finalize().as_bytes();
        assert_ne!(with, without);
    }

    #[test]
    fn round_trip_serde() {
        let bc = Barcode::from_bars(vec![
            Bar::new(1, 5).unwrap(),
            Bar::new(2, INF_DEATH).unwrap(),
        ]);
        let s = serde_json::to_string(&bc).unwrap();
        let back: Barcode = serde_json::from_str(&s).unwrap();
        assert_eq!(bc, back);
    }

    /// T1.20 — minimal-persistence boundary: `death == birth + 1` is
    /// valid (persistence = 1); `death == birth` is not. Existing
    /// tests cover the `==` rejection and a multi-unit happy path
    /// but not the smallest valid bar.
    #[test]
    fn t1_20_bar_minimal_persistence_is_one() {
        let b = Bar::new(10, 11).unwrap();
        assert_eq!(b.persistence(), 1);
        assert!(!b.is_infinite());
        // Death exactly equal to birth is still rejected.
        assert!(Bar::new(10, 10).is_err());
    }

    /// T1.20 — `Barcode` is a MULTISET: duplicate bars produce a
    /// DIFFERENT hash than a single bar. This pins that the `len`
    /// field is in the digest and that duplicates are preserved
    /// rather than collapsed. A future refactor to `BTreeSet<Bar>`
    /// (which would deduplicate) would silently break the chain's
    /// barcode commitments.
    #[test]
    fn t1_20_barcode_multiset_duplicates_produce_distinct_hash() {
        let bar = Bar::new(1, 5).unwrap();
        let single = Barcode::from_bars(vec![bar]);
        let doubled = Barcode::from_bars(vec![bar, bar]);
        let tripled = Barcode::from_bars(vec![bar, bar, bar]);
        assert_eq!(single.len(), 1);
        assert_eq!(doubled.len(), 2);
        assert_eq!(tripled.len(), 3);
        // All three hashes must differ.
        assert_ne!(single.hash(), doubled.hash());
        assert_ne!(doubled.hash(), tripled.hash());
        assert_ne!(single.hash(), tripled.hash());
    }

    /// T1.20 — the length prefix is the soundness gate that
    /// prevents the "1 bar (1,5)" from colliding with "0 bars"
    /// padded by 16 bytes of zeros (or other length-confusion
    /// attacks). Pin by recomputing the hash without the length
    /// prefix and checking it differs.
    #[test]
    fn t1_20_barcode_hash_includes_length_prefix() {
        let bc = Barcode::from_bars(vec![Bar::new(1, 5).unwrap()]);
        let with_len = bc.hash();
        let without_len = {
            let mut h = blake3::Hasher::new();
            h.update(BARCODE_TAG);
            // SKIP the length prefix — just write the bar bytes.
            h.update(&1u64.to_le_bytes()); // birth
            h.update(&5u64.to_le_bytes()); // death
            *h.finalize().as_bytes()
        };
        assert_ne!(
            with_len, without_len,
            "barcode hash must include the multiset length"
        );
    }
}
