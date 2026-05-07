//! Thermal priority ordering — the strict total order on
//! transactions used to break conflicts.
//!
//! Sort key: `(−energy, tx_id_bytes)` lexicographic. Higher
//! energy wins; equal-energy → lower tx_id bytes win. Pure
//! integer comparison; validator-deterministic.
//!
//! ## Why a strict TOTAL order matters
//!
//! In the conflict graph between contending transactions, a
//! cycle would deadlock under any "highest-energy first" rule
//! that isn't also a total order. The id-bytes tie-breaker
//! ensures the order is total even when many txs share the
//! same energy: the comparator is irreflexive, antisymmetric,
//! and transitive.

use std::cmp::Ordering;

use crate::tx::Tx;

/// Compare two transactions under thermal priority. Returns
/// `Ordering::Less` if `a` should commit BEFORE `b` (i.e., `a`
/// has higher priority).
pub fn compare_thermal(a: &Tx, b: &Tx) -> Ordering {
    // Higher energy first → reverse the natural u64 order.
    match b.energy.cmp(&a.energy) {
        Ordering::Equal => a.id.0.cmp(&b.id.0),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx::{StateKey, Tx, TxId};
    use std::collections::BTreeMap;

    fn tx(id_byte: u8, energy: u64) -> Tx {
        Tx::new(TxId([id_byte; 32]), energy, vec![], BTreeMap::new())
    }

    #[test]
    fn higher_energy_wins() {
        let a = tx(1, 1000);
        let b = tx(2, 500);
        assert_eq!(compare_thermal(&a, &b), Ordering::Less);
        assert_eq!(compare_thermal(&b, &a), Ordering::Greater);
    }

    #[test]
    fn equal_energy_breaks_on_tx_id() {
        let a = tx(1, 1000);
        let b = tx(2, 1000);
        // Lower id-bytes wins.
        assert_eq!(compare_thermal(&a, &b), Ordering::Less);
    }

    #[test]
    fn reflexive_returns_equal() {
        let a = tx(1, 1000);
        assert_eq!(compare_thermal(&a, &a), Ordering::Equal);
    }

    #[test]
    fn order_is_antisymmetric() {
        let a = tx(1, 1000);
        let b = tx(2, 500);
        assert_eq!(compare_thermal(&a, &b), Ordering::Less);
        assert_eq!(compare_thermal(&b, &a), Ordering::Greater);
    }

    #[test]
    fn order_is_transitive() {
        let a = tx(1, 1000);
        let b = tx(2, 500);
        let c = tx(3, 100);
        // a < b, b < c, therefore a < c.
        assert_eq!(compare_thermal(&a, &b), Ordering::Less);
        assert_eq!(compare_thermal(&b, &c), Ordering::Less);
        assert_eq!(compare_thermal(&a, &c), Ordering::Less);
    }

    #[test]
    fn sort_is_validator_deterministic() {
        // Sort the same set in different submission orders;
        // result must match.
        let mut a = vec![tx(5, 100), tx(2, 500), tx(8, 1000), tx(1, 100), tx(7, 500)];
        let mut b = a.clone();
        b.reverse();
        a.sort_by(compare_thermal);
        b.sort_by(compare_thermal);
        let a_ids: Vec<_> = a.iter().map(|t| t.id.0[0]).collect();
        let b_ids: Vec<_> = b.iter().map(|t| t.id.0[0]).collect();
        assert_eq!(a_ids, b_ids);
        // High-energy first: 8 (1000), then 2/7 (500, lower id first), then 1/5 (100).
        assert_eq!(a_ids, vec![8, 2, 7, 1, 5]);
    }
}
