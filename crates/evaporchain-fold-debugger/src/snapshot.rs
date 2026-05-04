//! `Snapshot` — one folded checkpoint.

use serde::{Deserialize, Serialize};

pub const SNAPSHOT_TAG: &[u8] = b"evaporchain:fold-snapshot:v1\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub height: u64,
    pub state_root: [u8; 32],
    /// 32-byte fold-accumulator commitment carried into the next
    /// fold step. Treated opaquely by this crate.
    pub fold_accumulator: [u8; 32],
}

impl Snapshot {
    pub fn new(height: u64, state_root: [u8; 32], fold_accumulator: [u8; 32]) -> Self {
        Self {
            height,
            state_root,
            fold_accumulator,
        }
    }

    pub fn id(&self) -> [u8; 32] {
        snapshot_id(self.height, &self.state_root, &self.fold_accumulator)
    }
}

pub fn snapshot_id(height: u64, state_root: &[u8; 32], fold_accumulator: &[u8; 32]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(SNAPSHOT_TAG);
    h.update(&height.to_le_bytes());
    h.update(state_root);
    h.update(fold_accumulator);
    *h.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_deterministic() {
        let s = Snapshot::new(100, [0xAA; 32], [0xBB; 32]);
        assert_eq!(s.id(), s.id());
    }

    #[test]
    fn id_changes_with_each_field() {
        let base = Snapshot::new(100, [0xAA; 32], [0xBB; 32]);
        let h0 = base.id();
        assert_ne!(Snapshot::new(101, [0xAA; 32], [0xBB; 32]).id(), h0);
        assert_ne!(Snapshot::new(100, [0xCC; 32], [0xBB; 32]).id(), h0);
        assert_ne!(Snapshot::new(100, [0xAA; 32], [0xDD; 32]).id(), h0);
    }

    #[test]
    fn id_uses_domain_tag() {
        let s = Snapshot::new(100, [0xAA; 32], [0xBB; 32]);
        let with = s.id();
        let mut naive = blake3::Hasher::new();
        naive.update(&100u64.to_le_bytes());
        naive.update(&[0xAA; 32]);
        naive.update(&[0xBB; 32]);
        let without: [u8; 32] = *naive.finalize().as_bytes();
        assert_ne!(with, without);
    }

    #[test]
    fn round_trip_serde() {
        let s = Snapshot::new(100, [0xAA; 32], [0xBB; 32]);
        let j = serde_json::to_string(&s).unwrap();
        let back: Snapshot = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
