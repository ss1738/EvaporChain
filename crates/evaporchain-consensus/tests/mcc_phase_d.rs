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
