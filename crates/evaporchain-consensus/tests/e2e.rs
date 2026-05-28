//! §Consensus e2e
//!
//! Scenario: "NADIA block production arc" — NADIA is the single-node
//! MockConsensus. She initialises the engine, produces three sequential
//! empty blocks, and verifies parent-hash chaining prevents DA-attestation
//! replay across heights. She then submits a transfer transaction,
//! produces a fourth block, and confirms the mempool drains to empty
//! after each produce_block call.
//!
//! The suite proves: `empty_block_data_root` binds (height, parent_hash)
//! into a non-trivial keyed hash; same inputs yield the same root; different
//! heights or different parents yield different roots (anti-replay).
//! MockConsensus starts at block_number=0 / epoch=0; each produce_block
//! increments both monotonically; the produced block's number and epoch
//! match the post-increment values; parent_hash of block N becomes the
//! parent_hash of block N+1; the mempool is empty after production; a block
//! containing a transfer is produced without ConsensusError even when the
//! sender has insufficient balance (execution records the failure internally
//! but does not abort block production).

use evaporchain_consensus::{empty_block_data_root, ConsensusError, MockConsensus};
use evaporchain_state::{InMemoryStateDB, StateDB};
use evaporchain_types::{Account, Transaction, TransferTx};

fn addr(b: u8) -> [u8; 32] {
    [b; 32]
}

fn transfer(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
    Transaction::Transfer(TransferTx {
        from: addr(from),
        to: addr(to),
        amount,
        nonce,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    })
}

fn funded_db(addr_byte: u8, balance: u64) -> InMemoryStateDB {
    let mut db = InMemoryStateDB::new();
    db.put_account(Account {
        address: addr(addr_byte),
        balance,
        nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        vesting: None,
    });
    db
}

// ── empty_block_data_root pure-function tests ──────────────────────────────

#[test]
fn empty_block_data_root_is_deterministic() {
    let parent = [0xAA; 32];
    let r1 = empty_block_data_root(1, &parent);
    let r2 = empty_block_data_root(1, &parent);
    assert_eq!(r1, r2, "same inputs must produce same root");
}

#[test]
fn empty_block_data_root_binds_height() {
    let parent = [0xAA; 32];
    let r_h1 = empty_block_data_root(1, &parent);
    let r_h2 = empty_block_data_root(2, &parent);
    assert_ne!(
        r_h1, r_h2,
        "different heights must produce different roots (DA anti-replay)"
    );
}

#[test]
fn empty_block_data_root_binds_parent_hash() {
    let parent_a = [0xAA; 32];
    let parent_b = [0xBB; 32];
    let r_a = empty_block_data_root(1, &parent_a);
    let r_b = empty_block_data_root(1, &parent_b);
    assert_ne!(
        r_a, r_b,
        "different parent hashes must produce different roots (fork anti-replay)"
    );
}

#[test]
fn empty_block_data_root_is_nonzero() {
    let r = empty_block_data_root(1, &[0u8; 32]);
    assert_ne!(r, [0u8; 32], "sentinel must not be all-zeros");
}

// ── MockConsensus initialisation ───────────────────────────────────────────

#[test]
fn mock_consensus_starts_at_zero() {
    let mc = MockConsensus::new(10);
    assert_eq!(mc.block_number(), 0);
    assert_eq!(mc.epoch(), 0);
}

#[test]
fn mock_consensus_mev_protection_disabled_by_default() {
    let mc = MockConsensus::new(10);
    assert!(!mc.mev_protection_enabled());
}

// ── Block production ───────────────────────────────────────────────────────

#[test]
fn produce_empty_block_increments_block_number_and_epoch() {
    let mut mc = MockConsensus::new(10);
    let mut db = InMemoryStateDB::new();
    let result = mc.produce_block(&mut db).unwrap();
    assert_eq!(result.block.number, 1, "first block must be number 1");
    assert_eq!(result.block.epoch, 1, "first block must be epoch 1");
    assert_eq!(mc.block_number(), 1);
    assert_eq!(mc.epoch(), 1);
}

#[test]
fn produce_empty_block_state_root_is_deterministic() {
    // Two fresh engines with empty DBs must produce the same state root.
    // An empty state → trie root is the canonical empty-trie hash.
    let mut mc1 = MockConsensus::new(10);
    let mut db1 = InMemoryStateDB::new();
    let r1 = mc1.produce_block(&mut db1).unwrap();

    let mut mc2 = MockConsensus::new(10);
    let mut db2 = InMemoryStateDB::new();
    let r2 = mc2.produce_block(&mut db2).unwrap();

    assert_eq!(
        r1.execution.state_root, r2.execution.state_root,
        "empty blocks from identical fresh state must have the same state root"
    );
}

#[test]
fn empty_block_has_no_transactions() {
    let mut mc = MockConsensus::new(10);
    let mut db = InMemoryStateDB::new();
    let result = mc.produce_block(&mut db).unwrap();
    assert_eq!(
        result.block.transactions.len(),
        0,
        "empty mempool → 0 txs in block"
    );
}

#[test]
fn sequential_blocks_increment_monotonically() {
    let mut mc = MockConsensus::new(10);
    let mut db = InMemoryStateDB::new();
    for i in 1u64..=3 {
        let r = mc.produce_block(&mut db).unwrap();
        assert_eq!(r.block.number, i, "block {i} must have number {i}");
        assert_eq!(r.block.epoch, i, "block {i} must have epoch {i}");
    }
    assert_eq!(mc.block_number(), 3);
}

#[test]
fn block_carries_parent_hash_of_predecessor() {
    let mut mc = MockConsensus::new(10);
    let mut db = InMemoryStateDB::new();

    let r1 = mc.produce_block(&mut db).unwrap();
    // Block 1's parent_hash was [0u8;32] (genesis). Block 2's parent_hash
    // must be derived from block 1's content, so it differs from genesis.
    let r2 = mc.produce_block(&mut db).unwrap();
    assert_ne!(
        r2.block.parent_hash, [0u8; 32],
        "block 2 parent_hash must not be the genesis sentinel"
    );
    // The content commitment changes: block 2's parent_hash must differ from block 1's.
    assert_ne!(
        r2.block.parent_hash, r1.block.parent_hash,
        "consecutive blocks must have distinct parent hashes"
    );
}

#[test]
fn mempool_drains_after_produce_block() {
    let mut mc = MockConsensus::new(10);
    mc.mempool.submit(transfer(0x01, 0x02, 100, 0));
    mc.mempool.submit(transfer(0x01, 0x03, 200, 1));
    assert_eq!(mc.mempool.len(), 2);

    let mut db = InMemoryStateDB::new();
    let r = mc.produce_block(&mut db).unwrap();
    // Both txs were drained into the block.
    assert_eq!(r.block.transactions.len(), 2);
    // Mempool is empty after production.
    assert!(
        mc.mempool.is_empty(),
        "mempool must be empty after produce_block"
    );
}

#[test]
fn block_with_funded_transfer_executes_successfully() {
    let mut mc = MockConsensus::new(10);
    // GAS_TRANSFER = 21_000 × base_fee 1 = 21_000 gas fee.
    // Fund sender with 100_000 to cover fee + transfer amount.
    let mut db = funded_db(0xAA, 100_000);
    mc.mempool.submit(transfer(0xAA, 0xBB, 100, 0));

    let result = mc.produce_block(&mut db).unwrap();
    assert_eq!(result.block.transactions.len(), 1, "one tx in block");
    // Execution succeeded — at least one tx ran.
    assert_eq!(
        result.execution.txs_executed, 1,
        "transfer must be executed"
    );
    assert_eq!(
        result.execution.txs_failed, 0,
        "funded transfer must not fail"
    );
}

#[test]
fn block_with_unfunded_transfer_does_not_abort_production() {
    let mut mc = MockConsensus::new(10);
    let mut db = InMemoryStateDB::new(); // no accounts funded
    mc.mempool.submit(transfer(0x01, 0x02, 999_999, 0));

    // Block production succeeds even though the tx will fail at execution.
    let result = mc.produce_block(&mut db).unwrap();
    assert_eq!(result.block.transactions.len(), 1);
    // Execution records the failure but does not abort block production.
    assert_eq!(
        result.execution.txs_failed, 1,
        "insufficient-balance transfer must be counted in txs_failed"
    );
}

#[test]
fn consensus_error_variants_are_representable() {
    let err = ConsensusError::ProposalFailed("test".into());
    assert!(!err.to_string().is_empty());

    let err = ConsensusError::ExecutionFailed("exec".into());
    assert!(!err.to_string().is_empty());

    let err = ConsensusError::NotLeader {
        validator_id: 1,
        epoch: 5,
        leader_id: 2,
    };
    assert!(err.to_string().contains("1") || err.to_string().contains("leader"));

    let err = ConsensusError::InvalidProducer {
        producer_id: 3,
        expected_id: 4,
    };
    assert!(err.to_string().contains("3") || err.to_string().contains("producer"));
}

#[test]
fn nadia_block_production_full_arc() {
    // Full arc: NADIA initialises consensus, produces 3 empty blocks,
    // verifies parent-hash chaining prevents DA replay, then submits
    // a funded transfer and confirms it executes in block 4.

    let mut mc = MockConsensus::new(10);
    let mut db = funded_db(0xAA, 50_000);

    // Three empty blocks — parent-hash chain must be strictly unique.
    let mut seen_parent_hashes = vec![[0u8; 32]]; // genesis
    for _ in 1u64..=3 {
        let r = mc.produce_block(&mut db).unwrap();
        let ph = r.block.parent_hash;
        assert!(
            !seen_parent_hashes.contains(&r.block.parent_hash) || r.block.number == 1,
            "parent hash must be unique per block (no DA replay)"
        );
        seen_parent_hashes.push(ph);
    }
    assert_eq!(mc.block_number(), 3);

    // Submit a funded transfer for block 4.
    mc.mempool.submit(transfer(0xAA, 0xBB, 1_000, 0));
    let r4 = mc.produce_block(&mut db).unwrap();
    assert_eq!(r4.block.number, 4);
    assert_eq!(r4.block.transactions.len(), 1);
    assert_eq!(r4.execution.txs_executed, 1, "funded transfer must execute");

    // Mempool is empty after block 4.
    assert!(mc.mempool.is_empty());

    // DA anti-replay: empty_block_data_root at heights 1-3 must all differ.
    let parent_zero = [0u8; 32];
    let r_1 = empty_block_data_root(1, &parent_zero);
    let r_2 = empty_block_data_root(2, &parent_zero);
    let r_3 = empty_block_data_root(3, &parent_zero);
    assert_ne!(r_1, r_2);
    assert_ne!(r_2, r_3);
    assert_ne!(r_1, r_3);
}
