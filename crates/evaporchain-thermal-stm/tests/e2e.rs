//! End-to-end integration tests for evaporchain-thermal-stm.
//!
//! Non-trivial fixture: "5-tx bank transfer batch".
//!
//!   State keys: A=[1;32], B=[2;32], C=[3;32], D=[4;32], E=[5;32].
//!   Initial state: empty (all keys absent).
//!
//!   Batch (submitted in reverse-priority order to confirm sorting):
//!     Tx1 (energy=10_000): reads A, writes A=b"50"  + B=b"150"
//!     Tx2 (energy= 8_000): reads A, writes A=b"30"  + C=b"70"  -- conflicts Tx1 on A
//!     Tx3 (energy= 6_000): reads B, writes B=b"90"  + D=b"60"  -- conflicts Tx1 on B
//!     Tx4 (energy= 5_000): reads C, writes C=b"20"  + D=b"80"  -- C/D untouched if Tx2,Tx3 aborted
//!     Tx5 (energy= 3_000): no reads,  writes E=b"ping"         -- independent
//!
//!   Priority sort: Tx1 > Tx2 > Tx3 > Tx4 > Tx5.
//!
//!   Execution trace:
//!     Tx1: no conflict -> COMMIT. committed_keys = {A, B}.
//!     Tx2: reads A in committed_keys -> ABORT (winner=Tx1, contended=[A]).
//!     Tx3: reads B in committed_keys -> ABORT (winner=Tx1, contended=[B]).
//!     Tx4: reads C (not touched) -> COMMIT. committed_keys += {C, D}.
//!     Tx5: writes E (not touched) -> COMMIT. committed_keys += {E}.
//!
//!   Final state: A=b"50", B=b"150", C=b"20", D=b"80", E=b"ping".
//!   Tx2 aborted: A is NOT overwritten to b"30"; C is absent (Tx2 aborted fully).
//!   Tx3 aborted: B is NOT overwritten to b"90"; D from Tx3 absent.
//!   Tx4 wins D because Tx3 aborted; D=b"80" from Tx4.
//!
//!   Deadlock impossibility: the cyclic conflict pair Tx1 (reads A,
//!   writes B) + hypothetical Tx_rev (reads B, writes A) would
//!   deadlock under 2PL. Thermal STM breaks it: Tx1 (higher energy)
//!   commits; Tx_rev aborts. No hang, no retry storm.
//!
//! Doctrine claim (INVENTION_STACK.md §4.3 / Thermal STM):
//! "Thermal STM is the first L1 STM where deadlocks are STRUCTURALLY
//!  IMPOSSIBLE. Conflicts resolve by the strict total order
//!  (energy desc, tx_id asc); two validators receiving the same batch
//!  in any order produce byte-identical commits. Aborted transactions
//!  have zero partial-write effect on state."
//!
//! Adversarial fixture: DuplicateTxId, partial-write isolation,
//! cyclic-conflict resolution.
//!
//! INVENTION_STACK §4.3: Thermal STM (Decay-Aware Concurrent VM).

use evaporchain_thermal_stm::{
    CommitOutcome, ScheduleError, StateKey, ThermalScheduler, Tx, TxId, TxOutcome,
};
use std::collections::BTreeMap;

// -- Helpers ------------------------------------------------------------------

fn key(b: u8) -> StateKey {
    StateKey([b; 32])
}

fn tid(b: u8) -> TxId {
    TxId([b; 32])
}

fn write_tx(id: u8, energy: u64, reads: Vec<StateKey>, writes: Vec<(StateKey, &[u8])>) -> Tx {
    let mut ws = BTreeMap::new();
    for (k, v) in writes {
        ws.insert(k, v.to_vec());
    }
    Tx::new(tid(id), energy, reads, ws)
}

// -- Non-trivial fixture ------------------------------------------------------

#[test]
fn fixture_five_tx_bank_transfer_batch() {
    let s = ThermalScheduler::new();

    // Keys: A=key(1), B=key(2), C=key(3), D=key(4), E=key(5).
    let tx1 = write_tx(1, 10_000, vec![key(1)],         vec![(key(1), b"50"),  (key(2), b"150")]);
    let tx2 = write_tx(2,  8_000, vec![key(1)],         vec![(key(1), b"30"),  (key(3), b"70")]);
    let tx3 = write_tx(3,  6_000, vec![key(2)],         vec![(key(2), b"90"),  (key(4), b"60")]);
    let tx4 = write_tx(4,  5_000, vec![key(3)],         vec![(key(3), b"20"),  (key(4), b"80")]);
    let tx5 = write_tx(5,  3_000, vec![],               vec![(key(5), b"ping")]);

    // Submit in reverse priority order to confirm sorting.
    let batch = vec![tx5.clone(), tx4.clone(), tx3.clone(), tx2.clone(), tx1.clone()];
    let CommitOutcome { outcomes, state } = s.run(BTreeMap::new(), batch).unwrap();

    // All 5 transactions accounted for.
    assert_eq!(outcomes.len(), 5);

    // Outcomes are in priority order: Tx1(10K), Tx2(8K), Tx3(6K), Tx4(5K), Tx5(3K).
    assert_eq!(outcomes[0], TxOutcome::Committed { tx_id: tid(1) }, "Tx1 must commit");

    // Tx2 aborts: reads A which Tx1 wrote.
    if let TxOutcome::AbortedConflict { ref winner, ref contended_keys, tx_id } = outcomes[1] {
        assert_eq!(tx_id, tid(2), "outcomes[1] must be Tx2");
        assert_eq!(*winner, tid(1), "Tx2's winner must be Tx1");
        assert!(contended_keys.contains(&key(1)), "contended key must be A");
    } else {
        panic!("Tx2 must abort; got {:?}", outcomes[1]);
    }

    // Tx3 aborts: reads B which Tx1 wrote.
    if let TxOutcome::AbortedConflict { ref winner, ref contended_keys, tx_id } = outcomes[2] {
        assert_eq!(tx_id, tid(3), "outcomes[2] must be Tx3");
        assert_eq!(*winner, tid(1), "Tx3's winner must be Tx1");
        assert!(contended_keys.contains(&key(2)), "contended key must be B");
    } else {
        panic!("Tx3 must abort; got {:?}", outcomes[2]);
    }

    // Tx4 and Tx5 commit.
    assert_eq!(outcomes[3], TxOutcome::Committed { tx_id: tid(4) }, "Tx4 must commit");
    assert_eq!(outcomes[4], TxOutcome::Committed { tx_id: tid(5) }, "Tx5 must commit");

    // Final state assertions.
    assert_eq!(state.get(&key(1)).unwrap(), b"50",   "A from Tx1 (Tx2 aborted)");
    assert_eq!(state.get(&key(2)).unwrap(), b"150",  "B from Tx1 (Tx3 aborted)");
    assert_eq!(state.get(&key(3)).unwrap(), b"20",   "C from Tx4 (Tx2 aborted, C was free)");
    assert_eq!(state.get(&key(4)).unwrap(), b"80",   "D from Tx4 (Tx3 aborted, D was free)");
    assert_eq!(state.get(&key(5)).unwrap(), b"ping", "E from Tx5 (independent)");
}

#[test]
fn fixture_submission_order_identical_to_reversed() {
    // Validator-determinism: same batch in any order -> byte-identical output.
    let s = ThermalScheduler::new();

    let tx1 = write_tx(1, 10_000, vec![key(1)], vec![(key(1), b"50"), (key(2), b"150")]);
    let tx2 = write_tx(2,  8_000, vec![key(1)], vec![(key(1), b"30"), (key(3), b"70")]);
    let tx3 = write_tx(3,  6_000, vec![key(2)], vec![(key(2), b"90"), (key(4), b"60")]);
    let tx4 = write_tx(4,  5_000, vec![key(3)], vec![(key(3), b"20"), (key(4), b"80")]);
    let tx5 = write_tx(5,  3_000, vec![],       vec![(key(5), b"ping")]);

    let fwd = s.run(BTreeMap::new(), vec![tx1.clone(), tx2.clone(), tx3.clone(), tx4.clone(), tx5.clone()]).unwrap();
    let rev = s.run(BTreeMap::new(), vec![tx5, tx4, tx3, tx2, tx1]).unwrap();

    assert_eq!(fwd, rev, "batch outcome must be submission-order-independent");
}

// -- Doctrine tests -----------------------------------------------------------

#[test]
fn doctrine_cyclic_conflict_no_deadlock() {
    // Tx_a reads key(1), writes key(2).
    // Tx_b reads key(2), writes key(1).
    // Under 2PL this is a deadlock. Thermal STM: higher-energy commits.
    let s = ThermalScheduler::new();

    let tx_a = write_tx(1, 1_000, vec![key(1)], vec![(key(2), b"a")]);
    let tx_b = write_tx(2,   500, vec![key(2)], vec![(key(1), b"b")]);

    let result = s.run(BTreeMap::new(), vec![tx_a, tx_b]).unwrap();
    assert_eq!(result.outcomes.len(), 2);

    // Tx_a (higher energy) commits; Tx_b aborts. No hang.
    assert!(matches!(result.outcomes[0], TxOutcome::Committed { tx_id } if tx_id == tid(1)));
    assert!(matches!(result.outcomes[1], TxOutcome::AbortedConflict { tx_id, .. } if tx_id == tid(2)));
}

#[test]
fn doctrine_aborted_tx_has_zero_state_effect() {
    // Atomic commit: aborted tx leaves NO partial writes.
    let s = ThermalScheduler::new();

    let winner = write_tx(1, 1_000, vec![],       vec![(key(0), b"win")]);
    // Loser wants to write key(0) (conflict) AND key(99) (would be free).
    let loser  = write_tx(2,   100, vec![key(0)], vec![(key(0), b"lose"), (key(99), b"side")]);

    let result = s.run(BTreeMap::new(), vec![winner, loser]).unwrap();
    assert_eq!(result.state.get(&key(0)).unwrap(), b"win");
    assert!(!result.state.contains_key(&key(99)), "aborted tx must not partially write key(99)");
}

#[test]
fn doctrine_non_overlapping_txs_all_commit() {
    // Non-overlapping write-sets: every tx commits regardless of energy order.
    let s = ThermalScheduler::new();

    let txs = vec![
        write_tx(1, 100, vec![], vec![(key(10), b"a")]),
        write_tx(2, 500, vec![], vec![(key(11), b"b")]),
        write_tx(3, 900, vec![], vec![(key(12), b"c")]),
        write_tx(4,  50, vec![], vec![(key(13), b"d")]),
    ];

    let result = s.run(BTreeMap::new(), txs).unwrap();
    let committed = result.outcomes.iter().filter(|o| matches!(o, TxOutcome::Committed { .. })).count();
    assert_eq!(committed, 4, "all 4 non-overlapping txs must commit");
}

#[test]
fn doctrine_equal_energy_tie_break_is_deterministic() {
    // Equal energy: lower tx_id wins. Total order -- no ties.
    let s = ThermalScheduler::new();

    let lo = write_tx(1, 1_000, vec![], vec![(key(0), b"lo")]); // id=[1;32] -- lower
    let hi = write_tx(2, 1_000, vec![], vec![(key(0), b"hi")]); // id=[2;32] -- higher

    let result = s.run(BTreeMap::new(), vec![hi, lo]).unwrap();
    // lo commits (lower id), hi aborts.
    assert!(matches!(result.outcomes[0], TxOutcome::Committed { tx_id } if tx_id == tid(1)));
    assert_eq!(result.state.get(&key(0)).unwrap(), b"lo");
}

// -- Adversarial fixture ------------------------------------------------------

#[test]
fn adversarial_duplicate_tx_id_rejected() {
    let s = ThermalScheduler::new();
    let dup_a = write_tx(1, 1_000, vec![], vec![(key(0), b"a")]);
    let dup_b = write_tx(1,   500, vec![], vec![(key(1), b"b")]);
    let err = s.run(BTreeMap::new(), vec![dup_a, dup_b]).unwrap_err();
    assert_eq!(err, ScheduleError::DuplicateTxId(tid(1)));
}

#[test]
fn adversarial_high_fan_out_conflict_one_commits() {
    // 10 txs all want key(0). Exactly one commits; 9 abort.
    let s = ThermalScheduler::new();

    let txs: Vec<_> = (1..=10u8)
        .map(|i| write_tx(i, (i as u64) * 100, vec![], vec![(key(0), &[i])]))
        .collect();
    let result = s.run(BTreeMap::new(), txs).unwrap();

    let committed = result.outcomes.iter().filter(|o| matches!(o, TxOutcome::Committed { .. })).count();
    let aborted   = result.outcomes.iter().filter(|o| matches!(o, TxOutcome::AbortedConflict { .. })).count();
    assert_eq!(committed, 1, "exactly one tx commits on a fully-contended key");
    assert_eq!(aborted, 9);

    // The committed tx is the highest energy one (id=10, energy=1000).
    assert!(matches!(result.outcomes[0], TxOutcome::Committed { tx_id } if tx_id == tid(10)));
    assert_eq!(result.state.get(&key(0)).unwrap(), &[10u8]);
}

// -- Cross-cuts ---------------------------------------------------------------

#[test]
fn outcomes_len_equals_batch_size() {
    // Pin: no transaction is silently dropped -- all are accounted for.
    let s = ThermalScheduler::new();
    let txs: Vec<_> = (1..=6u8)
        .map(|i| write_tx(i, (i as u64) * 200, vec![], vec![(key(i), &[i])]))
        .collect();
    let result = s.run(BTreeMap::new(), txs).unwrap();
    assert_eq!(result.outcomes.len(), 6);
    // All non-overlapping: all commit.
    let committed = result.outcomes.iter().filter(|o| matches!(o, TxOutcome::Committed { .. })).count();
    assert_eq!(committed, 6);
}

#[test]
fn initial_state_preserved_through_batch() {
    let s = ThermalScheduler::new();
    let mut initial = BTreeMap::new();
    initial.insert(key(99), b"existing".to_vec());

    let tx = write_tx(1, 1_000, vec![], vec![(key(0), b"new")]);
    let result = s.run(initial, vec![tx]).unwrap();

    assert_eq!(result.state.get(&key(99)).unwrap(), b"existing");
    assert_eq!(result.state.get(&key(0)).unwrap(),  b"new");
}
