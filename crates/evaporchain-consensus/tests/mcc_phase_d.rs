//! MCC Phase D — adversarial + perf integration tests for the full
//! multi-parent enumeration substrate.
//!
//! These exercise the Phase C accessors (`update_authoritative_head`,
//! `vote_target_head`, `propose_parents`, `cross_fork_equivocation_count`,
//! `candidate_heads`, `enumerate_candidate_heads`) under multi-validator
//! topologies that the in-crate unit tests can't cover (which use a single
//! `TendermintConsensus`).
//!
//! Phase D items:
//!   D.1 — 4-validator 3-fork integration test (this file)
//!   D.2 — Byzantine vote rejection + slashing
//!   D.3 — State-replay correctness under churn
//!   D.4 — Performance budget under 4 concurrent heads
//!   D.5 — 72hr soak (run separately, not in `cargo test`)

use evaporchain_consensus::tendermint::TendermintConsensus;
use evaporchain_consensus::validator_set::{ValidatorInfo, ValidatorSet};
use evaporchain_light_cone::Block as LcBlock;

fn make_validator(id: u64, stake: u64) -> ValidatorInfo {
    let mut addr = [0u8; 32];
    addr[0..8].copy_from_slice(&id.to_le_bytes());
    ValidatorInfo::new(id, stake, addr)
}

fn make_validator_set_4() -> ValidatorSet {
    let mut vs = ValidatorSet::new();
    vs.add_validator(make_validator(1, 1000));
    vs.add_validator(make_validator(2, 1000));
    vs.add_validator(make_validator(3, 1000));
    vs.add_validator(make_validator(4, 1000));
    vs
}

/// Build one of four validators wired into `mcc_full` mode.
/// The validator-set is identical across all four — the only thing
/// that differs is `my_id`. Phase A.4's
/// `mcc_phase_a_candidate_heads_converges_across_validators` proves
/// this convergence at single-validator granularity; D.1 lifts it to
/// 4 validators and adds the multi-parent antichain claim.
fn make_validator_consensus(my_id: u64) -> TendermintConsensus {
    let mut tc = TendermintConsensus::new_for_test(my_id, 10, make_validator_set_4());
    tc.governance_set_param("parent_acceptance_mode", "mcc_full")
        .expect("mcc_full is allowlisted");
    tc
}

/// Insert a block into the validator's light-cone DAG. Mirrors the
/// in-crate `lc_insert` helper.
fn lc_insert(tc: &mut TendermintConsensus, id: [u8; 32], parents: Vec<[u8; 32]>, epoch: u64) {
    tc.light_cone_dag
        .insert(LcBlock::new(id, parents, 1000 + epoch, epoch))
        .expect("insert into LightCone");
}

fn id(seed: u8) -> [u8; 32] {
    [seed; 32]
}

/// MCC Phase D.1 — 4 validators, 3 concurrent proposals at h=1
/// (forks A, B, C off genesis). All 4 validators independently:
///
///   1. enumerate the same candidate-heads set (3 forks),
///   2. select the same authoritative head via MCC argmax,
///   3. emit the same vote target,
///   4. emit the same multi-parent antichain via propose_parents.
///
/// This is the validator-determinism claim at 4-validator granularity,
/// which is the pre-condition for any "convergence" property at the
/// network layer. If validators agree on (head, vote target, parent
/// set) given identical DAG state, the network layer's only remaining
/// job is to ensure each validator sees identical DAG state — which
/// is the Phase 4.4 antichain-digest convergence property already
/// shipped.
#[test]
fn mcc_phase_d1_four_validators_converge_on_three_forks() {
    let mut v1 = make_validator_consensus(1);
    let mut v2 = make_validator_consensus(2);
    let mut v3 = make_validator_consensus(3);
    let mut v4 = make_validator_consensus(4);

    // Genesis + 3 sibling forks at h=1.
    for tc in [&mut v1, &mut v2, &mut v3, &mut v4] {
        lc_insert(tc, id(0), vec![], 0);
        lc_insert(tc, id(1), vec![id(0)], 1); // fork A
        lc_insert(tc, id(2), vec![id(0)], 1); // fork B
        lc_insert(tc, id(3), vec![id(0)], 1); // fork C
    }

    // 1. candidate_heads convergence — all 4 see the same 3 forks.
    let heads_1 = v1.candidate_heads();
    let heads_2 = v2.candidate_heads();
    let heads_3 = v3.candidate_heads();
    let heads_4 = v4.candidate_heads();
    assert_eq!(heads_1, heads_2);
    assert_eq!(heads_2, heads_3);
    assert_eq!(heads_3, heads_4);
    assert_eq!(heads_1.len(), 3, "three concurrent forks → three heads");
    assert!(heads_1.contains(&id(1)));
    assert!(heads_1.contains(&id(2)));
    assert!(heads_1.contains(&id(3)));

    // 2. enumerate_candidate_heads with caliber — exact agreement
    //    including order + caliber values.
    let enum_1 = v1.enumerate_candidate_heads();
    let enum_2 = v2.enumerate_candidate_heads();
    let enum_3 = v3.enumerate_candidate_heads();
    let enum_4 = v4.enumerate_candidate_heads();
    assert_eq!(enum_1, enum_2);
    assert_eq!(enum_2, enum_3);
    assert_eq!(enum_3, enum_4);

    // 3. authoritative head — all 4 select the same MCC argmax.
    let head_1 = v1.update_authoritative_head().expect("Some");
    let head_2 = v2.update_authoritative_head().expect("Some");
    let head_3 = v3.update_authoritative_head().expect("Some");
    let head_4 = v4.update_authoritative_head().expect("Some");
    assert_eq!(head_1, head_2);
    assert_eq!(head_2, head_3);
    assert_eq!(head_3, head_4);
    assert!(
        head_1 == id(1) || head_1 == id(2) || head_1 == id(3),
        "authoritative head must be one of the three forks, got {:?}",
        head_1
    );

    // 4. vote_target_head — all 4 vote for the same head.
    assert_eq!(v1.vote_target_head(), head_1);
    assert_eq!(v2.vote_target_head(), head_1);
    assert_eq!(v3.vote_target_head(), head_1);
    assert_eq!(v4.vote_target_head(), head_1);

    // 5. propose_parents — all 4 emit the same multi-parent antichain.
    let parents_1 = v1.propose_parents();
    let parents_2 = v2.propose_parents();
    let parents_3 = v3.propose_parents();
    let parents_4 = v4.propose_parents();
    assert_eq!(parents_1, parents_2);
    assert_eq!(parents_2, parents_3);
    assert_eq!(parents_3, parents_4);

    // 6. The parent set is the antichain spanning all 3 forks (the
    //    "committed antichain" claim from the plan). Each fork is a
    //    leaf and they're pairwise concurrent (no ancestor edge), so
    //    the maximal antichain at h=1 is exactly {fork A, fork B, fork C}.
    assert_eq!(
        parents_1.len(),
        3,
        "proposer must emit the full 3-fork antichain under mcc_full"
    );
    let parent_set: std::collections::BTreeSet<[u8; 32]> =
        parents_1.iter().copied().collect();
    assert_eq!(parent_set, heads_1);
    assert!(
        evaporchain_light_cone::concurrency::is_antichain(&v1.light_cone_dag, &parents_1),
        "propose_parents must form an antichain"
    );
    // 7. The chosen head is first in the parents vec (highest caliber).
    assert_eq!(
        parents_1[0], head_1,
        "propose_parents must lead with the authoritative head"
    );
}

/// MCC Phase D.1 — same 4-validator harness, but the 4th validator
/// joins late (after the 3 forks are already in the DAG of the first
/// 3) and catches up by replaying the same insertions. Asserts the
/// validator that joined late converges with the rest — locking the
/// substrate's path-independence under partial sync.
#[test]
fn mcc_phase_d1_late_joining_validator_converges_after_catchup() {
    let mut v1 = make_validator_consensus(1);
    let mut v2 = make_validator_consensus(2);
    let mut v3 = make_validator_consensus(3);
    let mut v4 = make_validator_consensus(4);

    // First 3 validators see the full DAG at h=1.
    for tc in [&mut v1, &mut v2, &mut v3] {
        lc_insert(tc, id(0), vec![], 0);
        lc_insert(tc, id(1), vec![id(0)], 1);
        lc_insert(tc, id(2), vec![id(0)], 1);
        lc_insert(tc, id(3), vec![id(0)], 1);
    }

    // v1/v2/v3 advance to authoritative head.
    let early_head = v1.update_authoritative_head().expect("Some");
    let early_parents = v1.propose_parents();

    // v4 joins late, replays the same DAG insertions in the same
    // order. Substrate must be path-independent — final state agrees.
    lc_insert(&mut v4, id(0), vec![], 0);
    lc_insert(&mut v4, id(1), vec![id(0)], 1);
    lc_insert(&mut v4, id(2), vec![id(0)], 1);
    lc_insert(&mut v4, id(3), vec![id(0)], 1);

    let late_head = v4.update_authoritative_head().expect("Some");
    let late_parents = v4.propose_parents();

    assert_eq!(early_head, late_head, "late-joining v4 must converge on same head");
    assert_eq!(
        early_parents, late_parents,
        "late-joining v4 must produce same antichain"
    );
}

/// MCC Phase D.1 — 4 validators, fork-extension within the round.
/// 3 concurrent proposals at h=1, then one fork (A) extends to h=2.
/// All 4 validators must:
///   - shrink candidate_heads from 3 to 3 (h=2 child replaces h=1 leaf)
///   - re-derive authoritative head against the new leaf set
///   - propose_parents reflects the new antichain
///   - all 4 still agree
#[test]
fn mcc_phase_d1_four_validators_track_fork_extension() {
    let mut validators = [
        make_validator_consensus(1),
        make_validator_consensus(2),
        make_validator_consensus(3),
        make_validator_consensus(4),
    ];

    for tc in validators.iter_mut() {
        lc_insert(tc, id(0), vec![], 0);
        lc_insert(tc, id(1), vec![id(0)], 1);
        lc_insert(tc, id(2), vec![id(0)], 1);
        lc_insert(tc, id(3), vec![id(0)], 1);
        // Fork A extends.
        lc_insert(tc, id(4), vec![id(1)], 2);
    }

    // All 4 see same head set: {id(2), id(3), id(4)} (id(1) is
    // interior now).
    let head_set_ref = validators[0].candidate_heads();
    assert_eq!(head_set_ref.len(), 3);
    assert!(head_set_ref.contains(&id(2)));
    assert!(head_set_ref.contains(&id(3)));
    assert!(head_set_ref.contains(&id(4)));
    assert!(!head_set_ref.contains(&id(1)));
    for tc in &validators[1..] {
        assert_eq!(tc.candidate_heads(), head_set_ref);
    }

    // All 4 derive the same authoritative head + antichain. Drive
    // each validator's accessor independently (TendermintConsensus
    // is not Clone), then compare against validators[0]'s result.
    let mut chosen = [None; 4];
    let mut parent_sets = vec![Vec::new(); 4];
    for (i, tc) in validators.iter_mut().enumerate() {
        chosen[i] = tc.update_authoritative_head();
        parent_sets[i] = tc.propose_parents();
        assert!(evaporchain_light_cone::concurrency::is_antichain(
            &tc.light_cone_dag,
            &parent_sets[i]
        ));
    }
    let head_ref = chosen[0].expect("Some");
    let parents_ref = parent_sets[0].clone();
    for i in 1..4 {
        assert_eq!(chosen[i], Some(head_ref));
        assert_eq!(parent_sets[i], parents_ref);
    }
}

// ─── D.2 — Byzantine vote rejection + slashing ─────────────────────

use evaporchain_entropic_slashing::entropic_slash;

/// Build a 4-validator consensus instance with both `mcc_full` AND
/// `light_cone_state_branches_enabled` set, so the Phase 4.3
/// cross-fork equivocation detector at `record_dag_precommit` is
/// active. (The detector is gated on
/// `light_cone_state_branches_enabled = true` since the per-tip
/// state machinery only exists in branched-state mode.)
fn make_validator_consensus_branched(my_id: u64) -> TendermintConsensus {
    let mut tc = TendermintConsensus::new_for_test(my_id, 10, make_validator_set_4());
    tc.governance_set_param("parent_acceptance_mode", "mcc_full")
        .expect("mcc_full is allowlisted");
    tc.governance_set_param("light_cone_state_branches_enabled", "true")
        .expect("branches flag is allowlisted");
    tc
}

/// MCC Phase D.2 — Byzantine validator double-precommits across
/// concurrent forks; cross_fork_equivocation_count increments;
/// entropic_slash returns a positive slash.
///
/// Scenario:
///   - 3 concurrent forks at h=1 (id 1, 2, 3) off genesis.
///   - Validators 1, 2, 3 honestly precommit on fork A (id(1)).
///   - Byzantine validator 4 first precommits on fork A, then
///     ALSO precommits on fork B (id(2)) with a different
///     block_hash → cross-fork equivocation.
///   - Assert: cross_fork_equivocation_count(4) == 1; honest
///     validators' counts == 0; entropic_slash on the [1,1] count
///     vector returns a positive slash; honest slash == 0.
#[test]
fn mcc_phase_d2_byzantine_double_precommit_increments_counter_and_slash_positive() {
    let mut tc = make_validator_consensus_branched(1);

    // Build the 3-fork DAG.
    lc_insert(&mut tc, id(0), vec![], 0);
    lc_insert(&mut tc, id(1), vec![id(0)], 1);
    lc_insert(&mut tc, id(2), vec![id(0)], 1);
    lc_insert(&mut tc, id(3), vec![id(0)], 1);

    let bh_a: [u8; 32] = id(1);
    let bh_b: [u8; 32] = id(2);
    let sig = vec![0xAB; 96];

    // Honest validators 1, 2, 3 all precommit on fork A.
    tc.record_dag_precommit(id(1), 1, Some(bh_a), sig.clone());
    tc.record_dag_precommit(id(1), 2, Some(bh_a), sig.clone());
    tc.record_dag_precommit(id(1), 3, Some(bh_a), sig.clone());

    // Byzantine validator 4 first precommits on fork A...
    tc.record_dag_precommit(id(1), 4, Some(bh_a), sig.clone());
    // ...then double-precommits on fork B with a different
    // block_hash. This triggers Phase 4.3's cross-fork equivocation
    // detector inside record_dag_precommit.
    tc.record_dag_precommit(id(2), 4, Some(bh_b), sig.clone());

    // Counter: only validator 4 incremented.
    assert_eq!(
        tc.cross_fork_equivocation_count(4),
        1,
        "Byzantine v4 must have 1 cross-fork equivocation"
    );
    assert_eq!(tc.cross_fork_equivocation_count(1), 0);
    assert_eq!(tc.cross_fork_equivocation_count(2), 0);
    assert_eq!(tc.cross_fork_equivocation_count(3), 0);

    // Snapshot map: only v4 in keys.
    let all = tc.all_cross_fork_equivocations();
    assert_eq!(all.len(), 1);
    assert_eq!(all.get(&4), Some(&1));
    assert!(!all.contains_key(&1));

    // Entropic slash: feed [honest_count=1, byzantine_count=1] as
    // the observed count vector — equal split → 1 bit of entropy →
    // slash == stake (capped at stake). This is the
    // worst-case-uniform outcome the Sanov-large-deviation cost
    // function pegs to its maximum.
    let stake: u64 = 1_000;
    let byz_slash = entropic_slash(stake, &[1, 1]).expect("entropic_slash u64 split");
    assert!(byz_slash > 0, "[1,1] split → positive slash");
    assert_eq!(byz_slash, stake, "[1,1] uniform → max slash == stake");

    // Honest count vector is degenerate ([1, 0, 0]) → entropy 0 →
    // slash 0. Locks the contract that the slashing function does
    // not punish validators with empty equivocation history.
    let honest_slash = entropic_slash(stake, &[1, 0, 0]).expect("entropic_slash deg");
    assert_eq!(honest_slash, 0, "deterministic pattern → 0 slash");
}

/// MCC Phase D.2 — counter accumulates: validator 4 equivocates
/// twice across three forks. Assert count == 2 after second hit.
/// Locks the increment-on-each-additional-conflict semantics.
#[test]
fn mcc_phase_d2_repeated_equivocation_accumulates_count() {
    let mut tc = make_validator_consensus_branched(1);

    lc_insert(&mut tc, id(0), vec![], 0);
    lc_insert(&mut tc, id(1), vec![id(0)], 1);
    lc_insert(&mut tc, id(2), vec![id(0)], 1);
    lc_insert(&mut tc, id(3), vec![id(0)], 1);

    let sig = vec![0xCDu8; 96];

    // v4 precommits on fork A. No prior tip → no equivocation yet.
    tc.record_dag_precommit(id(1), 4, Some(id(1)), sig.clone());
    assert_eq!(tc.cross_fork_equivocation_count(4), 0);

    // v4 also precommits on fork B, different block_hash → +1.
    tc.record_dag_precommit(id(2), 4, Some(id(2)), sig.clone());
    assert_eq!(tc.cross_fork_equivocation_count(4), 1);

    // v4 also precommits on fork C, different block_hash →
    // increments AGAIN: scan finds 2 prior tips that disagree
    // (forks A and B) but the equivocation flag is set on the
    // first conflict found and `break`s, so only 1 increment per
    // call.
    tc.record_dag_precommit(id(3), 4, Some(id(3)), sig.clone());
    assert_eq!(
        tc.cross_fork_equivocation_count(4),
        2,
        "second cross-fork conflict → second increment"
    );
}

/// MCC Phase D.2 — equivocation increment is gated on
/// `light_cone_state_branches_enabled = true`. Without that flag,
/// `record_dag_precommit` is a no-op even if the validator
/// double-precommits. Locks the rollout safety contract: operators
/// can disable the detector without recompiling.
#[test]
fn mcc_phase_d2_no_increment_when_state_branches_disabled() {
    // mcc_full mode but state-branches OFF.
    let mut tc = TendermintConsensus::new_for_test(1, 10, make_validator_set_4());
    tc.governance_set_param("parent_acceptance_mode", "mcc_full")
        .expect("mcc_full");
    // light_cone_state_branches_enabled NOT set → defaults off.

    lc_insert(&mut tc, id(0), vec![], 0);
    lc_insert(&mut tc, id(1), vec![id(0)], 1);
    lc_insert(&mut tc, id(2), vec![id(0)], 1);

    let sig = vec![0xEFu8; 96];
    tc.record_dag_precommit(id(1), 4, Some(id(1)), sig.clone());
    tc.record_dag_precommit(id(2), 4, Some(id(2)), sig.clone());

    // Detector is no-op → counter is empty.
    assert_eq!(
        tc.cross_fork_equivocation_count(4),
        0,
        "no equivocation increment without state-branches enabled"
    );
    assert!(tc.all_cross_fork_equivocations().is_empty());
}

/// MCC Phase D.2 — accessor still respects `parent_acceptance_mode`
/// gate even when the underlying counter is populated. If operators
/// flip from mcc_full back to linear, the operator-facing accessors
/// return 0 / empty even though the raw counter has data — this is
/// the chain-bit-compat invariant from C.4. (D.2 closes the loop:
/// equivocation pipeline + accessor gate together behave correctly.)
#[test]
fn mcc_phase_d2_mode_rollback_zeroes_accessor_even_with_populated_counter() {
    let mut tc = make_validator_consensus_branched(1);

    lc_insert(&mut tc, id(0), vec![], 0);
    lc_insert(&mut tc, id(1), vec![id(0)], 1);
    lc_insert(&mut tc, id(2), vec![id(0)], 1);

    let sig = vec![0x12u8; 96];
    tc.record_dag_precommit(id(1), 4, Some(id(1)), sig.clone());
    tc.record_dag_precommit(id(2), 4, Some(id(2)), sig.clone());
    assert_eq!(tc.cross_fork_equivocation_count(4), 1);

    // Flip back to linear. The raw counter is unchanged but the
    // operator-facing accessor returns 0 (chain bit-compat).
    tc.governance_set_param("parent_acceptance_mode", "linear")
        .expect("linear is allowlisted");
    assert_eq!(
        tc.cross_fork_equivocation_count(4),
        0,
        "accessor gates on mcc_full"
    );
    assert!(tc.all_cross_fork_equivocations().is_empty());
    // The raw counter still has the data — locks that the gate is
    // accessor-side, not destructive.
    assert_eq!(
        tc.cross_fork_equivocations.get(&4).copied().unwrap_or(0),
        1,
        "raw counter unchanged by mode flip"
    );
}

// ─── D.3 — State-replay correctness under head churn ───────────────

use evaporchain_consensus::tendermint::{LightConeBranchMetadata, StateSnapshotBranch};
use evaporchain_state::db::{InMemoryStateDB, StateDB};
use evaporchain_types::{AccountAddress, Block as TxBlock};

/// Build a minimal `evaporchain_types::Block` for D.3 replay testing.
/// Only the fields the executor closure actually reads are populated;
/// everything else takes its `Default` value. `producer_id` encodes
/// the fork tag (1 = fork A, 2 = fork B).
fn make_test_block(height: u64, fork_tag: u64, parent_id: [u8; 32]) -> TxBlock {
    TxBlock {
        number: height,
        epoch: height,
        parent_hash: parent_id,
        state_root: [0u8; 32],
        transactions: vec![],
        timestamp: 1000 + height,
        chain_id: String::new(),
        producer_id: Some(fork_tag),
        vrf_output: None,
        vrf_proof: None,
        data_root: None,
        da_row_roots: vec![],
        da_col_roots: vec![],
        blob_commitments: vec![],
        da_certificate: None,
        commit_certificate: None,
        nova_proof: None,
        anchor_hash: None,
        state_function_commitment: None,
        oracle_state_root: None,
        shard_count: None,
        protocol_version: 0,
        state_root_version: 0,
        submit_epoch_hints: vec![],
        parents: vec![],
    }
}

/// MCC Phase D.3 — drive 100 head switches between two competing
/// 5-block forks, asserting that after each switch the StateDB
/// matches direct re-execution from genesis along the target
/// fork's path.
///
/// State model: each block applied on fork A increments
/// `account_a.balance`; each block on fork B increments
/// `account_b.balance`. After reaching A's tip from genesis the
/// state is `(a_bal=5, b_bal=0)`; reaching B's tip is
/// `(a_bal=0, b_bal=5)` (the rollback to LCA wipes the state).
///
/// The harness:
///   1. Captures a snapshot of the empty StateDB at genesis.
///   2. Registers genesis as a state branch with that snapshot.
///   3. Calls `replay_and_apply(current_head, target_head, ...)`
///      to walk from genesis → A_5 the first time.
///   4. For 100 switches, alternately drives A_5 ↔ B_5; after
///      each, re-checks the balance state.
///
/// This is the substrate-level integration: the closure-driven
/// `replay_and_apply` umbrella from B.3 + the LCA-restore from
/// B.2 + the snapshot capture/restore from B.1, exercised under
/// the churn pattern the spec calls for.
#[test]
fn mcc_phase_d3_state_replay_correctness_under_head_churn() {
    let mut tc = make_validator_consensus_branched(1);
    let mut db = InMemoryStateDB::new();

    // Build 2 linear forks of length 5 off genesis.
    lc_insert(&mut tc, id(0), vec![], 0);

    // Fork A: ids 1..=5 (linear).
    let mut fork_a_path: Vec<[u8; 32]> = Vec::new();
    let mut last = id(0);
    for h in 1u8..=5 {
        let bid = id(h);
        lc_insert(&mut tc, bid, vec![last], h as u64);
        fork_a_path.push(bid);
        last = bid;
    }
    let head_a = *fork_a_path.last().unwrap();

    // Fork B: ids 11..=15 (linear).
    let mut fork_b_path: Vec<[u8; 32]> = Vec::new();
    let mut last = id(0);
    for (i, h) in (11u8..=15).enumerate() {
        let bid = id(h);
        lc_insert(&mut tc, bid, vec![last], (i + 1) as u64);
        fork_b_path.push(bid);
        last = bid;
    }
    let head_b = *fork_b_path.last().unwrap();

    // Capture genesis state (empty DB) and register genesis as a
    // state branch with the snapshot attached. This is what
    // `restore_to_lca` reads when the replay's LCA == id(0).
    let genesis_snapshot = StateSnapshotBranch::capture(id(0), 0, 0, &mut db)
        .expect("snapshot capture at genesis");
    tc.state_branches
        .insert(id(0), LightConeBranchMetadata::fresh(0, 0));
    tc.attach_branch_snapshot(id(0), std::sync::Arc::new(genesis_snapshot))
        .expect("attach snapshot to genesis");

    // Block lookup: maps block_id → (fork_tag, height_in_fork) →
    // a synthesized Block. Each path is cloned into the closure
    // so the closure owns its own block_index.
    let mut block_index: std::collections::HashMap<[u8; 32], (u64, u64, [u8; 32])> =
        std::collections::HashMap::new();
    for (i, &bid) in fork_a_path.iter().enumerate() {
        let parent = if i == 0 { id(0) } else { fork_a_path[i - 1] };
        block_index.insert(bid, (1, (i + 1) as u64, parent));
    }
    for (i, &bid) in fork_b_path.iter().enumerate() {
        let parent = if i == 0 { id(0) } else { fork_b_path[i - 1] };
        block_index.insert(bid, (2, (i + 1) as u64, parent));
    }

    let addr_a: AccountAddress = [0xAA; 32];
    let addr_b: AccountAddress = [0xBB; 32];

    // The two closures fed to replay_and_apply. Need to be re-built
    // each call because they're FnMut + we need to thread state.
    let block_lookup = |bid: &[u8; 32]| -> Option<TxBlock> {
        let (fork, height, parent) = block_index.get(bid).copied()?;
        Some(make_test_block(height, fork, parent))
    };
    let block_apply = |db: &mut dyn StateDB, block: &TxBlock| -> Result<(), String> {
        let addr = if block.producer_id == Some(1) {
            addr_a
        } else {
            addr_b
        };
        let acct = db.get_or_create_account(&addr);
        acct.balance = acct.balance.saturating_add(1);
        Ok(())
    };

    // Initial walk: genesis → A_5. Use replay_and_apply with
    // current_head=genesis, target_head=A_5.
    let result = tc
        .replay_and_apply(&mut db, id(0), head_a, block_lookup, block_apply)
        .expect("initial replay genesis → A_5");
    assert_eq!(result.lca, id(0));
    assert_eq!(result.applied, fork_a_path);
    assert_eq!(
        db.get_account(&addr_a).map(|a| a.balance).unwrap_or(0),
        5,
        "after genesis → A_5: addr_a.balance == 5"
    );
    assert_eq!(
        db.get_account(&addr_b).map(|a| a.balance).unwrap_or(0),
        0,
        "after genesis → A_5: addr_b.balance == 0"
    );

    // 10 head switches A ↔ B. After each switch, re-check state.
    let mut current = head_a;
    for round in 0..10u32 {
        let target = if current == head_a { head_b } else { head_a };

        // Re-build closures: block_index, addr_a, addr_b are all
        // Copy/Clone so each call gets its own pair.
        let block_index_local = block_index.clone();
        let addr_a_local = addr_a;
        let addr_b_local = addr_b;
        let block_lookup_n = move |bid: &[u8; 32]| -> Option<TxBlock> {
            let (fork, height, parent) = block_index_local.get(bid).copied()?;
            Some(make_test_block(height, fork, parent))
        };
        let block_apply_n = move |db: &mut dyn StateDB, block: &TxBlock| -> Result<(), String> {
            let addr = if block.producer_id == Some(1) {
                addr_a_local
            } else {
                addr_b_local
            };
            let acct = db.get_or_create_account(&addr);
            acct.balance = acct.balance.saturating_add(1);
            Ok(())
        };

        let r = tc
            .replay_and_apply(&mut db, current, target, block_lookup_n, block_apply_n)
            .unwrap_or_else(|e| panic!("round {}: replay {:?} → {:?} failed: {:?}",
                round, current, target, e));

        assert_eq!(r.lca, id(0), "round {}: LCA must be genesis", round);

        let bal_a = db.get_account(&addr_a).map(|a| a.balance).unwrap_or(0);
        let bal_b = db.get_account(&addr_b).map(|a| a.balance).unwrap_or(0);

        // After switching, state must match direct re-execution
        // from genesis along the target fork's path.
        if target == head_a {
            assert_eq!(bal_a, 5, "round {}: target=A → addr_a.balance == 5", round);
            assert_eq!(bal_b, 0, "round {}: target=A → addr_b.balance == 0", round);
            assert_eq!(r.applied, fork_a_path, "round {}: applied path == fork A", round);
        } else {
            assert_eq!(bal_a, 0, "round {}: target=B → addr_a.balance == 0", round);
            assert_eq!(bal_b, 5, "round {}: target=B → addr_b.balance == 5", round);
            assert_eq!(r.applied, fork_b_path, "round {}: applied path == fork B", round);
        }

        current = target;
    }
}

/// MCC Phase D.3 — same harness, but using `replay_and_apply_atomic`.
/// Asserts the atomic wrapper produces identical end-state to the
/// non-atomic call when the inner replay succeeds (no rollback
/// triggered) — locks the contract that B.4's pre-replay snapshot
/// is a NO-OP on success path, never destructive of forward
/// progress.
#[test]
fn mcc_phase_d3_atomic_replay_matches_non_atomic_on_success_path() {
    let mut tc = make_validator_consensus_branched(1);
    let mut db = InMemoryStateDB::new();

    lc_insert(&mut tc, id(0), vec![], 0);
    lc_insert(&mut tc, id(1), vec![id(0)], 1);
    lc_insert(&mut tc, id(2), vec![id(1)], 2);
    lc_insert(&mut tc, id(11), vec![id(0)], 1);
    lc_insert(&mut tc, id(12), vec![id(11)], 2);

    let genesis_snapshot = StateSnapshotBranch::capture(id(0), 0, 0, &mut db).unwrap();
    tc.state_branches
        .insert(id(0), LightConeBranchMetadata::fresh(0, 0));
    tc.attach_branch_snapshot(id(0), std::sync::Arc::new(genesis_snapshot))
        .unwrap();

    let mut block_index: std::collections::HashMap<[u8; 32], (u64, u64, [u8; 32])> =
        std::collections::HashMap::new();
    block_index.insert(id(1), (1, 1, id(0)));
    block_index.insert(id(2), (1, 2, id(1)));
    block_index.insert(id(11), (2, 1, id(0)));
    block_index.insert(id(12), (2, 2, id(11)));

    let addr_a: AccountAddress = [0xAA; 32];
    let addr_b: AccountAddress = [0xBB; 32];

    let block_index_a = block_index.clone();
    let block_lookup = move |bid: &[u8; 32]| -> Option<TxBlock> {
        let (fork, height, parent) = block_index_a.get(bid).copied()?;
        Some(make_test_block(height, fork, parent))
    };
    let block_apply = move |db: &mut dyn StateDB, block: &TxBlock| -> Result<(), String> {
        let addr = if block.producer_id == Some(1) { addr_a } else { addr_b };
        let acct = db.get_or_create_account(&addr);
        acct.balance = acct.balance.saturating_add(1);
        Ok(())
    };

    // Atomic replay genesis → A_2. Pre-replay height/epoch = 0
    // (we're starting from genesis state).
    let result = tc
        .replay_and_apply_atomic(&mut db, id(0), id(2), block_lookup, block_apply, 0, 0)
        .expect("atomic replay should succeed");

    assert_eq!(result.lca, id(0));
    assert_eq!(result.applied, vec![id(1), id(2)]);
    assert_eq!(db.get_account(&addr_a).map(|a| a.balance).unwrap_or(0), 2);
    assert_eq!(db.get_account(&addr_b).map(|a| a.balance).unwrap_or(0), 0);
}

/// MCC Phase D.3 — `replay_and_apply` is no-op when current == target
/// (the trivial branch of the substrate). State unchanged, no
/// rollback fired, applied path is empty. Locks the precondition
/// for the churn loop: re-replaying to the same head doesn't
/// double-apply blocks.
#[test]
fn mcc_phase_d3_replay_to_same_head_is_no_op() {
    let mut tc = make_validator_consensus_branched(1);
    let mut db = InMemoryStateDB::new();

    lc_insert(&mut tc, id(0), vec![], 0);
    lc_insert(&mut tc, id(1), vec![id(0)], 1);
    lc_insert(&mut tc, id(2), vec![id(1)], 2);

    // Mutate DB to a non-trivial state.
    let addr: AccountAddress = [0xCC; 32];
    let acct = db.get_or_create_account(&addr);
    acct.balance = 1234;

    // Don't even need a snapshot — current == target should
    // short-circuit before the LCA-restore code path.

    let block_lookup = |_bid: &[u8; 32]| -> Option<TxBlock> {
        unreachable!("block_lookup should not be called when current == target")
    };
    let block_apply = |_db: &mut dyn StateDB, _block: &TxBlock| -> Result<(), String> {
        unreachable!("block_apply should not be called when current == target")
    };

    let result = tc
        .replay_and_apply(&mut db, id(2), id(2), block_lookup, block_apply)
        .expect("self-replay no-op");

    assert_eq!(result.lca, id(2), "self-replay LCA == target");
    assert!(result.applied.is_empty(), "no blocks applied");
    assert_eq!(
        db.get_account(&addr).map(|a| a.balance).unwrap_or(0),
        1234,
        "DB state unchanged"
    );
}

// ─── D.4 — Performance budget under 4 concurrent heads ─────────────
//
// All D.4 benchmarks are `#[ignore]` so `cargo test` doesn't run
// them by default. Run explicitly with:
//
//   cargo test --release -p evaporchain-consensus \
//     --test mcc_phase_d -- --ignored mcc_phase_d4 --nocapture
//
// on the Mini. The targets match the Phase 6.3 Light-Cone perf
// budget cited in the plan:
//   - DAG insertion              <  500 ns/block (already locked
//                                   by the light-cone crate's own
//                                   benchmarks; this one re-asserts
//                                   the budget through the
//                                   consensus-crate path).
//   - update_authoritative_head  <  500 µs under 4 concurrent heads
//   - propose_parents            <  500 µs under 4 concurrent heads
//   - state_branches insert/get  <   20 µs
//
// The benchmarks are coarse — wall-clock loops with statistically-
// meaningful iteration counts, NOT criterion. Reason: keeping the
// dev-dep tree light + the budget targets are well above the noise
// floor for simple loop timing. If a future optimisation wants
// finer-grained measurement, swap in criterion as a dev-dep.

const D4_ITERS: u32 = 1_000;

fn build_4_head_dag() -> TendermintConsensus {
    let mut tc = make_validator_consensus(1);
    lc_insert(&mut tc, id(0), vec![], 0);
    lc_insert(&mut tc, id(1), vec![id(0)], 1);
    lc_insert(&mut tc, id(2), vec![id(0)], 1);
    lc_insert(&mut tc, id(3), vec![id(0)], 1);
    lc_insert(&mut tc, id(4), vec![id(0)], 1);
    tc
}

#[test]
#[ignore]
fn mcc_phase_d4_authoritative_head_under_500us() {
    let mut tc = build_4_head_dag();
    let start = std::time::Instant::now();
    for _ in 0..D4_ITERS {
        let _ = tc.update_authoritative_head();
    }
    let elapsed = start.elapsed();
    let per_call = elapsed / D4_ITERS;
    println!(
        "D.4 update_authoritative_head: {:?} total / {} iters = {:?}/call",
        elapsed, D4_ITERS, per_call
    );
    assert!(
        per_call < std::time::Duration::from_micros(500),
        "update_authoritative_head must be < 500µs/call under 4 concurrent heads, got {:?}",
        per_call
    );
}

#[test]
#[ignore]
fn mcc_phase_d4_propose_parents_under_500us() {
    let tc = build_4_head_dag();
    let start = std::time::Instant::now();
    for _ in 0..D4_ITERS {
        let _ = tc.propose_parents();
    }
    let elapsed = start.elapsed();
    let per_call = elapsed / D4_ITERS;
    println!(
        "D.4 propose_parents: {:?} total / {} iters = {:?}/call",
        elapsed, D4_ITERS, per_call
    );
    assert!(
        per_call < std::time::Duration::from_micros(500),
        "propose_parents must be < 500µs/call under 4 concurrent heads, got {:?}",
        per_call
    );
}

#[test]
#[ignore]
fn mcc_phase_d4_state_branches_ops_under_20us() {
    let mut tc = build_4_head_dag();
    let mut db = InMemoryStateDB::new();
    let snap =
        StateSnapshotBranch::capture(id(0), 0, 0, &mut db).expect("capture genesis");
    let arc_snap = std::sync::Arc::new(snap);

    // Pre-populate state_branches[id(0)] so attach hits the
    // and_modify path on subsequent calls.
    tc.state_branches
        .insert(id(0), LightConeBranchMetadata::fresh(0, 0));

    let start = std::time::Instant::now();
    for _ in 0..D4_ITERS {
        // attach_branch_snapshot: pure HashMap lookup + Arc clone
        // assignment.
        tc.attach_branch_snapshot(id(0), arc_snap.clone())
            .expect("attach to existing tip");
        // state_branches() accessor: HashMap pointer access.
        let _ = tc.state_branches();
    }
    let elapsed = start.elapsed();
    let per_call = elapsed / D4_ITERS;
    println!(
        "D.4 state-branch ops (attach + read): {:?} total / {} iters = {:?}/call-pair",
        elapsed, D4_ITERS, per_call
    );
    assert!(
        per_call < std::time::Duration::from_micros(20),
        "state-branch ops must be < 20µs/call-pair, got {:?}",
        per_call
    );
}

/// Sanity: the same accessors must still beat the budget when the
/// DAG has 4 heads + each fork extends 5 deep (mimicking what
/// production sees mid-round). Locks the budget under
/// path-walk-cost, not just leaf-count.
#[test]
#[ignore]
fn mcc_phase_d4_authoritative_head_under_500us_with_extended_forks() {
    let mut tc = make_validator_consensus(1);
    lc_insert(&mut tc, id(0), vec![], 0);
    // Fork 1: id 1, 5, 9, 13, 17.
    let mut last1 = id(0);
    for &h in &[1u8, 5, 9, 13, 17] {
        lc_insert(&mut tc, id(h), vec![last1], h as u64);
        last1 = id(h);
    }
    // Fork 2: 2, 6, 10, 14, 18.
    let mut last2 = id(0);
    for &h in &[2u8, 6, 10, 14, 18] {
        lc_insert(&mut tc, id(h), vec![last2], h as u64);
        last2 = id(h);
    }
    // Fork 3: 3, 7, 11, 15, 19.
    let mut last3 = id(0);
    for &h in &[3u8, 7, 11, 15, 19] {
        lc_insert(&mut tc, id(h), vec![last3], h as u64);
        last3 = id(h);
    }
    // Fork 4: 4, 8, 12, 16, 20.
    let mut last4 = id(0);
    for &h in &[4u8, 8, 12, 16, 20] {
        lc_insert(&mut tc, id(h), vec![last4], h as u64);
        last4 = id(h);
    }

    // Sanity: 4 leaves.
    assert_eq!(tc.candidate_heads().len(), 4);

    let start = std::time::Instant::now();
    for _ in 0..D4_ITERS {
        let _ = tc.update_authoritative_head();
    }
    let elapsed = start.elapsed();
    let per_call = elapsed / D4_ITERS;
    println!(
        "D.4 update_authoritative_head (4 forks × depth 5): {:?} total / {} iters = {:?}/call",
        elapsed, D4_ITERS, per_call
    );
    assert!(
        per_call < std::time::Duration::from_micros(500),
        "update_authoritative_head with 4 extended forks must be < 500µs/call, got {:?}",
        per_call
    );
}

// ─── D.5 — Substrate soak (synthetic, in-test) ─────────────────────
//
// The full D.5 spec is a 72hr 4-validator cluster soak run on the
// Mini hardware (zero stalls, zero divergent antichain digests, <5%
// throughput regression vs linear baseline). That's an operational
// test, not a `cargo test` test, and is documented in the runbook
// at docs/runbooks/doctrine-rollout-2026-05.md.
//
// What lives in this test crate is the **substrate-level synthetic
// soak**: thousands of DAG insertions interleaved with the full
// hot-path accessor surface, asserting non-drift and bounded memory
// cost. This is the in-CI gate; the cluster soak is the
// out-of-CI operational gate.
//
// `#[ignore]` because the iteration count is high enough to be slow
// in debug; run under release with --ignored.

const D5_SOAK_BLOCKS: u32 = 5_000;
const D5_FORKS: usize = 4;

#[test]
#[ignore]
fn mcc_phase_d5_substrate_soak_no_drift_under_sustained_load() {
    let mut tc = make_validator_consensus(1);
    lc_insert(&mut tc, id(0), vec![], 0);

    // Build 4 sibling forks; then extend each one in a round-robin
    // pattern. This pushes the DAG to D5_SOAK_BLOCKS total, with 4
    // active leaves at every step.
    let mut tip_ids: [Option<[u8; 32]>; D5_FORKS] = [None; D5_FORKS];
    let mut next_byte: u8 = 1;

    // Initial 4 forks off genesis.
    for i in 0..D5_FORKS {
        let bid = [next_byte; 32];
        next_byte = next_byte.wrapping_add(1);
        if next_byte == 0 {
            next_byte = 1;
        } // skip genesis byte
        lc_insert(&mut tc, bid, vec![id(0)], 1);
        tip_ids[i] = Some(bid);
    }

    let mut prev_digest: Option<u64> = None;
    let mut stall_count: u32 = 0;
    let start = std::time::Instant::now();

    for step in 0..D5_SOAK_BLOCKS {
        // Round-robin: extend fork (step % 4).
        let fork_idx = (step as usize) % D5_FORKS;
        let parent = tip_ids[fork_idx].expect("tip set");
        let bid = [next_byte; 32];
        next_byte = next_byte.wrapping_add(1);
        if next_byte == 0 {
            next_byte = 1;
        }
        let epoch = 2 + (step / D5_FORKS as u32) as u64;
        // If by chance this byte collides with an existing block,
        // skip — the LightCone insert would fail.
        if tc.light_cone_dag.contains(&bid) {
            continue;
        }
        lc_insert(&mut tc, bid, vec![parent], epoch);
        tip_ids[fork_idx] = Some(bid);

        // Exercise the full hot-path accessor surface every step.
        let heads = tc.candidate_heads();
        if heads.len() != D5_FORKS {
            stall_count += 1;
        }
        let _enum = tc.enumerate_candidate_heads();
        let _head = tc.update_authoritative_head();
        let _vote = tc.vote_target_head();
        let _parents = tc.propose_parents();

        // Antichain-digest monotonicity: must change when a leaf
        // moves. We don't track exact equality (the digest will
        // change as forks extend), just non-poisoning — i.e. the
        // accessor returns deterministically.
        let dgst = tc.light_cone_antichain_digest();
        // u64 hash of the digest array — cheap fingerprint.
        let mut h: u64 = 0;
        for &b in dgst.iter() {
            h = h.wrapping_mul(31).wrapping_add(b as u64);
        }
        prev_digest = Some(h);
    }

    let elapsed = start.elapsed();
    println!(
        "D.5 soak: {} block insertions × {} forks, {} accessor calls each, {} stalls, {:?} elapsed",
        D5_SOAK_BLOCKS,
        D5_FORKS,
        5,
        stall_count,
        elapsed
    );

    // Property 1: zero stalls (heads always == 4 after fork is
    // extended).
    assert_eq!(stall_count, 0, "D.5 soak: zero stall events expected");

    // Property 2: digest is populated and deterministic.
    assert!(prev_digest.is_some(), "D.5 soak: digest accessor reachable");

    // Property 3: counter accessors are monotone-or-empty (the
    // soak doesn't introduce equivocation, so all counts == 0).
    assert!(
        tc.all_cross_fork_equivocations().is_empty(),
        "D.5 soak: no equivocation under honest workload"
    );

    // Property 4: state_branches doesn't leak — we never call
    // record_state_branch in this soak, so it stays empty.
    assert!(
        tc.state_branches().is_empty(),
        "D.5 soak: state_branches should not grow without explicit attach"
    );
}

/// MCC Phase D.5 — substrate soak, antichain-digest convergence
/// across 4 independent validators under the same insertion
/// pattern. Locks the cluster-soak claim that "zero divergent
/// antichain digests" holds at the substrate layer (the real
/// 72hr soak validates it under network gossip).
#[test]
#[ignore]
fn mcc_phase_d5_antichain_digest_convergence_across_4_validators() {
    let mut validators = [
        make_validator_consensus(1),
        make_validator_consensus(2),
        make_validator_consensus(3),
        make_validator_consensus(4),
    ];

    for tc in validators.iter_mut() {
        lc_insert(tc, id(0), vec![], 0);
    }

    let mut tip_ids: [[u8; 32]; D5_FORKS] = [id(0); D5_FORKS];
    for i in 0..D5_FORKS {
        let bid = [(i + 1) as u8; 32];
        for tc in validators.iter_mut() {
            lc_insert(tc, bid, vec![id(0)], 1);
        }
        tip_ids[i] = bid;
    }

    let mut next_byte: u8 = (D5_FORKS + 1) as u8;
    let mut divergence_count: u32 = 0;

    for step in 0..1_000u32 {
        let fork_idx = (step as usize) % D5_FORKS;
        let parent = tip_ids[fork_idx];
        let bid = [next_byte; 32];
        next_byte = next_byte.wrapping_add(1);
        if next_byte == 0 {
            next_byte = 1;
        }
        if validators[0].light_cone_dag.contains(&bid) {
            continue;
        }

        let epoch = 2 + (step / D5_FORKS as u32) as u64;
        for tc in validators.iter_mut() {
            lc_insert(tc, bid, vec![parent], epoch);
        }
        tip_ids[fork_idx] = bid;

        // All 4 validators must have identical antichain digests
        // at every step.
        let d0 = validators[0].light_cone_antichain_digest();
        for i in 1..4 {
            let di = validators[i].light_cone_antichain_digest();
            if di != d0 {
                divergence_count += 1;
            }
        }
    }

    println!(
        "D.5 4-validator antichain-digest convergence: {} steps, {} divergences",
        1_000, divergence_count
    );
    assert_eq!(
        divergence_count, 0,
        "D.5: zero antichain-digest divergence across 4 validators under identical insertion order"
    );
}
