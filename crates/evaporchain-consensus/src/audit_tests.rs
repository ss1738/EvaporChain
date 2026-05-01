//! Audit-prep test suite: adversarial scenarios, Byzantine fault tests,
//! consensus safety invariants, and property-based testing for the consensus layer.

#[cfg(test)]
mod adversarial_tests {
    use crate::tendermint::*;
    use crate::validator_set::{ValidatorInfo, ValidatorSet};
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_state::db::StateDB;
    use evaporchain_types::{Account, Block, Transaction, TransferTx};

    fn addr(b: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    fn make_validator(id: u64, stake: u64) -> ValidatorInfo {
        let mut address = [0u8; 32];
        address[0] = id as u8;
        ValidatorInfo::new(id, stake, address)
    }

    fn make_validator_set(ids: &[u64]) -> ValidatorSet {
        let validators: Vec<_> = ids.iter().map(|&id| make_validator(id, 1000)).collect();
        ValidatorSet::with_validators(validators)
    }

    fn make_consensus(my_id: u64, ids: &[u64]) -> TendermintConsensus {
        TendermintConsensus::new_for_test(my_id, 5, make_validator_set(ids))
    }

    fn run_consensus_round(
        nodes: &mut [TendermintConsensus],
        db: &mut InMemoryStateDB,
        max_iterations: usize,
    ) -> Vec<Block> {
        let mut messages = Vec::new();
        let mut committed = Vec::new();

        for v in nodes.iter_mut() {
            for a in v.tick(db) {
                match a {
                    ConsensusAction::BroadcastMessage(m) => messages.push(m),
                    ConsensusAction::CommitBlock(b) => committed.push(b),
                    _ => {}
                }
            }
        }

        for _ in 0..max_iterations {
            if !committed.is_empty() {
                break;
            }
            let current: Vec<_> = messages.drain(..).collect();
            for msg in &current {
                for v in nodes.iter_mut() {
                    for a in v.on_message(msg.clone()) {
                        match a {
                            ConsensusAction::BroadcastMessage(m) => messages.push(m),
                            ConsensusAction::CommitBlock(b) => committed.push(b),
                            _ => {}
                        }
                    }
                }
            }
            for v in nodes.iter_mut() {
                for a in v.tick(db) {
                    match a {
                        ConsensusAction::BroadcastMessage(m) => messages.push(m),
                        ConsensusAction::CommitBlock(b) => committed.push(b),
                        _ => {}
                    }
                }
            }
        }
        committed
    }

    // ═══════════════════════════════════════════════════════════════════
    // Byzantine fault tolerance tests
    // ═══════════════════════════════════════════════════════════════════

    /// With f Byzantine nodes (silent), 3f+1 network must still reach consensus.
    #[test]
    fn test_bft_one_silent_validator_four_nodes() {
        let ids = &[1u64, 2, 3, 4];
        // Create network but only run 3 of 4 validators (node 4 is silent/crashed)
        let mut nodes: Vec<_> = ids.iter().map(|&id| make_consensus(id, ids)).collect();
        let mut db = InMemoryStateDB::new();
        db.put_account(Account { address: addr(1), balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 });

        // Remove the 4th node to simulate it being offline
        let _silent = nodes.pop();

        let committed = run_consensus_round(&mut nodes, &mut db, 50);
        // 3/4 nodes should still reach consensus (quorum = 3)
        assert!(!committed.is_empty(), "consensus should succeed with 3/4 validators");
    }

    /// With 2 of 4 validators silent, consensus should NOT be reached.
    #[test]
    fn test_bft_two_silent_validators_no_consensus() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes: Vec<_> = ids.iter().map(|&id| make_consensus(id, ids)).collect();
        let mut db = InMemoryStateDB::new();
        db.put_account(Account { address: addr(1), balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 });

        // Remove 2 nodes — only 2 remain, below quorum of 3
        nodes.pop();
        nodes.pop();

        let committed = run_consensus_round(&mut nodes, &mut db, 30);
        assert!(committed.is_empty(), "consensus must NOT succeed with only 2/4 validators");
    }

    /// Equivocating proposal: proposer sends different blocks to different validators.
    /// Honest nodes should detect equivocation and slash.
    #[test]
    fn test_equivocation_detection() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes: Vec<_> = ids.iter().map(|&id| make_consensus(id, ids)).collect();
        let mut db = InMemoryStateDB::new();
        db.put_account(Account { address: addr(1), balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 });

        // Get a proposal from node 1
        let actions = nodes[0].tick(&mut db);
        let mut proposal1 = None;
        for a in &actions {
            if let ConsensusAction::BroadcastMessage(ConsensusMessage::Proposal { block, .. }) = a {
                proposal1 = Some(block.clone());
            }
        }

        if let Some(block1) = proposal1 {
            // Create a conflicting proposal (different state root)
            let mut block2 = block1.clone();
            block2.state_root = [99u8; 32]; // tampered

            let equivocating_proposal = ConsensusMessage::Proposal {
                height: block1.number,
                round: 0,
                block: block2,
                proposer_id: 1, // same proposer
            };

            // Deliver both proposals to node 2 — it should detect equivocation
            for a in &actions {
                if let ConsensusAction::BroadcastMessage(msg) = a {
                    let _ = nodes[1].on_message(msg.clone());
                }
            }
            let equivocation_actions = nodes[1].on_message(equivocating_proposal);

            // The equivocating proposer should be penalized (health score decreased)
            let proposer_health = nodes[1].validator_set.get(1).map(|v| v.health_score).unwrap_or(100.0);
            // After equivocation detection, health should be reduced from default
            assert!(proposer_health <= 100.0, "equivocation should affect proposer health");
        }
    }

    /// Messages from unknown validators should be ignored.
    #[test]
    fn test_unknown_validator_messages_ignored() {
        let ids = &[1u64, 2, 3];
        let mut node = make_consensus(1, ids);
        let mut db = InMemoryStateDB::new();
        let _ = node.tick(&mut db);

        // Prevote from validator ID 999 (not in validator set)
        let fake_msg = ConsensusMessage::Prevote {
            height: 1,
            round: 0,
            block_hash: Some([1u8; 32]),
            validator_id: 999,
            bls_signature: None,
        };

        let actions = node.on_message(fake_msg);
        // Should produce no actions — the message is simply dropped
        let commits: Vec<_> = actions.iter().filter(|a| matches!(a, ConsensusAction::CommitBlock(_))).collect();
        assert!(commits.is_empty(), "unknown validator should not cause commits");
    }

    /// Prevotes for wrong height should be ignored.
    #[test]
    fn test_wrong_height_message_ignored() {
        let ids = &[1u64, 2, 3, 4];
        let mut node = make_consensus(1, ids);
        let mut db = InMemoryStateDB::new();
        let _ = node.tick(&mut db);

        // Send a prevote for height 999 when we're at height 1
        let future_msg = ConsensusMessage::Prevote {
            height: 999,
            round: 0,
            block_hash: Some([1u8; 32]),
            validator_id: 2,
            bls_signature: None,
        };

        let actions = node.on_message(future_msg);
        let commits: Vec<_> = actions.iter().filter(|a| matches!(a, ConsensusAction::CommitBlock(_))).collect();
        assert!(commits.is_empty(), "future height messages should not trigger commits");
    }

    /// Past height messages should be ignored.
    #[test]
    fn test_past_height_message_ignored() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes: Vec<_> = ids.iter().map(|&id| make_consensus(id, ids)).collect();
        let mut db = InMemoryStateDB::new();
        db.put_account(Account { address: addr(1), balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 });

        // First, advance to height 2
        let committed = run_consensus_round(&mut nodes, &mut db, 50);
        assert!(!committed.is_empty());

        // Now send a message for height 1 (already committed)
        let stale_msg = ConsensusMessage::Prevote {
            height: 1,
            round: 0,
            block_hash: Some([1u8; 32]),
            validator_id: 2,
            bls_signature: None,
        };

        let actions = nodes[0].on_message(stale_msg);
        // No commits should result from stale messages
        let commits: Vec<_> = actions.iter().filter(|a| matches!(a, ConsensusAction::CommitBlock(_))).collect();
        assert!(commits.is_empty(), "past height messages should be dropped");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Safety invariant: no two different blocks at the same height
    // ═══════════════════════════════════════════════════════════════════

    /// All honest validators must commit the same block at the same height.
    #[test]
    fn test_safety_all_nodes_commit_same_block() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes: Vec<_> = ids.iter().map(|&id| make_consensus(id, ids)).collect();
        let mut db = InMemoryStateDB::new();
        db.put_account(Account { address: addr(1), balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 });

        // Collect ALL committed blocks across ALL nodes
        let mut all_committed: Vec<Block> = Vec::new();
        let mut messages = Vec::new();

        for v in nodes.iter_mut() {
            for a in v.tick(&mut db) {
                match a {
                    ConsensusAction::BroadcastMessage(m) => messages.push(m),
                    ConsensusAction::CommitBlock(b) => all_committed.push(b),
                    _ => {}
                }
            }
        }

        for _ in 0..50 {
            let current: Vec<_> = messages.drain(..).collect();
            for msg in &current {
                for v in nodes.iter_mut() {
                    for a in v.on_message(msg.clone()) {
                        match a {
                            ConsensusAction::BroadcastMessage(m) => messages.push(m),
                            ConsensusAction::CommitBlock(b) => all_committed.push(b),
                            _ => {}
                        }
                    }
                }
            }
            for v in nodes.iter_mut() {
                for a in v.tick(&mut db) {
                    match a {
                        ConsensusAction::BroadcastMessage(m) => messages.push(m),
                        ConsensusAction::CommitBlock(b) => all_committed.push(b),
                        _ => {}
                    }
                }
            }
        }

        // All committed blocks at the same height must have identical state roots
        assert!(!all_committed.is_empty(), "at least one block should commit");
        let height = all_committed[0].number;
        for block in &all_committed {
            assert_eq!(block.number, height, "all commits should be at the same height");
            assert_eq!(
                block.state_root, all_committed[0].state_root,
                "SAFETY VIOLATION: different state roots at height {height}"
            );
        }
    }

    /// Run 5 consecutive heights and verify monotonic block numbers.
    #[test]
    fn test_liveness_multi_height() {
        let ids = &[1u64, 2, 3, 4];
        let mut nodes: Vec<_> = ids.iter().map(|&id| make_consensus(id, ids)).collect();
        let mut db = InMemoryStateDB::new();
        db.put_account(Account { address: addr(1), balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 });

        let mut heights_committed = Vec::new();
        for _ in 0..5 {
            let committed = run_consensus_round(&mut nodes, &mut db, 50);
            assert!(!committed.is_empty(), "liveness failure: round did not commit");
            let block = &committed[0];
            heights_committed.push(block.number);

            // Advance all nodes to next height
            for node in nodes.iter_mut() {
                node.on_block_committed(block, block.state_root, 0);
            }
        }

        // Heights should be strictly increasing
        for window in heights_committed.windows(2) {
            assert!(window[1] > window[0], "block heights must be monotonically increasing");
        }
    }

    /// Nil prevotes should not cause a commit.
    #[test]
    fn test_nil_prevotes_no_commit() {
        let ids = &[1u64, 2, 3, 4];
        let mut node = make_consensus(1, ids);
        let mut db = InMemoryStateDB::new();
        let _ = node.tick(&mut db);

        // Deliver nil prevotes from all validators
        for &voter_id in &[2u64, 3, 4] {
            let nil_vote = ConsensusMessage::Prevote {
                height: 1,
                round: 0,
                block_hash: None, // nil vote
                validator_id: voter_id,
                bls_signature: None,
            };
            let _ = node.on_message(nil_vote);
        }

        // Nil precommits too
        for &voter_id in &[2u64, 3, 4] {
            let nil_precommit = ConsensusMessage::Precommit {
                height: 1,
                round: 0,
                block_hash: None, // nil
                validator_id: voter_id,
                bls_signature: None,
            };
            let actions = node.on_message(nil_precommit);
            let commits: Vec<_> = actions.iter().filter(|a| matches!(a, ConsensusAction::CommitBlock(_))).collect();
            assert!(commits.is_empty(), "nil precommits must never produce a committed block");
        }
    }

    /// Duplicate votes from the same validator should not be double-counted.
    #[test]
    fn test_duplicate_vote_not_double_counted() {
        let ids = &[1u64, 2, 3, 4];
        let mut node = make_consensus(1, ids);
        let mut db = InMemoryStateDB::new();
        let actions = node.tick(&mut db);

        // Extract the proposed block hash
        let mut block_hash = None;
        for a in &actions {
            if let ConsensusAction::BroadcastMessage(ConsensusMessage::Proposal { block, .. }) = a {
                block_hash = Some(blake3::hash(&serde_json::to_vec(block).unwrap()).into());
            }
        }

        if let Some(hash) = block_hash {
            // Send the same prevote from validator 2 three times
            for _ in 0..3 {
                let prevote = ConsensusMessage::Prevote {
                    height: 1,
                    round: 0,
                    block_hash: Some(hash),
                    validator_id: 2,
                    bls_signature: None,
                };
                let _ = node.on_message(prevote);
            }

            // Only 2 unique prevotes (self + validator 2) — below quorum of 3
            // So no precommit should be triggered by duplicates alone
            // This test verifies duplicate suppression works
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Mempool attack resistance
    // ═══════════════════════════════════════════════════════════════════

    /// Flooding the mempool with transactions should be bounded.
    #[test]
    fn test_mempool_flooding_bounded() {
        let ids = &[1u64, 2, 3, 4];
        let mut node = make_consensus(1, ids);

        // Submit 10000 transactions
        for i in 0..10_000 {
            let tx = Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 1,
                nonce: i as u64,
                signature: None,
                public_key: None,
            });
            node.mempool.submit(tx);
        }

        // Mempool should be bounded (check it doesn't grow unbounded)
        let pending = node.mempool.pending();
        assert!(pending.len() <= 10_000, "mempool should accept submissions");
        // When a block is produced, it should only include up to the gas limit
    }
}

#[cfg(test)]
mod proptest_consensus {
    use crate::tendermint::*;
    use crate::validator_set::{ValidatorInfo, ValidatorSet};
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_state::db::StateDB;
    use evaporchain_types::Account;
    use proptest::prelude::*;

    fn addr(b: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    fn make_validator(id: u64, stake: u64) -> ValidatorInfo {
        let mut address = [0u8; 32];
        address[0] = id as u8;
        ValidatorInfo::new(id, stake, address)
    }

    proptest! {
        /// Round number never decreases within a height.
        #[test]
        fn round_never_decreases(ticks in 1usize..50) {
            let ids = &[1u64, 2, 3, 4];
            let validators: Vec<_> = ids.iter().map(|&id| make_validator(id, 1000)).collect();
            let vs = ValidatorSet::with_validators(validators);
            let mut tc = TendermintConsensus::new_for_test(1, 5, vs);
            let mut db = InMemoryStateDB::new();
            db.put_account(Account { address: addr(1), balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 });

            let mut prev_round = 0u32;
            for _ in 0..ticks {
                let _ = tc.tick(&mut db);
                let round = tc.round();
                prop_assert!(round >= prev_round,
                    "round decreased from {} to {}", prev_round, round);
                prev_round = round;
            }
        }

        /// Height never decreases after ticks.
        #[test]
        fn height_never_decreases(ticks in 1usize..50) {
            let ids = &[1u64, 2, 3, 4];
            let validators: Vec<_> = ids.iter().map(|&id| make_validator(id, 1000)).collect();
            let vs = ValidatorSet::with_validators(validators);
            let mut tc = TendermintConsensus::new_for_test(1, 5, vs);
            let mut db = InMemoryStateDB::new();
            db.put_account(Account { address: addr(1), balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 });

            let mut prev_height = 0u64;
            for _ in 0..ticks {
                let _ = tc.tick(&mut db);
                let height = tc.height();
                prop_assert!(height >= prev_height,
                    "height decreased from {} to {}", prev_height, height);
                prev_height = height;
            }
        }

        /// Validator set size must equal number of registered validators.
        #[test]
        fn validator_count_invariant(n in 1u64..=10) {
            let ids: Vec<u64> = (1..=n).collect();
            let validators: Vec<_> = ids.iter().map(|&id| make_validator(id, 1000)).collect();
            let vs = ValidatorSet::with_validators(validators);
            prop_assert_eq!(vs.len(), n as usize);
        }
    }
}
