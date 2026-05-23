//! End-to-end integration tests for evaporchain-antichain-mempool.
//!
//! Non-trivial fixture: a 3-producer concurrent block submission.
//!
//! Three validators (P1, P2, P3) each extend the genesis block at the
//! same epoch without seeing each other's blocks. Their outputs arrive
//! at the proposer as three concurrent tips on the LightCone DAG.
//! A fourth validator (P4) submits a low-energy fork that competes.
//! P1 then extends its own tip, creating A' as a child of A.
//!
//!   Genesis (id=0)  energy=500_000   epoch=0
//!     ├── Fork A  (id=1)  energy=800_000   epoch=1  [P1]
//!     ├── Fork B  (id=2)  energy=700_000   epoch=1  [P2]
//!     ├── Fork C  (id=3)  energy=600_000   epoch=1  [P3]
//!     └── Fork D  (id=4)  energy=200_000   epoch=1  [P4, low-energy]
//!
//!   Fork A' (id=5)  energy=900_000   epoch=2  [P1 extends own tip]
//!     └── parent: A (id=1)
//!
//! Concurrent pairs on this DAG:
//!   {A, B}, {A, C}, {A, D}, {B, C}, {B, D}, {C, D}  — all concurrent (siblings)
//!   {A, B, C, D}  — maximal antichain of genesis children
//!
//! Non-concurrent pairs:
//!   {Genesis, A}  — Genesis < A (parent/child)
//!   {A, A'}       — A < A' (parent/child)
//!   {Genesis, A'} — Genesis < A' (ancestor/descendant)
//!
//! Proposer scenario:
//!   Gate threshold = 2_000_000.
//!   {A, B, C} total at epoch 1 = 2_100_000 → PASS.
//!   {B, D}    total at epoch 1 =   900_000 → FAIL.
//!   {A, B, C} total at epoch 101 ≈ half-decayed → FAIL at 2_000_000.
//!
//! Doctrine claim (INVENTION_STACK §4.1 row 2):
//!   "Mempool IS the partial order. Producer extends maximal antichains
//!   whose total energy clears a threshold. No causal ordering between
//!   mempool members is imposed."

use std::collections::BTreeSet;

use evaporchain_antichain_mempool::{
    extend_to_maximal, is_maximal_antichain, total_energy_meets_threshold,
    Antichain, AntichainError,
};
use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_light_cone::{Block, BlockId, LightCone};

// ── Fixture ──────────────────────────────────────────────────────────────────

fn id(b: u8) -> BlockId { [b; 32] }

fn cl() -> ChainLambda { ChainLambda::new(Lambda::from_epochs(100)) }

const THRESHOLD: u64 = 2_000_000;

/// The 3-producer DAG. Five blocks total (genesis + A + B + C + D + A').
fn three_producer_dag() -> LightCone {
    let mut lc = LightCone::new();
    // Genesis
    lc.insert(Block::new(id(0), vec![],        500_000, 0)).unwrap();
    // Concurrent tips from P1, P2, P3, P4 — all children of genesis only.
    lc.insert(Block::new(id(1), vec![id(0)],   800_000, 1)).unwrap(); // Fork A  (P1)
    lc.insert(Block::new(id(2), vec![id(0)],   700_000, 1)).unwrap(); // Fork B  (P2)
    lc.insert(Block::new(id(3), vec![id(0)],   600_000, 1)).unwrap(); // Fork C  (P3)
    lc.insert(Block::new(id(4), vec![id(0)],   200_000, 1)).unwrap(); // Fork D  (P4)
    // P1 extends its own tip.
    lc.insert(Block::new(id(5), vec![id(1)],   900_000, 2)).unwrap(); // Fork A'
    lc
}

// ── Antichain construction ────────────────────────────────────────────────────

#[test]
fn three_concurrent_tips_form_valid_antichain() {
    let lc = three_producer_dag();
    let abc: BTreeSet<BlockId> = [id(1), id(2), id(3)].into_iter().collect();
    let a = Antichain::from_set(abc, &lc).unwrap();
    assert_eq!(a.len(), 3);
    assert!(a.contains(&id(1)));
    assert!(a.contains(&id(2)));
    assert!(a.contains(&id(3)));
}

#[test]
fn all_four_concurrent_tips_form_valid_antichain() {
    let lc = three_producer_dag();
    let abcd: BTreeSet<BlockId> = [id(1), id(2), id(3), id(4)].into_iter().collect();
    let a = Antichain::from_set(abcd, &lc).unwrap();
    assert_eq!(a.len(), 4);
}

// ── Maximality ────────────────────────────────────────────────────────────────

#[test]
fn abcd_is_maximal_antichain_of_the_dag() {
    let lc = three_producer_dag();
    // {A, B, C, D} are all siblings — no other block can be added without
    // introducing a comparable pair (genesis precedes all; A' is A's child).
    let abcd: BTreeSet<BlockId> = [id(1), id(2), id(3), id(4)].into_iter().collect();
    let a = Antichain::from_set(abcd, &lc).unwrap();
    assert!(is_maximal_antichain(&a, &lc),
        "ABCD must be a maximal antichain — no concurrent outsider exists");
}

#[test]
fn abc_not_maximal_while_d_is_concurrent() {
    let lc = three_producer_dag();
    let abc: BTreeSet<BlockId> = [id(1), id(2), id(3)].into_iter().collect();
    let a = Antichain::from_set(abc, &lc).unwrap();
    assert!(!is_maximal_antichain(&a, &lc),
        "ABC is not maximal — D is concurrent with all three and can be added");
}

#[test]
fn extend_from_seed_a_reaches_maximal() {
    let lc = three_producer_dag();
    let seed = Antichain::from_set([id(1)].into_iter().collect::<BTreeSet<_>>(), &lc).unwrap();
    // Propose candidates in descending energy order (A', B, C, D, genesis).
    let candidates = vec![id(5), id(2), id(3), id(4), id(0)];
    let maximal = extend_to_maximal(&seed, &lc, candidates).unwrap();
    // A' is A's child (not concurrent with A) → excluded.
    // B, C, D are concurrent with A → included.
    // Genesis precedes A → excluded.
    assert!(is_maximal_antichain(&maximal, &lc));
    assert!(maximal.contains(&id(2)), "B must be in the maximal extension");
    assert!(maximal.contains(&id(3)), "C must be in the maximal extension");
    assert!(maximal.contains(&id(4)), "D must be in the maximal extension");
    assert!(!maximal.contains(&id(5)), "A' is A's child — must not be added");
    assert!(!maximal.contains(&id(0)), "Genesis precedes A — must not be added");
}

// ── Energy-threshold gate ─────────────────────────────────────────────────────

#[test]
fn abc_clears_threshold_at_epoch_1() {
    let lc = three_producer_dag();
    let abc: BTreeSet<BlockId> = [id(1), id(2), id(3)].into_iter().collect();
    let a = Antichain::from_set(abc, &lc).unwrap();
    // Total at epoch 1 (elapsed=0): 800_000 + 700_000 + 600_000 = 2_100_000
    assert!(total_energy_meets_threshold(&a, &lc, cl(), 1, THRESHOLD),
        "ABC total 2_100_000 must clear threshold {THRESHOLD}");
    assert!(!total_energy_meets_threshold(&a, &lc, cl(), 1, 2_200_000),
        "ABC total 2_100_000 must NOT clear threshold 2_200_000");
}

#[test]
fn low_energy_pair_bd_fails_threshold() {
    let lc = three_producer_dag();
    let bd: BTreeSet<BlockId> = [id(2), id(4)].into_iter().collect();
    let a = Antichain::from_set(bd, &lc).unwrap();
    // Total at epoch 1: 700_000 + 200_000 = 900_000
    assert!(!total_energy_meets_threshold(&a, &lc, cl(), 1, THRESHOLD),
        "BD total 900_000 must not clear threshold {THRESHOLD}");
}

#[test]
fn abc_decay_drops_below_threshold_after_one_half_life() {
    let lc = three_producer_dag();
    let abc: BTreeSet<BlockId> = [id(1), id(2), id(3)].into_iter().collect();
    let a = Antichain::from_set(abc, &lc).unwrap();
    // After 100 epochs of decay (half-life=100) from observed_epoch=1:
    // each energy halves. New total ≈ 400_000 + 350_000 + 300_000 = 1_050_000.
    assert!(!total_energy_meets_threshold(&a, &lc, cl(), 101, THRESHOLD),
        "ABC after one half-life of decay must not clear threshold {THRESHOLD}");
    // But it still clears a lower threshold (1_000_000).
    assert!(total_energy_meets_threshold(&a, &lc, cl(), 101, 1_000_000),
        "ABC after one half-life must still clear a lower threshold 1_000_000");
}

// ── Proposer full-flow decision ───────────────────────────────────────────────

#[test]
fn proposer_admits_maximal_concurrent_tips_when_energy_clears_gate() {
    // Full flow: build maximal antichain → energy gate → accept/reject.
    let lc = three_producer_dag();
    let seed = Antichain::from_set([id(1)].into_iter().collect::<BTreeSet<_>>(), &lc).unwrap();
    // Extend greedily by descending energy (A', B, C, D, genesis).
    let candidates = vec![id(5), id(2), id(3), id(4), id(0)];
    let proposal = extend_to_maximal(&seed, &lc, candidates).unwrap();
    assert!(is_maximal_antichain(&proposal, &lc));

    // Gate check at epoch 1 with the chain threshold.
    let gate_pass = total_energy_meets_threshold(&proposal, &lc, cl(), 1, THRESHOLD);
    assert!(gate_pass,
        "maximal antichain {{A,B,C,D}} at epoch 1 must pass the proposer energy gate");
}

// ── Adversarial checks ───────────────────────────────────────────────────────

#[test]
fn adversarial_comparable_pair_genesis_and_child_rejected() {
    let lc = three_producer_dag();
    let err = Antichain::from_set(
        [id(0), id(1)].into_iter().collect::<BTreeSet<_>>(), &lc,
    ).unwrap_err();
    assert!(matches!(err, AntichainError::Comparable { .. }),
        "genesis < A is comparable — must be rejected from antichain");
}

#[test]
fn adversarial_comparable_pair_parent_and_grandchild_rejected() {
    let lc = three_producer_dag();
    // Genesis < A' (ancestor/descendant).
    let err = Antichain::from_set(
        [id(0), id(5)].into_iter().collect::<BTreeSet<_>>(), &lc,
    ).unwrap_err();
    assert!(matches!(err, AntichainError::Comparable { .. }),
        "genesis < A' (ancestor) is comparable — must be rejected");
}

#[test]
fn adversarial_absent_block_rejected() {
    let lc = three_producer_dag();
    let err = Antichain::from_set(
        [id(99)].into_iter().collect::<BTreeSet<_>>(), &lc,
    ).unwrap_err();
    assert!(matches!(err, AntichainError::Absent(_)),
        "a block not present in the LightCone must be rejected");
}

#[test]
fn adversarial_a_and_a_prime_rejected_parent_child() {
    let lc = three_producer_dag();
    // A (id=1) is parent of A' (id=5).
    let err = Antichain::from_set(
        [id(1), id(5)].into_iter().collect::<BTreeSet<_>>(), &lc,
    ).unwrap_err();
    assert!(matches!(err, AntichainError::Comparable { .. }),
        "A < A' (parent/child) must be rejected from antichain");
}

#[test]
fn empty_antichain_never_meets_positive_threshold() {
    let lc = three_producer_dag();
    let empty = Antichain::empty();
    assert!(!total_energy_meets_threshold(&empty, &lc, cl(), 0, 1),
        "empty antichain must fail any positive threshold");
    assert!(total_energy_meets_threshold(&empty, &lc, cl(), 0, 0),
        "empty antichain meets a zero threshold");
}
