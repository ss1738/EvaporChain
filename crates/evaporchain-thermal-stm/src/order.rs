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

    /// T1.20 — identical txs (same id, same energy) compare as
    /// `Equal`. Pins the irreflexive-via-equality contract: the
    /// comparator never returns `Less` or `Greater` for a tx
    /// against itself or an identical clone. Without this, a
    /// future refactor that introduced a tie-break tail would
    /// silently violate strict total-order semantics.
    #[test]
    fn t1_20_identical_txs_compare_equal() {
        let a = tx(0x42, 777);
        let b = a.clone();
        assert_eq!(compare_thermal(&a, &b), Ordering::Equal);
        assert_eq!(compare_thermal(&b, &a), Ordering::Equal);
    }

    /// T1.20 — energy extremes (`u64::MAX` vs `0`) order correctly.
    /// The `b.energy.cmp(&a.energy)` reversal must not under/overflow
    /// at the boundaries; this pins that the strict total order
    /// survives all u64 values, not just small ones.
    #[test]
    fn t1_20_energy_extremes_compare_correctly() {
        let zero = tx(1, 0);
        let max = tx(2, u64::MAX);
        // MAX has higher priority → MAX < zero in commit order.
        assert_eq!(compare_thermal(&max, &zero), Ordering::Less);
        assert_eq!(compare_thermal(&zero, &max), Ordering::Greater);
        // MAX vs MAX with different ids: id breaks the tie.
        let max_id_a = Tx::new(TxId([1; 32]), u64::MAX, vec![], BTreeMap::new());
        let max_id_b = Tx::new(TxId([2; 32]), u64::MAX, vec![], BTreeMap::new());
        assert_eq!(compare_thermal(&max_id_a, &max_id_b), Ordering::Less);
    }

    /// T1.20 — equal-energy tx_id tie-break uses the FULL 32-byte
    /// array lexicographically, not just byte[0]. Existing
    /// `equal_energy_breaks_on_tx_id` only varies byte[0] (via the
    /// `tx(id_byte, ..)` helper). A refactor that mistakenly compared
    /// only the first byte would silently violate strict-total-order
    /// for tx_ids that collide on byte[0] — exactly the case where
    /// id-byte tie-break matters most.
    #[test]
    fn t1_20_tx_id_full_byte_array_participates_in_tie_break() {
        let mut id_a = [0u8; 32];
        let mut id_b = [0u8; 32];
        // byte[0] tied at 0x55; differ at byte[31].
        id_a[0] = 0x55;
        id_b[0] = 0x55;
        id_a[31] = 0x01;
        id_b[31] = 0x02;
        let a = Tx::new(TxId(id_a), 1000, vec![], BTreeMap::new());
        let b = Tx::new(TxId(id_b), 1000, vec![], BTreeMap::new());
        assert_eq!(
            compare_thermal(&a, &b),
            Ordering::Less,
            "byte[31] difference must break ties when byte[0] is equal"
        );
        // Symmetric.
        assert_eq!(compare_thermal(&b, &a), Ordering::Greater);
        // And the unused state-key trail does NOT affect ordering.
        let unused_key = StateKey([0xFFu8; 32]);
        let a_with_reads = Tx::new(TxId(id_a), 1000, vec![unused_key], BTreeMap::new());
        assert_eq!(
            compare_thermal(&a_with_reads, &b),
            Ordering::Less,
            "read_set must not feed into thermal priority"
        );
    }
}
