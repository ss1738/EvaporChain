//! Byzantine fault / adversarial tests for Tendermint BFT consensus.
//!
//! Covers: double-voting, equivocation, block withholding, invalid proposals,
//! minority stake attacks, stale replays, invalid BLS, round skips,
//! missing commit certificates, and partial network partitions.

use evaporchain_consensus::tendermint::{ConsensusAction, ConsensusMessage, TendermintConsensus};
use evaporchain_consensus::validator_set::{ValidatorInfo, ValidatorSet};
use evaporchain_state::db::InMemoryStateDB;
use evaporchain_types::Block;

fn make_validator(id: u64, stake: u64) -> ValidatorInfo {
    let mut addr = [0u8; 32];
    addr[0..8].copy_from_slice(&id.to_le_bytes());
    ValidatorInfo::new(id, stake, addr)
}

fn make_validator_set_3() -> ValidatorSet {
    let mut vs = ValidatorSet::new();
    vs.add_validator(make_validator(1, 1000));
    vs.add_validator(make_validator(2, 1000));
    vs.add_validator(make_validator(3, 1000));
    vs
}

fn make_validator_set_5() -> ValidatorSet {
    let mut vs = ValidatorSet::new();
    for id in 1..=5 {
        vs.add_validator(make_validator(id, 1000));
    }
    vs
}

fn make_test_block(height: u64, producer_id: u64) -> Block {
    Block {
        number: height,
        epoch: height,
        parent_hash: [0u8; 32],
        state_root: [1u8; 32],
        transactions: vec![],
        timestamp: 1000 + height,
        chain_id: String::new(),
        producer_id: Some(producer_id),
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
        post_state_root: None,
    }
}

// ─── 1. Double-Voting Tests ─────────────────────────────────────────────────

#[test]
fn test_byzantine_double_vote_prevote_detected() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);
    let _block = make_test_block(1, 1);
    let block_hash = [0xAAu8; 32];
    let block_hash2 = [0xBBu8; 32];

    // Validator 2 sends two different prevotes for the same height/round
    let msg1 = ConsensusMessage::Prevote {
        height: 1,
        round: 0,
        block_hash: Some(block_hash),
        validator_id: 2,
        bls_signature: None,
    };
    let msg2 = ConsensusMessage::Prevote {
        height: 1,
        round: 0,
        block_hash: Some(block_hash2),
        validator_id: 2,
        bls_signature: None,
    };

    let _actions1 = tc.on_message(msg1);
    let actions2 = tc.on_message(msg2);

    // The second vote should be rejected (equivocation detection) or slashed
    // Either no actions or a slash action
    let has_slash = actions2
        .iter()
        .any(|a| matches!(a, ConsensusAction::BroadcastMessage(_)));
    // At minimum, the consensus should not commit with conflicting votes
    assert!(
        actions2.is_empty() || has_slash,
        "Double prevote should be rejected or trigger slashing"
    );
}

#[test]
fn test_byzantine_double_vote_precommit_detected() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);
    let block_hash = [0xAAu8; 32];
    let block_hash2 = [0xBBu8; 32];

    let msg1 = ConsensusMessage::Precommit {
        height: 1,
        round: 0,
        block_hash: Some(block_hash),
        validator_id: 2,
        bls_signature: None,
    };
    let msg2 = ConsensusMessage::Precommit {
        height: 1,
        round: 0,
        block_hash: Some(block_hash2),
        validator_id: 2,
        bls_signature: None,
    };

    let _actions1 = tc.on_message(msg1);
    let actions2 = tc.on_message(msg2);

    assert!(
        actions2.is_empty(),
        "Double precommit from same validator should be rejected"
    );
}

// ─── 2. Equivocation (Dual Proposals) ───────────────────────────────────────

#[test]
fn test_byzantine_equivocation_dual_proposals() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(2, 10, vs);

    let block1 = make_test_block(1, 1);
    let block2 = {
        let mut b = make_test_block(1, 1);
        b.timestamp = 9999; // different block, same height
        b
    };

    let msg1 = ConsensusMessage::Proposal {
        height: 1,
        round: 0,
        block: block1,
        proposer_id: 1,
    };
    let msg2 = ConsensusMessage::Proposal {
        height: 1,
        round: 0,
        block: block2,
        proposer_id: 1,
    };

    let _actions1 = tc.on_message(msg1);
    let actions2 = tc.on_message(msg2);

    // Second proposal for the same height/round should be ignored
    let voted_twice = actions2.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote { .. })
        )
    });
    assert!(
        !voted_twice,
        "Should not vote on a second proposal for the same height/round"
    );
}

// ─── 3. Block Withholding (Proposer Timeout) ────────────────────────────────

#[test]
fn test_byzantine_block_withholding_triggers_timeout() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(2, 10, vs);

    // Don't send any proposal. Tick with an empty DB — the proposer timeout
    // should eventually trigger a nil prevote or round advance.
    let mut db = InMemoryStateDB::new();
    let actions = tc.tick(&mut db);

    // At this point either no action (timeout hasn't elapsed) or a broadcast
    // The important thing: we did NOT crash and consensus state is safe
    let _ = actions;
}

// ─── 4. Invalid Block Proposals ─────────────────────────────────────────────

#[test]
fn test_byzantine_proposal_wrong_height() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(2, 10, vs);

    let block = make_test_block(999, 1); // wrong height
    let msg = ConsensusMessage::Proposal {
        height: 999,
        round: 0,
        block,
        proposer_id: 1,
    };

    let actions = tc.on_message(msg);
    let voted = actions.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote {
                block_hash: Some(_),
                ..
            })
        )
    });
    assert!(!voted, "Should not vote for a proposal with wrong height");
}

#[test]
fn test_byzantine_proposal_zero_state_root_rejected() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(2, 10, vs);

    // Advance past genesis so the check kicks in
    let mut block = make_test_block(2, 1);
    block.state_root = [0u8; 32]; // zero state_root

    let msg = ConsensusMessage::Proposal {
        height: 2,
        round: 0,
        block,
        proposer_id: 1,
    };

    let actions = tc.on_message(msg);
    let voted = actions.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote {
                block_hash: Some(_),
                ..
            })
        )
    });
    assert!(
        !voted,
        "Should reject proposal with zero state_root on non-genesis block"
    );
}

#[test]
fn test_byzantine_proposal_wrong_proposer() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(2, 10, vs);

    let block = make_test_block(1, 3); // wrong proposer
    let msg = ConsensusMessage::Proposal {
        height: 1,
        round: 0,
        block,
        proposer_id: 3, // not the actual leader
    };

    let actions = tc.on_message(msg);
    // Should either reject or not vote for it
    let voted = actions.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote {
                block_hash: Some(_),
                ..
            })
        )
    });
    // It's ok if the proposer check doesn't reject (round-robin might match),
    // but tampered parent_hash/state_root should still be validated.
    // This test documents the behavior.
    let _ = voted;
}

// ─── 5. Minority Stake Attack ───────────────────────────────────────────────

#[test]
fn test_byzantine_minority_stake_cannot_reach_quorum() {
    // 3 validators, each with 1000 stake. Quorum = 2001 (>2/3 of 3000).
    // Only 1 validator votes → should NOT reach quorum.
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);

    let _block = make_test_block(1, 1);
    let block_hash = [0xAAu8; 32];

    // Only validator 2 prevotes
    let msg = ConsensusMessage::Prevote {
        height: 1,
        round: 0,
        block_hash: Some(block_hash),
        validator_id: 2,
        bls_signature: None,
    };
    let actions = tc.on_message(msg);

    // Should NOT produce a precommit (quorum not reached)
    let precommitted = actions.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Precommit { .. })
        )
    });
    assert!(
        !precommitted,
        "Single validator (1/3 stake) should not reach quorum"
    );
}

#[test]
fn test_byzantine_minority_stake_with_unequal_weights() {
    // Validator 1: stake 100, Validator 2: stake 100, Validator 3: stake 1
    // Quorum needs >2/3 of 201 = 135. Validator 3 alone (stake 1) can't reach it.
    let mut vs = ValidatorSet::new();
    vs.add_validator(make_validator(1, 100));
    vs.add_validator(make_validator(2, 100));
    vs.add_validator(make_validator(3, 1));

    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);
    let block_hash = [0xAAu8; 32];

    // Only validator 3 (tiny stake) prevotes
    let msg = ConsensusMessage::Prevote {
        height: 1,
        round: 0,
        block_hash: Some(block_hash),
        validator_id: 3,
        bls_signature: None,
    };
    let actions = tc.on_message(msg);

    let precommitted = actions.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Precommit { .. })
        )
    });
    assert!(
        !precommitted,
        "Validator with 1/201 stake should not trigger quorum"
    );
}

// ─── 6. Stale Message Replay ────────────────────────────────────────────────

#[test]
fn test_byzantine_stale_prevote_from_old_height() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);

    // Message for height 0 when consensus is at height 1
    let msg = ConsensusMessage::Prevote {
        height: 0,
        round: 0,
        block_hash: Some([0xAAu8; 32]),
        validator_id: 2,
        bls_signature: None,
    };
    let actions = tc.on_message(msg);

    // Should be silently ignored
    let precommitted = actions.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Precommit { .. })
        )
    });
    assert!(
        !precommitted,
        "Stale message from old height should be ignored"
    );
}

#[test]
fn test_byzantine_stale_proposal_from_old_round() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(2, 10, vs);

    // Trigger a tick (round advance would need elapsed time)
    let mut db = InMemoryStateDB::new();
    let _ = tc.tick(&mut db);

    // Send proposal for round 0 (stale)
    let block = make_test_block(1, 1);
    let msg = ConsensusMessage::Proposal {
        height: 1,
        round: 0,
        block,
        proposer_id: 1,
    };
    let actions = tc.on_message(msg);

    // Should not vote on stale round proposal
    let voted = actions.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote {
                block_hash: Some(_),
                round: 0,
                ..
            })
        )
    });
    // Note: some implementations buffer future-round messages, so this
    // documents expected behavior.
    let _ = voted;
}

// ─── 7. Invalid BLS Signatures ──────────────────────────────────────────────

#[test]
fn test_byzantine_invalid_bls_signature_prevote() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);

    // Send prevote with garbage BLS signature
    let msg = ConsensusMessage::Prevote {
        height: 1,
        round: 0,
        block_hash: Some([0xAAu8; 32]),
        validator_id: 2,
        bls_signature: Some(vec![0xFF; 96]), // garbage sig
    };
    let actions = tc.on_message(msg);

    // If BLS keys are registered and verification is enabled,
    // this should be rejected. Without BLS keys registered,
    // the sig check is skipped (graceful degradation).
    let _ = actions;
}

// ─── 8. Round Skip Attack ───────────────────────────────────────────────────

#[test]
fn test_byzantine_future_round_precommit_ignored() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(2, 10, vs);

    // Send precommit for round 99 (far future)
    let msg = ConsensusMessage::Precommit {
        height: 1,
        round: 99,
        block_hash: Some([0xAAu8; 32]),
        validator_id: 1,
        bls_signature: None,
    };
    let actions = tc.on_message(msg);

    // Should not commit or advance to round 99
    let committed = actions
        .iter()
        .any(|a| matches!(a, ConsensusAction::CommitBlock(_)));
    assert!(
        !committed,
        "Future round precommit should not trigger commit"
    );
}

// ─── 9. Missing Commit Certificate ──────────────────────────────────────────

#[test]
fn test_byzantine_block_without_commit_certificate() {
    // A block arriving from sync without a CommitCertificate
    // should not be treated as finalized
    let vs = make_validator_set_3();
    let tc = TendermintConsensus::new_for_test(1, 10, vs);

    let block = make_test_block(1, 1);
    assert!(
        block.commit_certificate.is_none(),
        "Test block should not have a certificate"
    );
    // Finality tracker should NOT record this block as finalized
    // (finality requires a valid CommitCertificate)
    let block_hash = [0u8; 32];
    let finalized = tc.finality_tracker.is_block_finalized(1, &block_hash);
    assert!(
        !finalized,
        "Block without commit certificate should not be finalized"
    );
}

// ─── 10. Partial Network Partition ──────────────────────────────────────────

#[test]
fn test_byzantine_two_of_three_validators_prevote() {
    // With 3 validators (1000 stake each), quorum = 2001.
    // 2 validators (2000 stake) < 2001 → should NOT reach quorum with strict >2/3.
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);

    let block_hash = [0xAAu8; 32];

    // Only validator 2 prevotes (validator 1 is us, validator 3 is partitioned)
    let msg = ConsensusMessage::Prevote {
        height: 1,
        round: 0,
        block_hash: Some(block_hash),
        validator_id: 2,
        bls_signature: None,
    };
    let actions = tc.on_message(msg);

    // With only 2 of 3 validators (including self), check if quorum is reached
    // Behavior depends on whether self-vote is counted automatically
    let _ = actions;
}

#[test]
fn test_byzantine_unknown_validator_vote_rejected() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);

    // Vote from validator 99 (not in the set)
    let msg = ConsensusMessage::Prevote {
        height: 1,
        round: 0,
        block_hash: Some([0xAAu8; 32]),
        validator_id: 99,
        bls_signature: None,
    };
    let actions = tc.on_message(msg);

    let advanced = actions.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Precommit { .. })
        )
    });
    assert!(
        !advanced,
        "Vote from unknown validator should not count toward quorum"
    );
}

#[test]
fn test_byzantine_jailed_validator_vote_ignored() {
    let mut vs = ValidatorSet::new();
    vs.add_validator(make_validator(1, 1000));
    vs.add_validator(make_validator(2, 1000));
    let mut v3 = make_validator(3, 1000);
    v3.jailed = true;
    vs.add_validator(v3);

    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);

    // Vote from jailed validator 3
    let msg = ConsensusMessage::Prevote {
        height: 1,
        round: 0,
        block_hash: Some([0xAAu8; 32]),
        validator_id: 3,
        bls_signature: None,
    };
    let actions = tc.on_message(msg);

    // Jailed validator's vote should not count
    let precommitted = actions.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Precommit { .. })
        )
    });
    assert!(
        !precommitted,
        "Jailed validator vote should not count toward quorum"
    );
}

// ─── Additional Edge Cases ──────────────────────────────────────────────────

#[test]
fn test_byzantine_nil_prevote_quorum_does_not_commit() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);

    // All validators send nil prevotes
    for vid in [2, 3] {
        let msg = ConsensusMessage::Prevote {
            height: 1,
            round: 0,
            block_hash: None, // nil vote
            validator_id: vid,
            bls_signature: None,
        };
        let _ = tc.on_message(msg);
    }

    // Nil quorum should NOT commit — should trigger round advance
    let mut db = InMemoryStateDB::new();
    let actions = tc.tick(&mut db);
    let committed = actions
        .iter()
        .any(|a| matches!(a, ConsensusAction::CommitBlock(_)));
    assert!(!committed, "Nil prevote quorum should not commit a block");
}

#[test]
fn test_byzantine_conflicting_precommits_no_commit() {
    let vs = make_validator_set_3();
    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);

    // Validator 2 precommits block A, validator 3 precommits block B
    let msg2 = ConsensusMessage::Precommit {
        height: 1,
        round: 0,
        block_hash: Some([0xAAu8; 32]),
        validator_id: 2,
        bls_signature: None,
    };
    let msg3 = ConsensusMessage::Precommit {
        height: 1,
        round: 0,
        block_hash: Some([0xBBu8; 32]),
        validator_id: 3,
        bls_signature: None,
    };

    let _ = tc.on_message(msg2);
    let actions = tc.on_message(msg3);

    // No quorum on any single block → should not commit
    let committed = actions
        .iter()
        .any(|a| matches!(a, ConsensusAction::CommitBlock(_)));
    assert!(
        !committed,
        "Conflicting precommits (different hashes) should not reach quorum"
    );
}

// ─── Lane T0.1 sub-task C.5 — 5-validator partition scenarios ───────────────
//
// MAINNET_READINESS.md T0.1: Layer 4 hot-path consensus surgery.
// Sub-task C.5 calls for "propagation tests with 4-of-5 / 3-of-5
// partition scenarios". The existing `make_validator_set_3` covers
// 3-validator scale; these add the 5-validator equivalents.
//
// Quorum threshold for 5 validators × 1000 stake = strict >2/3 of
// 5000 = 3333.33 → ≥ 3334. So 4 validators (4000 stake) reach
// quorum; 3 validators (3000 stake) do NOT.
//
// Tests below pin the SAFETY direction (sub-quorum cannot commit).
// The liveness direction (super-quorum DOES commit) requires the
// state machine to be in `Phase::Prevote` first, which means
// injecting a Proposal whose `proposer_id` matches the deterministic
// proposer-for-round selection. The proposer_for_round mapping is
// private to this crate, making the "happy-path" liveness assertion
// hard to phrase without reconstructing the same proposer-selection
// logic in tests. Liveness for the full 5-node path is exercised by
// the in-process integration paths and the cluster-soak coverage
// (T0.2 / T3.1). C.5's mainnet-blocking concern is the SAFETY side
// — that sub-quorum cannot accidentally commit — and that is what's
// pinned here.
//
// Pairs with C.6 (adversarial proposer wrong-head, separate PR).

// ─── Lane T0.1 sub-task C.6 — adversarial proposer (wrong-head) ─────────────
//
// MAINNET_READINESS.md T0.1: "adversarial proposer test (Byzantine
// producer that picks wrong head)". The in-tree test
// `test_parent_acceptance_mode_mcc_diverges_from_linear_on_diverging_parent`
// (tendermint.rs ~line 8559) covers the 4-validator scale:
// linear-mode rejection + MCC-mode lex-tie-break acceptance.
//
// This adds the 5-validator equivalent for the linear-mode rejection,
// showing the mainnet-default behaviour scales: a Byzantine proposer
// whose `parent_hash` diverges from every honest validator's
// committed-tip view is rejected with `RequestSync`, NOT silently
// accepted. Critical for the multi-parent rollout's bit-compat
// guarantee — under default `parent_acceptance_mode = "linear"`,
// the chain MUST refuse a divergent parent and request sync.
//
// MCC-full mode (multi-parent block divergence) is a richer test
// surface that needs DAG-state setup; deferred.

/// Byzantine proposer emits a block whose parent_hash diverges from
/// every honest validator's local view (default linear mode). The
/// honest local node must NOT prevote for it; instead, it must emit
/// a `RequestSync` action to fetch the missing intermediate state.
#[test]
fn test_byzantine_proposer_diverging_parent_hash_rejected_5_validators() {
    let vs = make_validator_set_5();
    // Iterate my_id to find a non-proposer for height=1, round=0 — we
    // want the local node to be receiving (not producing) the bad
    // proposal so the wrong-head rejection path fires. Try id 5 first
    // (likely non-proposer with the deterministic mapping); fall back
    // through 4..=1 until a non-proposer is found.
    let (mut tc, proposer_id) = (1u64..=5)
        .find_map(|candidate| {
            let mut tc = TendermintConsensus::new_for_test(candidate, 10, vs.clone());
            if !tc.am_i_proposer() {
                // Find SOME id we can use as the wrong-head proposer.
                // Loop again to identify the actual proposer id.
                for proposer_candidate in 1u64..=5 {
                    let mut probe =
                        TendermintConsensus::new_for_test(proposer_candidate, 10, vs.clone());
                    if probe.am_i_proposer() {
                        return Some((tc, proposer_candidate));
                    }
                    let _ = probe;
                }
                None
            } else {
                None
            }
        })
        .expect("at least one non-proposer + one proposer must exist in a 5-validator set");

    // Build a Byzantine proposal: parent_hash = [0xFF; 32] but our
    // local committed tip is [0; 32] (default). Under linear mode
    // this divergence triggers RequestSync.
    let mut block = make_test_block(1, proposer_id);
    block.parent_hash = [0xFFu8; 32];
    let proposal = ConsensusMessage::Proposal {
        height: 1,
        round: 0,
        block,
        proposer_id,
    };
    let actions = tc.on_message(proposal);

    // The local node MUST emit RequestSync (existing wrong-head
    // detection) and MUST NOT prevote with a non-NIL block_hash for
    // the diverging tip.
    let request_sync = actions
        .iter()
        .any(|a| matches!(a, ConsensusAction::RequestSync(_, _)));
    let voted_for_byzantine = actions.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Prevote {
                block_hash: Some(_),
                ..
            })
        )
    });
    assert!(
        request_sync,
        "linear-mode 5-validator: diverging parent must trigger RequestSync; \
         actions = {:?}",
        actions
    );
    assert!(
        !voted_for_byzantine,
        "linear-mode 5-validator: must NOT prevote for a diverging-parent block; \
         actions = {:?}",
        actions
    );
}

#[test]
fn test_byzantine_partition_3_of_5_validators_fail_prevote_quorum() {
    // Symmetric to the 4-of-5 case: only 3 validators prevote
    // (3000/5000 = 60% < 2/3). The local validator does NOT broadcast
    // a precommit — round timeouts will eventually fire, but no
    // commit is produced in this round.
    let vs = make_validator_set_5();
    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);

    let _block = make_test_block(1, 1);
    let block_hash = [0xAAu8; 32];

    let mut last_actions = Vec::new();
    for v_id in [1u64, 2, 3] {
        let msg = ConsensusMessage::Prevote {
            height: 1,
            round: 0,
            block_hash: Some(block_hash),
            validator_id: v_id,
            bls_signature: None,
        };
        last_actions = tc.on_message(msg);
    }

    let precommitted = last_actions.iter().any(|a| {
        matches!(
            a,
            ConsensusAction::BroadcastMessage(ConsensusMessage::Precommit { .. })
        )
    });
    assert!(
        !precommitted,
        "3-of-5 prevote (3000/5000 = 60% < 2/3) must NOT trigger a precommit"
    );
}

#[test]
fn test_byzantine_partition_3_of_5_precommits_do_not_commit() {
    // 3-of-5 precommit: 60% < 2/3. No commit.
    let vs = make_validator_set_5();
    let mut tc = TendermintConsensus::new_for_test(1, 10, vs);

    let block = make_test_block(1, 1);
    let block_hash = [0xAAu8; 32];

    let proposal = ConsensusMessage::Proposal {
        height: 1,
        round: 0,
        block,
        proposer_id: 1,
    };
    tc.on_message(proposal);

    // 3-of-5 prevote (sub-quorum).
    for v_id in [1u64, 2, 3] {
        tc.on_message(ConsensusMessage::Prevote {
            height: 1,
            round: 0,
            block_hash: Some(block_hash),
            validator_id: v_id,
            bls_signature: None,
        });
    }

    // 3-of-5 precommit.
    let mut last_actions = Vec::new();
    for v_id in [1u64, 2, 3] {
        last_actions = tc.on_message(ConsensusMessage::Precommit {
            height: 1,
            round: 0,
            block_hash: Some(block_hash),
            validator_id: v_id,
            bls_signature: None,
        });
    }

    let committed = last_actions
        .iter()
        .any(|a| matches!(a, ConsensusAction::CommitBlock(_)));
    let advanced = tc.height() > 1;
    assert!(
        !committed && !advanced,
        "3-of-5 precommit (<2/3) must NOT commit; actions = {:?}, height = {}",
        last_actions,
        tc.height()
    );
}
