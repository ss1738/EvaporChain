//! Cross-module coverage for evaporchain-light-cone — DAG insert
//! caps, leaves, LCA, path-from-to, prune variants, antichain input
//! cap, Decay-Lamport derivation parity + error variants, time arrow
//! integration, serde round-trips.

use evaporchain_energy_kernel::{ChainLambda, Lambda};
use evaporchain_light_cone::{
    all_block_clocks, block_lamport_clock, causal_future, causal_past, closing_antichain,
    closing_antichain_digest, comparable, digest_antichain, find_first_divergence, find_lca,
    is_antichain, is_concurrent, precedes, time_arrow_holds_at,
    block_path_from_to,
    dag::{LightConeError, MAX_PARENTS_PER_BLOCK},
    decay_lamport::ClockDerivationError,
    concurrency::MAX_ANTICHAIN_INPUT,
    Block, BlockId, LightCone,
};

fn id(b: u8) -> BlockId {
    let mut x = [0u8; 32];
    x[31] = b;
    x
}

/// Diamond DAG: g(0) → {a(1), b(2)} → m(3).
fn diamond() -> LightCone {
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0), vec![], 1_000, 0)).unwrap();
    lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap();
    lc.insert(Block::new(id(2), vec![id(0)], 900, 1)).unwrap();
    lc.insert(Block::new(id(3), vec![id(1), id(2)], 800, 2)).unwrap();
    lc
}

// =================================================================
// Doctrine pins
// =================================================================

#[test]
fn doctrine_caps_are_pinned() {
    assert_eq!(MAX_PARENTS_PER_BLOCK, 16);
    assert_eq!(MAX_ANTICHAIN_INPUT, 64);
}

// =================================================================
// Insert — caps + error variants
// =================================================================

#[test]
fn insert_rejects_seventeen_parents_with_diagnostic() {
    let mut lc = LightCone::new();
    // 16 valid parent blocks.
    for i in 0..16u8 {
        lc.insert(Block::new(id(i), vec![], 1, 0)).unwrap();
    }
    // Crafting a 17-parent child must be rejected at the cap.
    let parents: Vec<BlockId> = (0..17u8).map(id).collect();
    let err = lc.insert(Block::new(id(100), parents, 1, 1)).unwrap_err();
    match err {
        LightConeError::TooManyParents(b, n) => {
            assert_eq!(b, id(100));
            assert_eq!(n, 17);
        }
        other => panic!("expected TooManyParents, got {other:?}"),
    }
    // Block was NOT inserted on rejection.
    assert!(!lc.contains(&id(100)));
}

#[test]
fn insert_at_exact_cap_succeeds() {
    let mut lc = LightCone::new();
    for i in 0..16u8 {
        lc.insert(Block::new(id(i), vec![], 1, 0)).unwrap();
    }
    let parents: Vec<BlockId> = (0..16u8).map(id).collect();
    lc.insert(Block::new(id(100), parents, 1, 1)).unwrap();
    assert!(lc.contains(&id(100)));
}

#[test]
fn missing_parent_error_carries_block_and_parent_ids() {
    let mut lc = LightCone::new();
    let err = lc.insert(Block::new(id(5), vec![id(99)], 1, 1)).unwrap_err();
    match err {
        LightConeError::MissingParent { block, parent } => {
            assert_eq!(block, id(5));
            assert_eq!(parent, id(99));
        }
        other => panic!("expected MissingParent, got {other:?}"),
    }
}

#[test]
fn lightcone_error_display_includes_ids() {
    let e = LightConeError::AlreadyInserted(id(7));
    let s = e.to_string();
    assert!(s.to_lowercase().contains("already"));
    let e = LightConeError::TooManyParents(id(8), 99);
    assert!(e.to_string().contains("99"));
}

// =================================================================
// leaves() + closing_antichain
// =================================================================

#[test]
fn leaves_returns_only_childless_blocks() {
    let lc = diamond();
    let leaves: Vec<BlockId> = lc.leaves().collect();
    assert_eq!(leaves, vec![id(3)]);
}

#[test]
fn leaves_on_multi_tip_dag_returns_all_tips() {
    // g → a, g → b — both a and b are leaves.
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0), vec![], 1, 0)).unwrap();
    lc.insert(Block::new(id(1), vec![id(0)], 1, 1)).unwrap();
    lc.insert(Block::new(id(2), vec![id(0)], 1, 1)).unwrap();
    let mut leaves: Vec<BlockId> = lc.leaves().collect();
    leaves.sort();
    assert_eq!(leaves, vec![id(1), id(2)]);
    // closing_antichain = leaves, sorted.
    let ac = closing_antichain(&lc);
    assert!(is_antichain(&lc, &ac));
}

// =================================================================
// LCA
// =================================================================

#[test]
fn find_lca_in_diamond_is_genesis() {
    let lc = diamond();
    assert_eq!(find_lca(&lc, id(1), id(2)), Some(id(0)));
}

#[test]
fn find_lca_with_self_returns_self() {
    let lc = diamond();
    // ancestors_a includes a itself; intersection includes a → highest-epoch.
    assert_eq!(find_lca(&lc, id(3), id(3)), Some(id(3)));
}

#[test]
fn find_lca_missing_block_returns_none() {
    let lc = diamond();
    assert!(find_lca(&lc, id(99), id(3)).is_none());
    assert!(find_lca(&lc, id(3), id(99)).is_none());
}

#[test]
fn find_lca_disjoint_dags_return_none() {
    // Two genesis blocks with no shared ancestor.
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0), vec![], 1, 0)).unwrap();
    lc.insert(Block::new(id(1), vec![], 1, 0)).unwrap();
    assert!(find_lca(&lc, id(0), id(1)).is_none());
}

// =================================================================
// block_path_from_to (first-parent chain)
// =================================================================

#[test]
fn block_path_same_block_is_empty() {
    let lc = diamond();
    assert_eq!(block_path_from_to(&lc, id(3), id(3)), Some(vec![]));
}

#[test]
fn block_path_walks_first_parent_chain() {
    let lc = diamond();
    // m(3).parents = [a(1), b(2)] — first-parent chain: m → a → g.
    let path = block_path_from_to(&lc, id(0), id(3)).unwrap();
    assert_eq!(path, vec![id(1), id(3)]);
}

#[test]
fn block_path_from_non_ancestor_returns_none() {
    let lc = diamond();
    // a(1) and b(2) are concurrent; b is not on a's first-parent chain.
    assert!(block_path_from_to(&lc, id(2), id(1)).is_none());
}

#[test]
fn block_path_unknown_endpoint_returns_none() {
    let lc = diamond();
    assert!(block_path_from_to(&lc, id(99), id(3)).is_none());
    assert!(block_path_from_to(&lc, id(0), id(99)).is_none());
}

// =================================================================
// prune_before_epoch + prune_orphan_branch
// =================================================================

#[test]
fn prune_before_epoch_drops_old_blocks_and_edges() {
    let mut lc = diamond();
    // Diamond observed_epochs: 0, 1, 1, 2. Prune anything < epoch 2.
    let n_pruned = lc.prune_before_epoch(2);
    assert_eq!(n_pruned, 3);
    assert!(!lc.contains(&id(0)));
    assert!(lc.contains(&id(3)));
    // m's `parents` Vec still names the pruned ids, so causal_past
    // surfaces them (but doesn't traverse deeper — they're gone from
    // the DAG, so no further expansion happens).
    let past = causal_past(&lc, id(3));
    assert_eq!(past.len(), 2, "past names m's direct parents only");
    assert!(past.contains(&id(1)));
    assert!(past.contains(&id(2)));
    assert!(!past.contains(&id(0)), "genesis no longer reachable through DAG");
}

#[test]
fn prune_orphan_branch_on_leaf_removes_exclusive_chain() {
    // g → a → b, where b is a leaf. Prune b → b dropped; a is now leaf;
    // but a is reachable only from b, so a goes too; g has no other
    // children → g goes. All three pruned.
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0), vec![], 1, 0)).unwrap();
    lc.insert(Block::new(id(1), vec![id(0)], 1, 1)).unwrap();
    lc.insert(Block::new(id(2), vec![id(1)], 1, 2)).unwrap();
    let pruned = lc.prune_orphan_branch(id(2));
    assert_eq!(pruned.len(), 3);
    assert!(lc.is_empty());
}

#[test]
fn prune_orphan_branch_stops_at_shared_branch_point() {
    // g → a, g → b. Prune b — only b drops; g has surviving child a.
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0), vec![], 1, 0)).unwrap();
    lc.insert(Block::new(id(1), vec![id(0)], 1, 1)).unwrap();
    lc.insert(Block::new(id(2), vec![id(0)], 1, 1)).unwrap();
    let pruned = lc.prune_orphan_branch(id(2));
    assert_eq!(pruned.len(), 1);
    assert!(pruned.contains(&id(2)));
    assert!(lc.contains(&id(0)));
    assert!(lc.contains(&id(1)));
}

#[test]
fn prune_orphan_branch_non_leaf_is_noop() {
    let lc_orig = diamond();
    let mut lc = lc_orig.clone();
    // id(0) has children — pruning it is rejected.
    let pruned = lc.prune_orphan_branch(id(0));
    assert!(pruned.is_empty());
    assert_eq!(lc.len(), lc_orig.len());
}

#[test]
fn prune_orphan_branch_missing_tip_is_noop() {
    let mut lc = diamond();
    let pruned = lc.prune_orphan_branch(id(99));
    assert!(pruned.is_empty());
}

// =================================================================
// is_antichain — input cap
// =================================================================

#[test]
fn is_antichain_rejects_oversized_input() {
    let lc = diamond();
    let oversized: Vec<BlockId> = (0..(MAX_ANTICHAIN_INPUT as u8 + 1))
        .map(id)
        .collect();
    assert!(!is_antichain(&lc, &oversized));
}

#[test]
fn is_antichain_at_cap_size_processes_normally() {
    // 64 distinct random ids that aren't in the DAG — comparable() will
    // return false for unknown blocks → is_antichain returns true.
    let lc = diamond();
    let at_cap: Vec<BlockId> = (10..(10 + MAX_ANTICHAIN_INPUT as u8))
        .map(id)
        .collect();
    assert_eq!(at_cap.len(), MAX_ANTICHAIN_INPUT);
    assert!(is_antichain(&lc, &at_cap));
}

// =================================================================
// Decay-Lamport — error variants + cross-API agreement
// =================================================================

#[test]
fn block_lamport_clock_zero_quantum_errs() {
    let lc = diamond();
    let err = block_lamport_clock(&lc, id(3), 0).unwrap_err();
    assert_eq!(err, ClockDerivationError::ZeroQuantum);
}

#[test]
fn block_lamport_clock_unknown_block_errs() {
    let lc = diamond();
    let err = block_lamport_clock(&lc, id(99), 100).unwrap_err();
    assert_eq!(err, ClockDerivationError::BlockNotFound(id(99)));
}

#[test]
fn all_block_clocks_agrees_with_per_block_query() {
    let lc = diamond();
    let all = all_block_clocks(&lc, 100).unwrap();
    for b in [id(0), id(1), id(2), id(3)] {
        let one = block_lamport_clock(&lc, b, 100).unwrap();
        assert_eq!(*all.get(&b).unwrap(), one);
    }
}

#[test]
fn all_block_clocks_zero_quantum_errs() {
    let lc = diamond();
    assert_eq!(
        all_block_clocks(&lc, 0).unwrap_err(),
        ClockDerivationError::ZeroQuantum
    );
}

// =================================================================
// causal_past / causal_future — known-block + unknown-block paths
// =================================================================

#[test]
fn causal_past_of_unknown_returns_empty() {
    let lc = diamond();
    assert!(causal_past(&lc, id(99)).is_empty());
}

#[test]
fn causal_future_of_unknown_returns_empty() {
    let lc = diamond();
    assert!(causal_future(&lc, id(99)).is_empty());
}

#[test]
fn causal_past_excludes_start() {
    let lc = diamond();
    let past = causal_past(&lc, id(3));
    assert!(!past.contains(&id(3)));
    assert!(past.contains(&id(0)));
    assert!(past.contains(&id(1)));
    assert!(past.contains(&id(2)));
}

#[test]
fn causal_future_of_genesis_includes_all_descendants() {
    let lc = diamond();
    let fut = causal_future(&lc, id(0));
    assert_eq!(fut.len(), 3);
    assert!(fut.contains(&id(1)));
    assert!(fut.contains(&id(2)));
    assert!(fut.contains(&id(3)));
}

// =================================================================
// Time arrow integration
// =================================================================

#[test]
fn time_arrow_holds_when_ancestor_outweighs_descendant() {
    let ancestor = Block::new(id(0), vec![], 2_000, 0);
    let descendant = Block::new(id(1), vec![id(0)], 500, 10);
    let lambda = ChainLambda::new(Lambda::from_epochs(100));
    assert!(time_arrow_holds_at(&ancestor, &descendant, lambda, 50));
}

// =================================================================
// concurrency convenience predicates
// =================================================================

#[test]
fn precedes_is_antisymmetric_in_diamond() {
    let lc = diamond();
    assert!(precedes(&lc, id(0), id(3)));
    assert!(!precedes(&lc, id(3), id(0)));
    assert!(!precedes(&lc, id(1), id(1))); // not reflexive
}

#[test]
fn comparable_concurrent_partition() {
    let lc = diamond();
    // Every pair in {g, a, b, m} is either comparable or concurrent,
    // never both, never neither.
    for x in [id(0), id(1), id(2), id(3)] {
        for y in [id(0), id(1), id(2), id(3)] {
            if x == y {
                assert!(comparable(&lc, x, y));
                assert!(!is_concurrent(&lc, x, y));
            } else {
                assert_ne!(comparable(&lc, x, y), is_concurrent(&lc, x, y));
            }
        }
    }
}

// =================================================================
// digest_antichain — domain-separated empty digest sentinel
// =================================================================

#[test]
fn digest_of_empty_antichain_matches_domain_tag_only() {
    let d = digest_antichain(&[]);
    let mut h = blake3::Hasher::new();
    h.update(b"evaporchain-antichain-digest-v1");
    let expected = *h.finalize().as_bytes();
    assert_eq!(d, expected);
}

#[test]
fn closing_antichain_digest_composes_with_step_by_step() {
    let lc = diamond();
    assert_eq!(
        closing_antichain_digest(&lc),
        digest_antichain(&closing_antichain(&lc))
    );
}

// =================================================================
// find_first_divergence — edge cases
// =================================================================

#[test]
fn find_first_divergence_returns_lowest_overlap() {
    let a = vec![(5u64, [50u8; 32]), (6, [60; 32]), (7, [70; 32])];
    let b = vec![(5, [50u8; 32]), (6, [99; 32]), (7, [70; 32])];
    let dp = find_first_divergence(&a, &b).unwrap();
    assert_eq!(dp.height, 6);
    assert_eq!(dp.local_digest, [60; 32]);
    assert_eq!(dp.remote_digest, [99; 32]);
}

// =================================================================
// Serde round-trips
// =================================================================

#[test]
fn block_serde_round_trips() {
    let b = Block::new(id(7), vec![id(0), id(1)], 4242, 99);
    let json = serde_json::to_string(&b).unwrap();
    let back: Block = serde_json::from_str(&json).unwrap();
    assert_eq!(back, b);
}

// NOTE: full `LightCone` JSON round-trip is intentionally not tested
// here — the internal `BTreeMap<[u8; 32], _>` uses array keys, which
// serde_json rejects ("key must be a string"). The struct is serde-
// derived, so a binary format like bincode or postcard would work; but
// JSON isn't the on-the-wire format for the DAG.
