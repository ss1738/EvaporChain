//! Executor-parity test harness.
//!
//! Closes AUDIT_2026_05_13.md Theme B structurally: every `Transaction`
//! variant + initial state combination must produce byte-identical
//! post-state under `SimpleExecutor` and `ParallelExecutor`. A divergence
//! is either a `SimpleExecutor` bug, a `ParallelExecutor` bug, or both —
//! each becomes a finding.
//!
//! Multi-phase arc (see `docs/plans/EXECUTOR_PARITY_PLAN.md`):
//! - Phase 1 *(this PR)*: scaffolding + Transfer baseline.
//! - Phase 2: ValidatorStake / Exit / ClaimStake — closes audit C3.
//! - Phase 3: 7 blackholed tx types — closes audit C4.
//! - Phase 4: Privacy (Shield / Unshield / PrivateTransfer).
//! - Phase 5: Contract + tail variants.
//! - Phase 6: Adversarial fixtures + governance-flag flip matrix.
//!
//! Phase 1 comparator covers `accounts` and the `execute_block` return
//! disposition (success / error). Later phases extend the comparator to
//! `stakes`, `delegations`, `sentinel_*`, `governance_*`, `objects`,
//! `ghosts`, `spent_nullifiers`, `note_commitments`, `state_root`.

use evaporchain_execution::parallel::ParallelExecutor;
use evaporchain_execution::{ExecutionEngine, SimpleExecutor};
use evaporchain_state::db::{InMemoryStateDB, StateDB};
use evaporchain_types::{Account, Block, Epoch, Transaction, TransferTx};

// ─── Public harness types ──────────────────────────────────────────────

/// A single parity scenario the harness can run.
pub struct ParityFixture {
    pub name: &'static str,
    pub seed: fn(&mut InMemoryStateDB),
    pub transaction: Transaction,
    pub block_number: u64,
    pub epoch: Epoch,
}

/// Single divergence between the two executors on one comparator domain.
#[derive(Debug)]
pub struct Divergence {
    pub domain: &'static str,
    pub detail: String,
    pub simple_value: String,
    pub parallel_value: String,
}

// ─── Helpers ────────────────────────────────────────────────────────────

fn addr(byte: u8) -> [u8; 32] {
    let mut a = [0u8; 32];
    a[0] = byte;
    a
}

fn fund(db: &mut InMemoryStateDB, byte: u8, balance: u64) {
    db.put_account(Account {
        address: addr(byte),
        balance,
        nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        last_touched_epoch: 0,
        vesting: None,
    });
}

fn make_block(number: u64, epoch: Epoch, tx: Transaction) -> Block {
    Block {
        number,
        epoch,
        parent_hash: [0u8; 32],
        state_root: [0u8; 32],
        transactions: vec![tx],
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

// ─── Comparator domains (Phase 1: accounts + disposition only) ─────────

fn compare_accounts(
    simple: &InMemoryStateDB,
    parallel: &InMemoryStateDB,
    out: &mut Vec<Divergence>,
) {
    let mut all_addrs: std::collections::BTreeSet<[u8; 32]> =
        simple.all_account_addresses().into_iter().collect();
    for a in parallel.all_account_addresses() {
        all_addrs.insert(a);
    }

    for a in all_addrs {
        let s = simple.get_account(&a);
        let p = parallel.get_account(&a);
        match (s, p) {
            (None, None) => {}
            (Some(sa), Some(pa)) => {
                if sa.balance != pa.balance {
                    out.push(Divergence {
                        domain: "accounts.balance",
                        detail: format!("addr={:02x}…", a[0]),
                        simple_value: sa.balance.to_string(),
                        parallel_value: pa.balance.to_string(),
                    });
                }
                if sa.nonce != pa.nonce {
                    out.push(Divergence {
                        domain: "accounts.nonce",
                        detail: format!("addr={:02x}…", a[0]),
                        simple_value: sa.nonce.to_string(),
                        parallel_value: pa.nonce.to_string(),
                    });
                }
                if sa.last_touched_epoch != pa.last_touched_epoch {
                    out.push(Divergence {
                        domain: "accounts.last_touched_epoch",
                        detail: format!("addr={:02x}…", a[0]),
                        simple_value: sa.last_touched_epoch.to_string(),
                        parallel_value: pa.last_touched_epoch.to_string(),
                    });
                }
            }
            (Some(_), None) => out.push(Divergence {
                domain: "accounts.presence",
                detail: format!("addr={:02x}… exists only in SimpleExecutor DB", a[0]),
                simple_value: "Some(_)".into(),
                parallel_value: "None".into(),
            }),
            (None, Some(_)) => out.push(Divergence {
                domain: "accounts.presence",
                detail: format!("addr={:02x}… exists only in ParallelExecutor DB", a[0]),
                simple_value: "None".into(),
                parallel_value: "Some(_)".into(),
            }),
        }
    }
}

// ─── Harness entry point ───────────────────────────────────────────────

/// Run a fixture through both executors, returning every divergence found.
/// Empty Vec means full parity on the comparator domains active in Phase 1.
pub fn run_parity(fixture: &ParityFixture) -> Vec<Divergence> {
    let mut divergences: Vec<Divergence> = Vec::new();

    // Two independent DBs seeded identically.
    let mut simple_db = InMemoryStateDB::new();
    let mut parallel_db = InMemoryStateDB::new();
    (fixture.seed)(&mut simple_db);
    (fixture.seed)(&mut parallel_db);

    let block = make_block(fixture.block_number, fixture.epoch, fixture.transaction.clone());

    let mut simple = SimpleExecutor::new_for_test(7);
    let mut parallel = ParallelExecutor::new_for_test(7);

    let simple_result = simple.execute_block(&mut simple_db, &block);
    let parallel_result = parallel.execute_block(&mut parallel_db, &block);

    // Disposition parity: both must succeed or both must fail with the same
    // error variant. Phase 1 compares by debug-string; later phases may
    // tighten this to error-variant matching.
    match (&simple_result, &parallel_result) {
        (Ok(s), Ok(p)) => {
            if s.txs_executed != p.txs_executed {
                divergences.push(Divergence {
                    domain: "result.txs_executed",
                    detail: fixture.name.into(),
                    simple_value: s.txs_executed.to_string(),
                    parallel_value: p.txs_executed.to_string(),
                });
            }
            if s.txs_failed != p.txs_failed {
                divergences.push(Divergence {
                    domain: "result.txs_failed",
                    detail: fixture.name.into(),
                    simple_value: s.txs_failed.to_string(),
                    parallel_value: p.txs_failed.to_string(),
                });
            }
        }
        (Err(s), Err(p)) => {
            // Both errored: only flag if the error variant differs.
            let sd = format!("{:?}", s);
            let pd = format!("{:?}", p);
            if sd != pd {
                divergences.push(Divergence {
                    domain: "result.error_variant",
                    detail: fixture.name.into(),
                    simple_value: sd,
                    parallel_value: pd,
                });
            }
        }
        (Ok(_), Err(p)) => divergences.push(Divergence {
            domain: "result.disposition",
            detail: fixture.name.into(),
            simple_value: "Ok(_)".into(),
            parallel_value: format!("Err({:?})", p),
        }),
        (Err(s), Ok(_)) => divergences.push(Divergence {
            domain: "result.disposition",
            detail: fixture.name.into(),
            simple_value: format!("Err({:?})", s),
            parallel_value: "Ok(_)".into(),
        }),
    }

    compare_accounts(&simple_db, &parallel_db, &mut divergences);

    divergences
}

/// Assert full parity. Panics with the full divergence list — one run
/// surfaces every misalignment, not just the first.
pub fn assert_parity(fixture: &ParityFixture) {
    let divergences = run_parity(fixture);
    assert!(
        divergences.is_empty(),
        "parity failure on fixture `{}`:\n{}",
        fixture.name,
        divergences
            .iter()
            .map(|d| format!(
                "  · {}: {}\n      Simple   = {}\n      Parallel = {}",
                d.domain, d.detail, d.simple_value, d.parallel_value
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ─── Phase 1: Transfer baseline fixtures ───────────────────────────────

#[test]
fn parity_transfer_happy_path() {
    let fixture = ParityFixture {
        name: "transfer-happy-path",
        seed: |db| fund(db, 1, 1000),
        transaction: Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 300,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_transfer_insufficient_balance() {
    let fixture = ParityFixture {
        name: "transfer-insufficient-balance",
        seed: |db| fund(db, 1, 100),
        transaction: Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 500,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_transfer_zero_amount() {
    // Zero-amount transfer is a no-op-with-fee — useful baseline because
    // both executors handle the "tx accepted, state unchanged except fee"
    // path identically.
    let fixture = ParityFixture {
        name: "transfer-zero-amount",
        seed: |db| {
            fund(db, 1, 1000);
            fund(db, 2, 0);
        },
        transaction: Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 0,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}
