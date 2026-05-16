//! End-to-end integration tests for evaporchain-light-cone.
//!
//! Non-trivial fixture: network-partition fork-and-merge DAG.
//!
//! During a network partition two validator groups produce concurrent
//! block chains. When the network heals a merge block joins them.
//!
//!   G  (genesis,        epoch=0, energy=1_000)
//!   ├── A1 (group-1,   epoch=1, energy=  900)   ┐ concurrent
//!   │   └── A2         epoch=2, energy=  800)   │ with all
//!   │       └── A3     epoch=3, energy=  700)   │ B-chain
//!   └── B1 (group-2,   epoch=1, energy=  900)   │ blocks
//!       └── B2         epoch=2, energy=  800)   │
//!           └── B3     epoch=3, energy=  700)   ┘
//!               (both A3 and B3 are leaves before merge)
//!
//!   M  (merge block,   epoch=4, energy=  600, parents={A3, B3})
//!   └── D  (descendant, epoch=5, energy=  500)
//!
//! After the merge:
//!   causal_past(M)  = {G, A1, A2, A3, B1, B2, B3}   (7 blocks)
//!   causal_future(G) = {A1, A2, A3, B1, B2, B3, M, D} (8 blocks)
//!   Antichain {A3, B3} = frontier before merge (concurrent leaves)
//!   Time arrow: G has more remaining energy at epoch=5 than M or D
//!
//! Doctrine claim (INVENTION_STACK §4.1 #1): "Light-Cone Consensus:
//! causal-set partial-order (Sorkin/Pratt). Energy decay gives the
//! time arrow. Two concurrent blocks have no path between them;
//! causal past is transitively closed; precedes is anti-symmetric."
//!
//! Adversarial fixture: missing parent, duplicate insert, parent-count
//! overflow, antichain rejects comparable pairs, time arrow requires
//! common observation time.
//!
//! INVENTION_STACK §4.1 #1: Light-Cone Consensus.

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_light_cone::{
    causal_future, causal_past, closing_antichain, comparable, is_antichain, is_concurrent,
    precedes, time_arrow_holds_at, Block, LightCone, LightConeError,
};
use evaporchain_light_cone::concurrency::MAX_ANTICHAIN_INPUT;
use evaporchain_light_cone::dag::MAX_PARENTS_PER_BLOCK;

// ── Helpers ───────────────────────────────────────────────────────────────

fn id(b: u8) -> [u8; 32] {
    let mut x = [0u8; 32];
    x[31] = b;
    x
}

fn lambda() -> ChainLambda {
    ChainLambda::new(Lambda::from_epochs(100))
}

// ── Build the partition DAG ───────────────────────────────────────────────

fn build_partition_dag() -> LightCone {
    let mut lc = LightCone::new();

    // Genesis.
    lc.insert(Block::new(id(0x00), vec![],             1_000, 0)).unwrap();
    // Group-1 chain.
    lc.insert(Block::new(id(0xA1), vec![id(0x00)],       900, 1)).unwrap();
    lc.insert(Block::new(id(0xA2), vec![id(0xA1)],       800, 2)).unwrap();
    lc.insert(Block::new(id(0xA3), vec![id(0xA2)],       700, 3)).unwrap();
    // Group-2 chain (concurrent with A-chain).
    lc.insert(Block::new(id(0xB1), vec![id(0x00)],       900, 1)).unwrap();
    lc.insert(Block::new(id(0xB2), vec![id(0xB1)],       800, 2)).unwrap();
    lc.insert(Block::new(id(0xB3), vec![id(0xB2)],       700, 3)).unwrap();
    // Merge block — parents are both concurrent leaves.
    lc.insert(Block::new(id(0xCC), vec![id(0xA3), id(0xB3)], 600, 4)).unwrap();
    // Descendant of merge.
    lc.insert(Block::new(id(0xDD), vec![id(0xCC)],       500, 5)).unwrap();

    lc
}

// ── Non-trivial fixture: network-partition fork-and-merge DAG ─────────────

#[test]
fn dag_contains_all_nine_blocks() {
    let lc = build_partition_dag();
    assert_eq!(lc.len(), 9);
}

#[test]
fn merge_block_causal_past_is_full_history() {
    // causal_past(M) must include every block in both chains plus genesis.
    let lc = build_partition_dag();
    let past = causal_past(&lc, id(0xCC));

    for &expected in &[id(0x00), id(0xA1), id(0xA2), id(0xA3), id(0xB1), id(0xB2), id(0xB3)] {
        assert!(past.contains(&expected),
            "causal_past(M) must include block {:?}", expected);
    }
    assert_eq!(past.len(), 7,
        "causal_past(M) must contain exactly 7 ancestors");
}

#[test]
fn genesis_causal_future_is_all_descendants() {
    // causal_future(G) = every block in the DAG except G itself.
    let lc = build_partition_dag();
    let future = causal_future(&lc, id(0x00));

    for &expected in &[id(0xA1), id(0xA2), id(0xA3), id(0xB1), id(0xB2), id(0xB3), id(0xCC), id(0xDD)] {
        assert!(future.contains(&expected),
            "causal_future(G) must include block {:?}", expected);
    }
    assert_eq!(future.len(), 8,
        "causal_future(G) must contain all 8 non-genesis blocks");
}

#[test]
fn a_chain_and_b_chain_blocks_are_concurrent() {
    // All A-chain × B-chain pairs must be concurrent (no path either way).
    let lc = build_partition_dag();
    let a_chain = [id(0xA1), id(0xA2), id(0xA3)];
    let b_chain = [id(0xB1), id(0xB2), id(0xB3)];

    for &a in &a_chain {
        for &b in &b_chain {
            assert!(
                is_concurrent(&lc, a, b),
                "A-chain block {a:?} and B-chain block {b:?} must be concurrent"
            );
            assert!(
                !comparable(&lc, a, b),
                "concurrent blocks must not be comparable"
            );
        }
    }
}

#[test]
fn precedes_is_anti_symmetric_no_cycles() {
    // For every (ancestor, descendant) pair: ancestor precedes descendant,
    // but descendant does NOT precede ancestor.
    let lc = build_partition_dag();
    let ancestor_descendant_pairs: &[([u8; 32], [u8; 32])] = &[
        (id(0x00), id(0xA3)),
        (id(0x00), id(0xCC)),
        (id(0x00), id(0xDD)),
        (id(0xA1), id(0xA3)),
        (id(0xA1), id(0xCC)),
        (id(0xB2), id(0xCC)),
        (id(0xCC), id(0xDD)),
    ];
    for &(anc, desc) in ancestor_descendant_pairs {
        assert!(precedes(&lc, anc, desc),
            "{anc:?} must precede {desc:?}");
        assert!(!precedes(&lc, desc, anc),
            "{desc:?} must NOT precede {anc:?} — no cycles");
    }
}

#[test]
fn frontier_before_merge_is_valid_antichain() {
    // {A3, B3} is the natural frontier just before the merge: both are
    // concurrent leaves. The antichain test must pass.
    let lc = build_partition_dag();
    let frontier = [id(0xA3), id(0xB3)];
    assert!(is_antichain(&lc, &frontier),
        "{{A3, B3}} must be a valid antichain");
}

#[test]
fn leaves_after_merge_is_singleton() {
    // After D is appended, D is the only leaf (no block lacks a descendant).
    // But in our DAG D is the last block — verify leaf count.
    let lc = build_partition_dag();
    let leaves: Vec<_> = lc.leaves().collect();
    assert_eq!(leaves.len(), 1, "after merge+descendant, exactly one leaf (D)");
    assert_eq!(leaves[0], id(0xDD), "the single leaf must be D");
}

#[test]
fn closing_antichain_before_merge_has_two_concurrent_leaves() {
    // Before the merge is added, A3 and B3 are both leaves.
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0x00), vec![],             1_000, 0)).unwrap();
    lc.insert(Block::new(id(0xA1), vec![id(0x00)],       900, 1)).unwrap();
    lc.insert(Block::new(id(0xA2), vec![id(0xA1)],       800, 2)).unwrap();
    lc.insert(Block::new(id(0xA3), vec![id(0xA2)],       700, 3)).unwrap();
    lc.insert(Block::new(id(0xB1), vec![id(0x00)],       900, 1)).unwrap();
    lc.insert(Block::new(id(0xB2), vec![id(0xB1)],       800, 2)).unwrap();
    lc.insert(Block::new(id(0xB3), vec![id(0xB2)],       700, 3)).unwrap();

    let antichain = closing_antichain(&lc);
    assert_eq!(antichain.len(), 2,
        "two concurrent chain-tips must form a 2-element antichain");
    let mut sorted = antichain.clone();
    sorted.sort();
    assert!(sorted.contains(&id(0xA3)) && sorted.contains(&id(0xB3)),
        "antichain must be {{A3, B3}}, got {sorted:?}");
    assert!(is_antichain(&lc, &antichain),
        "closing_antichain must always return a valid antichain");
}

#[test]
fn doctrine_time_arrow_ancestor_dominates_descendant() {
    // INVENTION_STACK §4.1 doctrine: "energy decay gives the time arrow."
    // At any common epoch, ancestor has more remaining energy than descendant.
    //
    // G  (epoch=0, energy=1_000) vs D (epoch=5, energy=500) at t=10.
    let λ = lambda();
    let genesis    = Block::new(id(0x00), vec![], 1_000, 0);
    let descendant = Block::new(id(0xDD), vec![id(0xCC)], 500, 5);

    assert!(
        time_arrow_holds_at(&genesis, &descendant, λ, 10),
        "at t=10, genesis (older, higher initial energy) must dominate descendant"
    );

    // And the reverse must NOT hold.
    assert!(
        !time_arrow_holds_at(&descendant, &genesis, λ, 10),
        "descendant cannot dominate genesis — wrong time direction"
    );
}

#[test]
fn doctrine_concurrent_blocks_have_no_path() {
    // The partial-order doctrine: two concurrent blocks (A1, B1) have
    // no causal relationship. Formally: neither is in the other's past.
    let lc = build_partition_dag();
    let past_a1 = causal_past(&lc, id(0xA1));
    let past_b1 = causal_past(&lc, id(0xB1));

    assert!(!past_a1.contains(&id(0xB1)),
        "B1 must NOT be in causal_past(A1)");
    assert!(!past_b1.contains(&id(0xA1)),
        "A1 must NOT be in causal_past(B1)");
}

// ── Adversarial fixture ───────────────────────────────────────────────────

#[test]
fn adversarial_missing_parent_rejected() {
    // A block that references a parent not yet in the DAG must be
    // rejected. Prevents out-of-order injection.
    let mut lc = LightCone::new();
    let err = lc.insert(Block::new(id(0x01), vec![id(0xFF)], 1_000, 1)).unwrap_err();
    assert!(
        matches!(err, LightConeError::MissingParent { block: _, parent: _ }),
        "missing parent must be rejected, got {err:?}"
    );
}

#[test]
fn adversarial_duplicate_insert_rejected() {
    // The DAG is insert-only. Reinserting an existing block is an error.
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0x00), vec![], 1_000, 0)).unwrap();
    let err = lc.insert(Block::new(id(0x00), vec![], 1_000, 0)).unwrap_err();
    assert_eq!(err, LightConeError::AlreadyInserted(id(0x00)));
}

#[test]
fn adversarial_too_many_parents_rejected() {
    // SUB-N6 audit fix: a block with more than MAX_PARENTS_PER_BLOCK
    // parents must be rejected to prevent memory/CPU DoS.
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0x00), vec![], 1_000, 0)).unwrap();

    // Build MAX_PARENTS_PER_BLOCK + 1 parent blocks.
    for i in 1..=(MAX_PARENTS_PER_BLOCK as u8 + 1) {
        lc.insert(Block::new(id(i), vec![id(0x00)], 900, 1)).unwrap();
    }
    let parent_ids: Vec<[u8; 32]> = (1..=(MAX_PARENTS_PER_BLOCK as u8 + 1)).map(id).collect();
    let err = lc.insert(Block::new(id(0xFF), parent_ids, 800, 2)).unwrap_err();
    assert!(
        matches!(err, LightConeError::TooManyParents(_, n) if n == MAX_PARENTS_PER_BLOCK + 1),
        "too-many-parents must be rejected, got {err:?}"
    );
}

#[test]
fn adversarial_antichain_rejects_comparable_pair() {
    // A set containing two comparable blocks (one precedes the other)
    // is NOT an antichain.
    let lc = build_partition_dag();
    // G precedes A3 — not an antichain.
    let not_antichain = [id(0x00), id(0xA3)];
    assert!(!is_antichain(&lc, &not_antichain),
        "{{G, A3}} must NOT be an antichain (G precedes A3)");
}

#[test]
fn adversarial_antichain_input_overflow_returns_false() {
    // SUB-N7: `is_antichain` with > MAX_ANTICHAIN_INPUT elements must
    // return false rather than burning O(n³) CPU.
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0x00), vec![], 1_000, 0)).unwrap();
    // Insert MAX_ANTICHAIN_INPUT + 1 concurrent children of genesis.
    for i in 1..=(MAX_ANTICHAIN_INPUT as u8 + 1) {
        lc.insert(Block::new(id(i), vec![id(0x00)], 900, 1)).unwrap();
    }
    let big_set: Vec<[u8; 32]> = (1..=(MAX_ANTICHAIN_INPUT as u8 + 1)).map(id).collect();
    assert!(!is_antichain(&lc, &big_set),
        "antichain check with > MAX_ANTICHAIN_INPUT must return false (DoS guard)");
}

#[test]
fn adversarial_time_arrow_false_before_common_epoch() {
    // time_arrow_holds_at returns false when t < either block's
    // observed_epoch — the arrow is only meaningful at common time.
    let λ = lambda();
    let ancestor   = Block::new(id(0x01), vec![], 1_000, 5);
    let descendant = Block::new(id(0x02), vec![id(0x01)], 900, 10);

    // t=3 is before ancestor's observed_epoch=5 → false.
    assert!(!time_arrow_holds_at(&ancestor, &descendant, λ, 3),
        "time arrow before ancestor's epoch must be false");

    // t=7 is after ancestor but before descendant's epoch=10 → false.
    assert!(!time_arrow_holds_at(&ancestor, &descendant, λ, 7),
        "time arrow before descendant's epoch must be false");

    // t=10 → valid common epoch → true.
    assert!(time_arrow_holds_at(&ancestor, &descendant, λ, 10),
        "time arrow at common epoch must be true");
}
