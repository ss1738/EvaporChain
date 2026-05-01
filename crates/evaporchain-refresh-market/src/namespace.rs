//! `Namespace` — registered state-object scope with declared capacity
//! and live utilisation count.

use serde::{Deserialize, Serialize};

pub type NamespaceId = Vec<u8>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Namespace {
    pub id: NamespaceId,
    /// Declared maximum number of concurrently-active state-object slots.
    /// Set at registration; rotatable via governance, not by users.
    pub capacity: u64,
    /// Currently-active slot count. AMM input.
    pub used: u64,
}

impl Namespace {
    pub fn new(id: NamespaceId, capacity: u64) -> Self {
        Self {
            id,
            capacity,
            used: 0,
        }
    }

    pub fn is_full(&self) -> bool {
        self.used >= self.capacity
    }

    pub fn headroom(&self) -> u64 {
        self.capacity.saturating_sub(self.used)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_namespace_has_zero_used() {
        let n = Namespace::new(vec![1, 2, 3], 100);
        assert_eq!(n.used, 0);
        assert_eq!(n.headroom(), 100);
        assert!(!n.is_full());
    }

    #[test]
    fn full_namespace_has_no_headroom() {
        let mut n = Namespace::new(vec![1], 10);
        n.used = 10;
        assert_eq!(n.headroom(), 0);
        assert!(n.is_full());
    }
}
