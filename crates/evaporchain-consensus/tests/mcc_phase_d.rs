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
