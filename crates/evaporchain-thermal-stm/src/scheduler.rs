//! `ThermalScheduler` — the optimistic-concurrency commit engine.
//!
//! Algorithm (per batch):
//!
//! 1. Sort the input transactions under [`compare_thermal`].
//! 2. Initialise an empty `committed_writes` set.
//! 3. For each tx in priority order:
//!    a. Compute the set of contended keys: read-set ∩
//!       committed_writes ∪ write-set ∩ committed_writes.
//!    b. If contended is non-empty: **abort** the tx; record
//!       its conflict outcome. (Higher-priority tx already won
//!       any contended key.)
//!    c. Else: **commit** the tx — apply its write-set to state,
//!       and mark the tx's read-set + write-set as committed
//!       (so later txs see the conflict).
//! 4. Return the per-tx outcome list.
//!
//! Validator-determinism: the output is a pure function of the
//! input batch + initial state. Two validators see the same
//! commits / aborts in the same order.

use std::collections::{BTreeMap, BTreeSet};

use thiserror::Error;

use crate::order::compare_thermal;
use crate::tx::{StateKey, StateValue, Tx, TxOutcome};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScheduleError {
    #[error("two transactions share id {0:?} — ids must be unique within a batch")]
    DuplicateTxId(crate::tx::TxId),
}

/// The result of running a batch through the scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOutcome {
    /// Per-tx outcome, in commit-priority order.
    pub outcomes: Vec<TxOutcome>,
    /// The post-batch state — caller installs.
    pub state: BTreeMap<StateKey, StateValue>,
}

#[derive(Debug, Clone, Default)]
pub struct ThermalScheduler;

impl ThermalScheduler {
    pub fn new() -> Self {
        Self
    }

    /// Run a batch of transactions against the given initial
    /// state. Returns the per-tx outcomes + the post-batch state.
    pub fn run(
        &self,
        initial_state: BTreeMap<StateKey, StateValue>,
        batch: Vec<Tx>,
    ) -> Result<CommitOutcome, ScheduleError> {
        // Reject duplicate ids — they'd make conflict resolution
        // ambiguous.
        let mut seen = BTreeSet::new();
        for t in &batch {
            if !seen.insert(t.id) {
                return Err(ScheduleError::DuplicateTxId(t.id));
            }
        }

        // Sort by thermal priority.
        let mut sorted = batch;
        sorted.sort_by(compare_thermal);

        let mut state = initial_state;
        // Keys that have been touched (read OR written) by an
        // already-committed tx in this batch.
        let mut committed_keys: BTreeMap<StateKey, crate::tx::TxId> = BTreeMap::new();

        let mut outcomes = Vec::with_capacity(sorted.len());

        for tx in sorted {
            // Find any conflict against the committed set.
            let mut contended: Vec<StateKey> = Vec::new();
            let mut winner: Option<crate::tx::TxId> = None;

            for k in &tx.read_set {
                if let Some(prev) = committed_keys.get(k) {
                    contended.push(*k);
                    winner.get_or_insert(*prev);
                }
            }
            for k in tx.write_set.keys() {
                if let Some(prev) = committed_keys.get(k) {
                    if !contended.contains(k) {
                        contended.push(*k);
                    }
                    winner.get_or_insert(*prev);
                }
            }

            if !contended.is_empty() {
                outcomes.push(TxOutcome::AbortedConflict {
                    tx_id: tx.id,
                    winner: winner.expect("we found at least one"),
                    contended_keys: contended,
                });
                continue;
            }

            // Commit: apply write-set, mark read-set + write-set
            // as touched.
            for (k, v) in &tx.write_set {
                state.insert(*k, v.clone());
            }
            for k in &tx.read_set {
                committed_keys.insert(*k, tx.id);
            }
            for k in tx.write_set.keys() {
                committed_keys.insert(*k, tx.id);
            }
            outcomes.push(TxOutcome::Committed { tx_id: tx.id });
        }

        Ok(CommitOutcome { outcomes, state })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx::{StateKey, Tx, TxId};
    use std::collections::BTreeMap;

    fn key(b: u8) -> StateKey {
        StateKey([b; 32])
    }
    fn id(b: u8) -> TxId {
        TxId([b; 32])
    }

    fn write_only(id_byte: u8, energy: u64, write_key: StateKey, value: &[u8]) -> Tx {
        let mut ws = BTreeMap::new();
        ws.insert(write_key, value.to_vec());
        Tx::new(id(id_byte), energy, vec![], ws)
    }

    fn read_then_write(
        id_byte: u8,
        energy: u64,
        read_key: StateKey,
        write_key: StateKey,
        value: &[u8],
    ) -> Tx {
        let mut ws = BTreeMap::new();
        ws.insert(write_key, value.to_vec());
        Tx::new(id(id_byte), energy, vec![read_key], ws)
    }

    // ── basic scheduling ──────────────────────────────────────────

    #[test]
    fn empty_batch_returns_empty_outcomes() {
        let s = ThermalScheduler::new();
        let r = s.run(BTreeMap::new(), vec![]).unwrap();
        assert!(r.outcomes.is_empty());
        assert!(r.state.is_empty());
    }

    #[test]
    fn single_tx_commits() {
        let s = ThermalScheduler::new();
        let tx = write_only(1, 1000, key(0), b"hello");
        let r = s.run(BTreeMap::new(), vec![tx.clone()]).unwrap();
        assert_eq!(r.outcomes, vec![TxOutcome::Committed { tx_id: id(1) }]);
        assert_eq!(r.state.get(&key(0)).unwrap(), b"hello");
    }

    #[test]
    fn non_overlapping_txs_all_commit() {
        let s = ThermalScheduler::new();
        let txs = vec![
            write_only(1, 1000, key(0), b"a"),
            write_only(2, 500, key(1), b"b"),
            write_only(3, 100, key(2), b"c"),
        ];
        let r = s.run(BTreeMap::new(), txs).unwrap();
        assert_eq!(r.outcomes.len(), 3);
        assert!(r
            .outcomes
            .iter()
            .all(|o| matches!(o, TxOutcome::Committed { .. })));
        assert_eq!(r.state.get(&key(0)).unwrap(), b"a");
        assert_eq!(r.state.get(&key(1)).unwrap(), b"b");
        assert_eq!(r.state.get(&key(2)).unwrap(), b"c");
    }

    // ── conflict resolution ───────────────────────────────────────

    #[test]
    fn write_write_conflict_higher_energy_wins() {
        // Both want to write key(0). High-energy commits, low-energy
        // aborts.
        let s = ThermalScheduler::new();
        let high = write_only(1, 1000, key(0), b"high");
        let low = write_only(2, 100, key(0), b"low");
        let r = s.run(BTreeMap::new(), vec![low, high]).unwrap();

        // Sorted: high first.
        assert_eq!(r.outcomes[0], TxOutcome::Committed { tx_id: id(1) });
        let TxOutcome::AbortedConflict {
            tx_id,
            winner,
            contended_keys,
        } = &r.outcomes[1]
        else {
            panic!("expected AbortedConflict, got {:?}", r.outcomes[1]);
        };
        assert_eq!(*tx_id, id(2));
        assert_eq!(*winner, id(1));
        assert_eq!(contended_keys, &vec![key(0)]);

        assert_eq!(r.state.get(&key(0)).unwrap(), b"high");
    }

    #[test]
    fn read_write_conflict_aborts_lower_priority_reader() {
        // Tx-high writes key(0). Tx-low reads key(0). The reader
        // can't proceed because its snapshot of key(0) is stale.
        let s = ThermalScheduler::new();
        let high = write_only(1, 1000, key(0), b"new");
        let low = read_then_write(2, 100, key(0), key(1), b"derived");
        let r = s.run(BTreeMap::new(), vec![low, high]).unwrap();
        assert_eq!(r.outcomes[0], TxOutcome::Committed { tx_id: id(1) });
        assert!(matches!(r.outcomes[1], TxOutcome::AbortedConflict { .. }));
    }

    #[test]
    fn submission_order_does_not_change_commits() {
        // The scheduler produces the same outcomes regardless of
        // input order. Validator-determinism.
        let s = ThermalScheduler::new();
        let high = write_only(1, 1000, key(0), b"high");
        let low = write_only(2, 100, key(0), b"low");
        let r1 = s
            .run(BTreeMap::new(), vec![high.clone(), low.clone()])
            .unwrap();
        let r2 = s.run(BTreeMap::new(), vec![low, high]).unwrap();
        assert_eq!(r1, r2);
    }

    // ── equal-energy tie-break ────────────────────────────────────

    #[test]
    fn equal_energy_lower_id_wins() {
        let s = ThermalScheduler::new();
        let a = write_only(1, 1000, key(0), b"A"); // id [1; 32] — lower
        let b = write_only(2, 1000, key(0), b"B"); // id [2; 32] — higher
        let r = s.run(BTreeMap::new(), vec![b, a]).unwrap();
        // A commits (lower id wins on equal energy).
        assert_eq!(r.outcomes[0], TxOutcome::Committed { tx_id: id(1) });
        assert!(matches!(r.outcomes[1], TxOutcome::AbortedConflict { .. }));
        assert_eq!(r.state.get(&key(0)).unwrap(), b"A");
    }

    // ── deadlock impossibility (the press claim) ──────────────────

    #[test]
    fn cyclic_conflict_resolves_without_deadlock() {
        // Tx1 reads key(0), writes key(1). Tx2 reads key(1), writes key(0).
        // Under naive 2PL, this would deadlock. Under thermal, the
        // higher-energy tx commits; the other aborts.
        let s = ThermalScheduler::new();
        let t1 = read_then_write(1, 1000, key(0), key(1), b"v1");
        let t2 = read_then_write(2, 500, key(1), key(0), b"v2");
        let r = s.run(BTreeMap::new(), vec![t1, t2]).unwrap();
        assert_eq!(r.outcomes[0], TxOutcome::Committed { tx_id: id(1) });
        assert!(matches!(r.outcomes[1], TxOutcome::AbortedConflict { .. }));
    }

    // ── validator determinism ─────────────────────────────────────

    #[test]
    fn validator_determinism_same_batch_byte_identical() {
        let s = ThermalScheduler::new();
        let txs = vec![
            write_only(1, 1000, key(0), b"a"),
            write_only(2, 500, key(0), b"b"),
            read_then_write(3, 800, key(0), key(1), b"c"),
            write_only(4, 100, key(2), b"d"),
        ];
        let r1 = s.run(BTreeMap::new(), txs.clone()).unwrap();
        let r2 = s.run(BTreeMap::new(), txs).unwrap();
        assert_eq!(r1, r2);
    }

    // ── duplicate ids rejected ────────────────────────────────────

    #[test]
    fn duplicate_ids_rejected() {
        let s = ThermalScheduler::new();
        let dup_a = write_only(1, 1000, key(0), b"a");
        let dup_b = write_only(1, 500, key(1), b"b");
        let err = s.run(BTreeMap::new(), vec![dup_a, dup_b]).unwrap_err();
        assert_eq!(err, ScheduleError::DuplicateTxId(id(1)));
    }

    // ── atomic commit ─────────────────────────────────────────────

    #[test]
    fn aborted_tx_does_not_partial_write() {
        // Aborted tx had two writes; if state contains either, the
        // commit was non-atomic.
        let s = ThermalScheduler::new();
        let high = write_only(1, 1000, key(0), b"high");
        let mut low_ws = BTreeMap::new();
        low_ws.insert(key(0), b"low0".to_vec()); // conflicts
        low_ws.insert(key(1), b"low1".to_vec()); // would be free
        let low = Tx::new(id(2), 100, vec![], low_ws);
        let r = s.run(BTreeMap::new(), vec![low, high]).unwrap();
        // key(0) is high's, key(1) was NOT written (low aborted).
        assert_eq!(r.state.get(&key(0)).unwrap(), b"high");
        assert!(r.state.get(&key(1)).is_none());
    }

    // ── initial state preserved ───────────────────────────────────

    #[test]
    fn initial_state_persists_when_unchanged() {
        let s = ThermalScheduler::new();
        let mut state = BTreeMap::new();
        state.insert(key(99), b"existing".to_vec());
        let tx = write_only(1, 1000, key(0), b"new");
        let r = s.run(state, vec![tx]).unwrap();
        assert_eq!(r.state.get(&key(99)).unwrap(), b"existing");
        assert_eq!(r.state.get(&key(0)).unwrap(), b"new");
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Thermal STM is the first L1 STM where deadlocks
        // are STRUCTURALLY IMPOSSIBLE — any conflict cycle is
        // broken by the strict total order on (energy, tx_id).
        // Equal-priority transactions can't deadlock either,
        // because the id-bytes tie-breaker is total."
        //
        // Demonstrate: 4 mutually-conflicting transactions with
        // various priorities → exactly the highest-priority one
        // commits, the rest abort. No hangs, no retry storm,
        // no deadlock.
        let s = ThermalScheduler::new();
        let txs = vec![
            write_only(1, 100, key(0), b"a"),
            write_only(2, 1000, key(0), b"b"),
            write_only(3, 500, key(0), b"c"),
            write_only(4, 1000, key(0), b"d"), // ties with id 2
        ];
        let r = s.run(BTreeMap::new(), txs).unwrap();
        // id 2 wins: energy 1000, lower id-bytes than id 4.
        assert_eq!(r.outcomes[0], TxOutcome::Committed { tx_id: id(2) });
        // The remaining three abort, all citing id 2 as the winner.
        for o in &r.outcomes[1..] {
            let TxOutcome::AbortedConflict { winner, .. } = o else {
                panic!("expected abort, got {o:?}");
            };
            assert_eq!(*winner, id(2));
        }
        assert_eq!(r.state.get(&key(0)).unwrap(), b"b");
    }

    proptest::proptest! {
        #[test]
        fn property_submission_order_irrelevant(
            n in 1usize..16usize,
            seed in 0u64..1000u64,
        ) {
            // Build n distinct write-only txs all targeting key(0) with
            // pseudo-random energy. Run forward and run reversed; outcomes
            // must match.
            let s = ThermalScheduler::new();
            let mut txs = Vec::new();
            for i in 0..n {
                let e = (seed.wrapping_mul(31).wrapping_add(i as u64)) % 10_000;
                txs.push(write_only(i as u8 + 1, e, key(0), &[i as u8]));
            }
            let mut rev = txs.clone();
            rev.reverse();
            let r1 = s.run(BTreeMap::new(), txs).unwrap();
            let r2 = s.run(BTreeMap::new(), rev).unwrap();
            proptest::prop_assert_eq!(r1, r2);
        }

        #[test]
        fn property_exactly_one_writer_commits_per_contended_key(
            n in 2usize..16usize,
        ) {
            // n write-only txs all targeting key(0). Exactly one
            // commits; the rest abort.
            let s = ThermalScheduler::new();
            let mut txs = Vec::new();
            for i in 0..n {
                txs.push(write_only(i as u8 + 1, (i as u64 + 1) * 100, key(0), &[i as u8]));
            }
            let r = s.run(BTreeMap::new(), txs).unwrap();
            let committed = r.outcomes.iter().filter(|o| matches!(o, TxOutcome::Committed { .. })).count();
            proptest::prop_assert_eq!(committed, 1);
        }
    }

    /// T1.20 — `CommitOutcome::outcomes` is returned in
    /// commit-priority order (source comment at line 39:
    /// "in commit-priority order"). 3 txs with distinct energies
    /// + disjoint write-sets must appear in descending-energy
    /// order regardless of submission order.
    #[test]
    fn t1_20_outcomes_returned_in_priority_order() {
        let s = ThermalScheduler::new();
        let lo = write_only(1, 100, key(10), b"L");
        let mid = write_only(2, 500, key(11), b"M");
        let hi = write_only(3, 9000, key(12), b"H");
        // Submit lo→mid→hi; scheduler must reorder.
        let r = s.run(BTreeMap::new(), vec![lo, mid, hi]).unwrap();
        assert_eq!(r.outcomes.len(), 3);
        let tx_id = |o: &TxOutcome| match o {
            TxOutcome::Committed { tx_id } => *tx_id,
            TxOutcome::AbortedConflict { tx_id, .. } => *tx_id,
        };
        assert_eq!(tx_id(&r.outcomes[0]), id(3), "highest-energy tx first");
        assert_eq!(tx_id(&r.outcomes[1]), id(2));
        assert_eq!(tx_id(&r.outcomes[2]), id(1));
    }

    /// T1.20 — `outcomes.len()` equals the input batch size, even
    /// when some txs abort. Pin that no tx is silently dropped —
    /// the scheduler accounts for every accepted tx.
    #[test]
    fn t1_20_outcomes_count_equals_batch_size_even_with_aborts() {
        let s = ThermalScheduler::new();
        let mut batch = Vec::new();
        for i in 1..=4u8 {
            batch.push(write_only(i, (i as u64) * 1000, key(0), &[i]));
        }
        let r = s.run(BTreeMap::new(), batch).unwrap();
        assert_eq!(r.outcomes.len(), 4);
        let committed_count = r
            .outcomes
            .iter()
            .filter(|o| matches!(o, TxOutcome::Committed { .. }))
            .count();
        let aborted_count = r
            .outcomes
            .iter()
            .filter(|o| matches!(o, TxOutcome::AbortedConflict { .. }))
            .count();
        assert_eq!(committed_count, 1);
        assert_eq!(aborted_count, 3);
        assert_eq!(committed_count + aborted_count, 4);
    }

    /// T1.20 — `ThermalScheduler::default()` produces an
    /// equivalent scheduler to `new()`. Existing tests all use
    /// `new()`; pin the Default delegation.
    #[test]
    fn t1_20_scheduler_default_matches_new() {
        let s_default: ThermalScheduler = Default::default();
        let s_new = ThermalScheduler::new();
        let tx_a = write_only(1, 1000, key(0), b"A");
        let tx_b = tx_a.clone();
        let r_default = s_default.run(BTreeMap::new(), vec![tx_a]).unwrap();
        let r_new = s_new.run(BTreeMap::new(), vec![tx_b]).unwrap();
        assert_eq!(r_default, r_new);
    }
}
