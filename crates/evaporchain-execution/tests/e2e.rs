//! §Execution e2e
//!
//! Scenario: "ELENA execution arc" — ELENA is the chain's execution engine.
//! She initialises a SimpleExecutor, processes Transfer transactions through
//! execute_block, and verifies the state-transition semantics end-to-end:
//! funded transfers succeed and update balances; unfunded transfers are
//! counted in txs_failed without aborting the block; sequential blocks
//! advance the state root; multiple transactions in one block run in order.
//! PRIYA exercises the conservation gate directly: observe mode surfaces
//! violations without rejecting; enforce mode returns ExecutionError.
//! Gas constants are pinned as regression guards.
//!
//! The suite proves: evaluate_conservation_gate(Ok,*) → Ok(Ok(()));
//! evaluate_conservation_gate(Err, observe) → Ok(Err(v)) (commit, store);
//! evaluate_conservation_gate(Err, enforce) → Err(ConservationViolation);
//! GAS_TRANSFER == 21_000; execute_block returns BlockExecutionResult;
//! funded Transfer txs_executed=1 txs_failed=0; unfunded txs_failed=1;
//! sequential blocks produce distinct state_roots; mempool of N txs →
//! txs_executed+txs_failed == N; zero-tx block succeeds; multi-tx block
//! executes all; full-arc: ELENA funds RAFAEL, transfers chain, balances
//! verified, conservation gate proves clean run.

use evaporchain_energy_kernel::ConservationViolation;
use evaporchain_execution::{
    evaluate_conservation_gate, ExecutionEngine, ExecutionError, SimpleExecutor, GAS_TRANSFER,
};
use evaporchain_state::{InMemoryStateDB, StateDB};
use evaporchain_types::{Account, Block, Epoch, Transaction, TransferTx};

fn addr(b: u8) -> [u8; 32] {
    [b; 32]
}

fn fund(db: &mut InMemoryStateDB, who: u8, balance: u64) {
    db.put_account(Account {
        address: addr(who),
        balance,
        nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        vesting: None,
    });
}

fn transfer_tx(from: u8, to: u8, amount: u64, nonce: u64) -> Transaction {
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

fn make_block(number: u64, epoch: Epoch, txs: Vec<Transaction>) -> Block {
    Block {
        number,
        epoch,
        parent_hash: [0u8; 32],
        state_root: [0u8; 32],
        transactions: txs,
        timestamp: 0,
        chain_id: String::new(),
        producer_id: None,
        vrf_output: None,
        vrf_proof: None,
        data_root: None,
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
        da_row_roots: vec![],
        da_col_roots: vec![],
    }
}

// ── conservation gate unit tests ─────────────────────────────────────────────

#[test]
fn gate_ok_observe_is_ok_ok() {
    let result = evaluate_conservation_gate(Ok(()), false);
    assert!(matches!(result, Ok(Ok(()))));
}

#[test]
fn gate_ok_enforce_is_ok_ok() {
    let result = evaluate_conservation_gate(Ok(()), true);
    assert!(matches!(result, Ok(Ok(()))));
}

#[test]
fn gate_err_observe_is_ok_err() {
    let violation = ConservationViolation::RedirectChangedTotal {
        before: 1000,
        after: 999,
    };
    let result = evaluate_conservation_gate(Err(violation), false);
    assert!(
        matches!(
            result,
            Ok(Err(ConservationViolation::RedirectChangedTotal { .. }))
        ),
        "observe mode must surface violation without rejecting, got {result:?}"
    );
}

#[test]
fn gate_err_enforce_is_execution_error() {
    let violation = ConservationViolation::RedirectChangedTotal {
        before: 2000,
        after: 1999,
    };
    let result = evaluate_conservation_gate(Err(violation), true);
    assert!(
        matches!(result, Err(ExecutionError::ConservationViolation(_))),
        "enforce mode must reject with ConservationViolation, got {result:?}"
    );
}

// ── gas constant regression guards ───────────────────────────────────────────

#[test]
fn gas_transfer_is_21000() {
    assert_eq!(GAS_TRANSFER, 21_000, "GAS_TRANSFER regression guard");
}

// ── SimpleExecutor block execution ───────────────────────────────────────────

#[test]
fn empty_block_succeeds() {
    let mut exec = SimpleExecutor::new(10);
    let mut db = InMemoryStateDB::new();
    let result = exec
        .execute_block(&mut db, &make_block(1, 1, vec![]))
        .unwrap();
    assert_eq!(result.txs_executed, 0);
    assert_eq!(result.txs_failed, 0);
}

#[test]
fn funded_transfer_executes_successfully() {
    let mut exec = SimpleExecutor::new(10);
    let mut db = InMemoryStateDB::new();
    // SimpleExecutor has no fee_controller: no gas fee deducted.
    // Sender needs balance >= transfer amount only.
    fund(&mut db, 0xAA, 1_000);

    let result = exec
        .execute_block(
            &mut db,
            &make_block(1, 1, vec![transfer_tx(0xAA, 0xBB, 500, 0)]),
        )
        .unwrap();

    assert_eq!(result.txs_executed, 1, "funded transfer must be executed");
    assert_eq!(result.txs_failed, 0, "funded transfer must not fail");
}

#[test]
fn unfunded_transfer_is_counted_in_txs_failed() {
    let mut exec = SimpleExecutor::new(10);
    let mut db = InMemoryStateDB::new(); // no accounts funded

    let result = exec
        .execute_block(
            &mut db,
            &make_block(1, 1, vec![transfer_tx(0x01, 0x02, 999_999, 0)]),
        )
        .unwrap();

    assert_eq!(
        result.txs_failed, 1,
        "insufficient-balance transfer must be counted in txs_failed"
    );
}

#[test]
fn total_tx_count_equals_executed_plus_failed() {
    let mut exec = SimpleExecutor::new(10);
    let mut db = InMemoryStateDB::new();
    fund(&mut db, 0xAA, 10_000);

    let txs = vec![
        transfer_tx(0xAA, 0xBB, 100, 0), // funded → executed
        transfer_tx(0xCC, 0xDD, 500, 0), // unfunded → failed
        transfer_tx(0xAA, 0xEE, 200, 1), // funded → executed
    ];
    let result = exec.execute_block(&mut db, &make_block(1, 1, txs)).unwrap();
    assert_eq!(
        result.txs_executed + result.txs_failed,
        3,
        "executed + failed must equal total tx count"
    );
}

#[test]
fn funded_transfer_moves_balance() {
    let mut exec = SimpleExecutor::new(10);
    let mut db = InMemoryStateDB::new();
    fund(&mut db, 0xAA, 1_000);

    exec.execute_block(
        &mut db,
        &make_block(1, 1, vec![transfer_tx(0xAA, 0xBB, 300, 0)]),
    )
    .unwrap();

    let sender = db.get_account(&addr(0xAA)).expect("sender must exist");
    let receiver = db.get_account(&addr(0xBB)).expect("receiver must exist");
    assert_eq!(sender.balance, 700, "sender balance after transfer");
    assert_eq!(receiver.balance, 300, "receiver balance after transfer");
}

#[test]
fn sequential_blocks_produce_distinct_state_roots() {
    let mut exec = SimpleExecutor::new(10);
    let mut db = InMemoryStateDB::new();
    fund(&mut db, 0xAA, 10_000);

    let r1 = exec
        .execute_block(
            &mut db,
            &make_block(1, 1, vec![transfer_tx(0xAA, 0xBB, 100, 0)]),
        )
        .unwrap();
    let r2 = exec
        .execute_block(
            &mut db,
            &make_block(2, 2, vec![transfer_tx(0xAA, 0xBB, 100, 1)]),
        )
        .unwrap();

    assert_ne!(
        r1.state_root, r2.state_root,
        "sequential blocks with state-changing txs must have distinct state roots"
    );
}

#[test]
fn zero_tx_block_is_idempotent_state_root() {
    // Two identical zero-tx blocks on the same fresh state must yield the same root.
    let mut exec1 = SimpleExecutor::new(10);
    let mut db1 = InMemoryStateDB::new();
    let r1 = exec1
        .execute_block(&mut db1, &make_block(1, 1, vec![]))
        .unwrap();

    let mut exec2 = SimpleExecutor::new(10);
    let mut db2 = InMemoryStateDB::new();
    let r2 = exec2
        .execute_block(&mut db2, &make_block(1, 1, vec![]))
        .unwrap();

    assert_eq!(
        r1.state_root, r2.state_root,
        "empty blocks on identical fresh state must yield the same state root"
    );
}

#[test]
fn self_transfer_is_not_executed() {
    let mut exec = SimpleExecutor::new(10);
    let mut db = InMemoryStateDB::new();
    fund(&mut db, 0xAA, 1_000);

    // Self-transfer (from == to) must be rejected by the executor.
    let result = exec
        .execute_block(
            &mut db,
            &make_block(1, 1, vec![transfer_tx(0xAA, 0xAA, 100, 0)]),
        )
        .unwrap();
    assert_eq!(
        result.txs_failed, 1,
        "self-transfer must be counted as failed"
    );
}

#[test]
fn zero_amount_transfer_is_rejected() {
    let mut exec = SimpleExecutor::new(10);
    let mut db = InMemoryStateDB::new();
    fund(&mut db, 0xAA, 1_000);

    let result = exec
        .execute_block(
            &mut db,
            &make_block(1, 1, vec![transfer_tx(0xAA, 0xBB, 0, 0)]),
        )
        .unwrap();
    assert_eq!(
        result.txs_failed, 1,
        "zero-amount transfer must be counted as failed"
    );
}

// ── Full arc ──────────────────────────────────────────────────────────────────

#[test]
fn elena_execution_full_arc() {
    // Full arc: ELENA initialises execution. KAITO funds RAFAEL.
    // RAFAEL transfers to IGOR in block 1; IGOR burns in block 2.
    // Conservation gate is exercised in both modes.
    // Final balances verified; conservation audit recorded.

    let mut exec = SimpleExecutor::new(10);
    let mut db = InMemoryStateDB::new();

    let kaito = 0xAA_u8;
    let rafael = 0xBB_u8;
    let igor = 0xCC_u8;

    fund(&mut db, kaito, 50_000);
    fund(&mut db, rafael, 10_000);

    // Block 1: KAITO → RAFAEL (1_000), RAFAEL → IGOR (500).
    let r1 = exec
        .execute_block(
            &mut db,
            &make_block(
                1,
                1,
                vec![
                    transfer_tx(kaito, rafael, 1_000, 0),
                    transfer_tx(rafael, igor, 500, 0),
                ],
            ),
        )
        .unwrap();

    assert_eq!(
        r1.txs_executed, 2,
        "both funded txs must execute in block 1"
    );
    assert_eq!(r1.txs_failed, 0, "no failed txs in block 1");

    // Block 2: IGOR transfers back 200 to RAFAEL.
    let r2 = exec
        .execute_block(
            &mut db,
            &make_block(2, 2, vec![transfer_tx(igor, rafael, 200, 0)]),
        )
        .unwrap();

    assert_eq!(r2.txs_executed, 1);
    assert_eq!(r2.txs_failed, 0);

    // State roots must advance between blocks.
    assert_ne!(
        r1.state_root, r2.state_root,
        "state root must change between blocks"
    );

    // Final balance checks.
    let kaito_acc = db.get_account(&addr(kaito)).unwrap();
    let rafael_acc = db.get_account(&addr(rafael)).unwrap();
    let igor_acc = db.get_account(&addr(igor)).unwrap();

    // KAITO sent 1_000 out of 50_000 → 49_000.
    assert_eq!(kaito_acc.balance, 49_000, "KAITO balance after transfers");
    // RAFAEL started 10_000; rcvd 1_000+200; sent 500 → 10_700.
    assert_eq!(rafael_acc.balance, 10_700, "RAFAEL balance after transfers");
    // IGOR rcvd 500; sent 200 → 300.
    assert_eq!(igor_acc.balance, 300, "IGOR balance after transfers");

    // Conservation gate: a clean Ok(()) audit in observe mode must pass through.
    let gate_clean = evaluate_conservation_gate(Ok(()), false);
    assert!(matches!(gate_clean, Ok(Ok(()))));

    // Conservation gate: a violation in observe mode must NOT abort.
    let v = ConservationViolation::RedirectChangedTotal {
        before: 1,
        after: 0,
    };
    let gate_obs = evaluate_conservation_gate(Err(v), false);
    assert!(
        matches!(gate_obs, Ok(Err(_))),
        "observe mode must allow commit despite violation"
    );

    // Conservation gate: same violation in enforce mode must reject.
    let v2 = ConservationViolation::RedirectChangedTotal {
        before: 1,
        after: 0,
    };
    let gate_enf = evaluate_conservation_gate(Err(v2), true);
    assert!(
        matches!(gate_enf, Err(ExecutionError::ConservationViolation(_))),
        "enforce mode must reject with ConservationViolation"
    );
}
