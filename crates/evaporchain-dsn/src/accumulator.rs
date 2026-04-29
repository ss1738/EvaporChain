//! `Accumulator` — domain-separated blake3 chain of nullifiers.

use serde::{Deserialize, Serialize};

const ACC_TAG: &[u8] = b"evaporchain-dsn-acc";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Accumulator {
    pub value: [u8; 32],
    pub count: u64,
}

impl Accumulator {
    pub const fn empty() -> Self {
        Self {
            value: [0u8; 32],
            count: 0,
        }
    }

    /// Fold a 32-byte nullifier into the accumulator. Order-dependent
    /// (substrate stand-in; production uses a commutative scheme so
    /// concurrent folds compose).
    pub fn fold(self, nullifier: &[u8; 32]) -> Self {
        let mut h = blake3::Hasher::new();
        h.update(ACC_TAG);
        h.update(&self.value);
        h.update(nullifier);
        Self {
            value: *h.finalize().as_bytes(),
            count: self.count.saturating_add(1),
        }
    }
}

impl Default for Accumulator {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero() {
        let a = Accumulator::empty();
        assert_eq!(a.value, [0u8; 32]);
        assert_eq!(a.count, 0);
    }

    #[test]
    fn fold_changes_value_and_count() {
        let a = Accumulator::empty();
        let b = a.fold(&[1u8; 32]);
        assert_ne!(b.value, a.value);
        assert_eq!(b.count, 1);
    }

    #[test]
    fn fold_twice_chains() {
        let a = Accumulator::empty();
        let b = a.fold(&[1u8; 32]);
        let c = b.fold(&[2u8; 32]);
        assert_ne!(c.value, b.value);
        assert_eq!(c.count, 2);
    }

    #[test]
    fn order_matters_in_substrate() {
        let a = Accumulator::empty().fold(&[1u8; 32]).fold(&[2u8; 32]);
        let b = Accumulator::empty().fold(&[2u8; 32]).fold(&[1u8; 32]);
        // Substrate hash-chain is order-dependent; production
        // commutative scheme would tie. Asserting the substrate's
        // documented behaviour.
        assert_ne!(a.value, b.value);
    }
}
