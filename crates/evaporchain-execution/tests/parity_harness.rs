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
use evaporchain_types::{
    Account, Block, ClaimDelegationTx, DelegateTx, Epoch, GovernanceAction, GovernanceTx,
    MultiSigTx, StakeRecord, Transaction, TransferTx, UndelegateTx, ValidatorStakeTx,
};

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

fn compare_stakes(
    simple: &InMemoryStateDB,
    parallel: &InMemoryStateDB,
    out: &mut Vec<Divergence>,
) {
    let mut all_ids: std::collections::BTreeSet<u64> = simple
        .all_stakes()
        .iter()
        .map(|s| s.validator_id)
        .collect();
    for s in parallel.all_stakes() {
        all_ids.insert(s.validator_id);
    }

    for vid in all_ids {
        let s = simple.get_stake(vid);
        let p = parallel.get_stake(vid);
        match (s, p) {
            (None, None) => {}
            (Some(ss), Some(ps)) => {
                if ss.staked_amount != ps.staked_amount {
                    out.push(Divergence {
                        domain: "stakes.staked_amount",
                        detail: format!("validator_id={vid}"),
                        simple_value: ss.staked_amount.to_string(),
                        parallel_value: ps.staked_amount.to_string(),
                    });
                }
                if ss.staked_at_epoch != ps.staked_at_epoch {
                    out.push(Divergence {
                        domain: "stakes.staked_at_epoch",
                        detail: format!("validator_id={vid}"),
                        simple_value: ss.staked_at_epoch.to_string(),
                        parallel_value: ps.staked_at_epoch.to_string(),
                    });
                }
                if ss.unbonding_epoch != ps.unbonding_epoch {
                    out.push(Divergence {
                        domain: "stakes.unbonding_epoch",
                        detail: format!("validator_id={vid}"),
                        simple_value: format!("{:?}", ss.unbonding_epoch),
                        parallel_value: format!("{:?}", ps.unbonding_epoch),
                    });
                }
                if ss.slashed_amount != ps.slashed_amount {
                    out.push(Divergence {
                        domain: "stakes.slashed_amount",
                        detail: format!("validator_id={vid}"),
                        simple_value: ss.slashed_amount.to_string(),
                        parallel_value: ps.slashed_amount.to_string(),
                    });
                }
                if ss.validator_address != ps.validator_address {
                    out.push(Divergence {
                        domain: "stakes.validator_address",
                        detail: format!("validator_id={vid}"),
                        simple_value: format!("{:02x}…", ss.validator_address[0]),
                        parallel_value: format!("{:02x}…", ps.validator_address[0]),
                    });
                }
            }
            (Some(_), None) => out.push(Divergence {
                domain: "stakes.presence",
                detail: format!("validator_id={vid} exists only in SimpleExecutor DB"),
                simple_value: "Some(_)".into(),
                parallel_value: "None".into(),
            }),
            (None, Some(_)) => out.push(Divergence {
                domain: "stakes.presence",
                detail: format!("validator_id={vid} exists only in ParallelExecutor DB"),
                simple_value: "None".into(),
                parallel_value: "Some(_)".into(),
            }),
        }
    }
}

fn compare_delegations(
    simple: &InMemoryStateDB,
    parallel: &InMemoryStateDB,
    out: &mut Vec<Divergence>,
) {
    let mut all_keys: std::collections::BTreeSet<([u8; 32], u64)> = simple
        .all_delegations()
        .iter()
        .map(|d| (d.delegator, d.validator_id))
        .collect();
    for d in parallel.all_delegations() {
        all_keys.insert((d.delegator, d.validator_id));
    }

    for (delegator, vid) in all_keys {
        let s = simple.get_delegation(&delegator, vid);
        let p = parallel.get_delegation(&delegator, vid);
        match (s, p) {
            (None, None) => {}
            (Some(sd), Some(pd)) => {
                if sd.amount != pd.amount {
                    out.push(Divergence {
                        domain: "delegations.amount",
                        detail: format!("delegator={:02x}… vid={vid}", delegator[0]),
                        simple_value: sd.amount.to_string(),
                        parallel_value: pd.amount.to_string(),
                    });
                }
                if sd.delegated_at_epoch != pd.delegated_at_epoch {
                    out.push(Divergence {
                        domain: "delegations.delegated_at_epoch",
                        detail: format!("delegator={:02x}… vid={vid}", delegator[0]),
                        simple_value: sd.delegated_at_epoch.to_string(),
                        parallel_value: pd.delegated_at_epoch.to_string(),
                    });
                }
                if sd.unbonding_amount != pd.unbonding_amount {
                    out.push(Divergence {
                        domain: "delegations.unbonding_amount",
                        detail: format!("delegator={:02x}… vid={vid}", delegator[0]),
                        simple_value: sd.unbonding_amount.to_string(),
                        parallel_value: pd.unbonding_amount.to_string(),
                    });
                }
                if sd.unbonding_epoch != pd.unbonding_epoch {
                    out.push(Divergence {
                        domain: "delegations.unbonding_epoch",
                        detail: format!("delegator={:02x}… vid={vid}", delegator[0]),
                        simple_value: format!("{:?}", sd.unbonding_epoch),
                        parallel_value: format!("{:?}", pd.unbonding_epoch),
                    });
                }
            }
            (Some(_), None) => out.push(Divergence {
                domain: "delegations.presence",
                detail: format!(
                    "delegator={:02x}… vid={vid} exists only in SimpleExecutor DB",
                    delegator[0]
                ),
                simple_value: "Some(_)".into(),
                parallel_value: "None".into(),
            }),
            (None, Some(_)) => out.push(Divergence {
                domain: "delegations.presence",
                detail: format!(
                    "delegator={:02x}… vid={vid} exists only in ParallelExecutor DB",
                    delegator[0]
                ),
                simple_value: "None".into(),
                parallel_value: "Some(_)".into(),
            }),
        }
    }
}

fn compare_proposals(
    simple: &InMemoryStateDB,
    parallel: &InMemoryStateDB,
    out: &mut Vec<Divergence>,
) {
    let mut all_ids: std::collections::BTreeSet<u64> = simple
        .all_proposals()
        .iter()
        .map(|p| p.proposal_id)
        .collect();
    for p in parallel.all_proposals() {
        all_ids.insert(p.proposal_id);
    }

    for id in all_ids {
        let s = simple.get_proposal(id);
        let p = parallel.get_proposal(id);
        match (s, p) {
            (None, None) => {}
            (Some(sp), Some(pp)) => {
                if sp.title != pp.title
                    || sp.param_key != pp.param_key
                    || sp.param_value != pp.param_value
                    || sp.start_epoch != pp.start_epoch
                    || sp.end_epoch != pp.end_epoch
                    || sp.votes_for != pp.votes_for
                    || sp.votes_against != pp.votes_against
                    || sp.status != pp.status
                    || sp.voters != pp.voters
                {
                    out.push(Divergence {
                        domain: "governance.proposal_content",
                        detail: format!("proposal_id={id}"),
                        simple_value: format!(
                            "title={:?} status={:?} for={} against={} voters={}",
                            sp.title,
                            sp.status,
                            sp.votes_for,
                            sp.votes_against,
                            sp.voters.len()
                        ),
                        parallel_value: format!(
                            "title={:?} status={:?} for={} against={} voters={}",
                            pp.title,
                            pp.status,
                            pp.votes_for,
                            pp.votes_against,
                            pp.voters.len()
                        ),
                    });
                }
            }
            (Some(_), None) => out.push(Divergence {
                domain: "governance.proposal_presence",
                detail: format!("proposal_id={id} exists only in SimpleExecutor DB"),
                simple_value: "Some(_)".into(),
                parallel_value: "None".into(),
            }),
            (None, Some(_)) => out.push(Divergence {
                domain: "governance.proposal_presence",
                detail: format!("proposal_id={id} exists only in ParallelExecutor DB"),
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
    compare_stakes(&simple_db, &parallel_db, &mut divergences);
    compare_delegations(&simple_db, &parallel_db, &mut divergences);
    compare_proposals(&simple_db, &parallel_db, &mut divergences);

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

// ─── Phase 2: ValidatorStake fixtures (closes audit C3) ─────────────────

#[test]
fn parity_validator_stake_happy_path() {
    // AUDIT_2026_05_13 C3 surfaces here: pre-fix, ParallelExecutor's
    // `exec_validator_stake` debited the staker's balance via the
    // overlay but never wrote a StakeRecord (overlay put_stake was `{}`).
    // SimpleExecutor's execute_validator_stake correctly persists the
    // record. The divergence shows up under `stakes.presence`:
    //   Simple   = Some(_)
    //   Parallel = None
    // After the C3 fix (move ValidatorStake to serial phase + port the
    // SimpleExecutor arm), this fixture is green.
    let fixture = ParityFixture {
        name: "validator-stake-happy-path",
        seed: |db| fund(db, 1, 100_000),
        transaction: Transaction::ValidatorStake(ValidatorStakeTx {
            validator_address: addr(1),
            stake_amount: 50_000,
            validator_id: 1,
            nonce: 0,
            bls_public_key: None,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_validator_stake_insufficient_balance() {
    // Both executors must reject with InsufficientBalance — no stake
    // record created, no balance moved.
    let fixture = ParityFixture {
        name: "validator-stake-insufficient-balance",
        seed: |db| fund(db, 1, 100),
        transaction: Transaction::ValidatorStake(ValidatorStakeTx {
            validator_address: addr(1),
            stake_amount: 50_000,
            validator_id: 1,
            nonce: 0,
            bls_public_key: None,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_validator_stake_zero_amount() {
    // ZeroAmount must reject identically; pre-fix the parallel path
    // returned ZeroAmount before the put_stake bug, so this case was
    // accidentally green.
    let fixture = ParityFixture {
        name: "validator-stake-zero-amount",
        seed: |db| fund(db, 1, 100_000),
        transaction: Transaction::ValidatorStake(ValidatorStakeTx {
            validator_address: addr(1),
            stake_amount: 0,
            validator_id: 1,
            nonce: 0,
            bls_public_key: None,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_validator_stake_nonce_mismatch() {
    // Nonce mismatch must reject in both executors with the same
    // expected/got values.
    let fixture = ParityFixture {
        name: "validator-stake-nonce-mismatch",
        seed: |db| fund(db, 1, 100_000),
        transaction: Transaction::ValidatorStake(ValidatorStakeTx {
            validator_address: addr(1),
            stake_amount: 50_000,
            validator_id: 1,
            nonce: 7, // staker nonce is 0
            bls_public_key: None,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

// ─── Phase 3: delegation-trio fixtures (closes audit C4 — first cluster) ──

/// Seed: validator-1 has a 10_000 stake record + delegator funded with 5_000.
/// Reusable across Delegate / Undelegate / ClaimDelegation fixtures.
fn seed_with_validator_and_delegator(db: &mut InMemoryStateDB) {
    db.put_stake(StakeRecord {
        validator_id: 1,
        validator_address: addr(1),
        staked_amount: 10_000,
        staked_at_epoch: 1,
        unbonding_epoch: None,
        slashed_amount: 0,
    });
    fund(db, 64, 5_000);
}

#[test]
fn parity_delegate_happy_path() {
    // Pre-fix divergence: Parallel errored "delegation txs execute in
    // serial phase" → r.txs_failed == 1 → no delegation record.
    // SimpleExecutor wrote the record. Post-fix both succeed identically.
    let fixture = ParityFixture {
        name: "delegate-happy-path",
        seed: seed_with_validator_and_delegator,
        transaction: Transaction::Delegate(DelegateTx {
            delegator: addr(64),
            validator_id: 1,
            amount: 1_000,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_delegate_to_nonexistent_validator() {
    // Validator-99 has no stake record — both executors must reject
    // identically with ContractError, no balance moved.
    let fixture = ParityFixture {
        name: "delegate-to-nonexistent-validator",
        seed: |db| fund(db, 64, 5_000),
        transaction: Transaction::Delegate(DelegateTx {
            delegator: addr(64),
            validator_id: 99,
            amount: 1_000,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_delegate_zero_amount() {
    let fixture = ParityFixture {
        name: "delegate-zero-amount",
        seed: seed_with_validator_and_delegator,
        transaction: Transaction::Delegate(DelegateTx {
            delegator: addr(64),
            validator_id: 1,
            amount: 0,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_delegate_insufficient_balance() {
    let fixture = ParityFixture {
        name: "delegate-insufficient-balance",
        seed: |db| {
            db.put_stake(StakeRecord {
                validator_id: 1,
                validator_address: addr(1),
                staked_amount: 10_000,
                staked_at_epoch: 1,
                unbonding_epoch: None,
                slashed_amount: 0,
            });
            fund(db, 64, 100);
        },
        transaction: Transaction::Delegate(DelegateTx {
            delegator: addr(64),
            validator_id: 1,
            amount: 5_000,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_undelegate_without_prior_delegation() {
    // No prior Delegate → both executors must reject with ContractError
    // ("no delegation from ... to validator-id ...").
    let fixture = ParityFixture {
        name: "undelegate-without-prior-delegation",
        seed: |db| fund(db, 64, 5_000),
        transaction: Transaction::Undelegate(UndelegateTx {
            delegator: addr(64),
            validator_id: 1,
            amount: 500,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_undelegate_zero_amount() {
    let fixture = ParityFixture {
        name: "undelegate-zero-amount",
        seed: |db| fund(db, 64, 5_000),
        transaction: Transaction::Undelegate(UndelegateTx {
            delegator: addr(64),
            validator_id: 1,
            amount: 0,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_claim_delegation_without_unbonding() {
    // Empty state → no delegation record → both executors must reject
    // identically with "no delegation from ...".
    let fixture = ParityFixture {
        name: "claim-delegation-without-prior-delegation",
        seed: |db| fund(db, 64, 5_000),
        transaction: Transaction::ClaimDelegation(ClaimDelegationTx {
            delegator: addr(64),
            validator_id: 1,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

// ─── Phase 3b: Governance + MultiSig fixtures (closes audit C4 — second cluster) ──

#[test]
fn parity_governance_create_proposal_happy_path() {
    // Pre-fix divergence: Parallel errored "governance txs execute in
    // serial phase" → no proposal stored. SimpleExecutor put_proposal.
    // Post-fix both succeed identically.
    let fixture = ParityFixture {
        name: "governance-create-proposal-happy",
        seed: |db| fund(db, 60, 1_000),
        transaction: Transaction::Governance(GovernanceTx {
            action: GovernanceAction::CreateProposal {
                title: "raise block gas limit".into(),
                param_key: "block_gas_limit".into(),
                param_value: "5000000".into(),
                voting_epochs: 50,
            },
            sender: addr(60),
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_governance_create_proposal_title_too_long() {
    let huge_title: String = "x".repeat(300); // > MAX_PROPOSAL_TITLE_BYTES
    let fixture = ParityFixture {
        name: "governance-create-proposal-title-too-long",
        seed: |db| fund(db, 60, 1_000),
        transaction: Transaction::Governance(GovernanceTx {
            action: GovernanceAction::CreateProposal {
                title: huge_title,
                param_key: "block_gas_limit".into(),
                param_value: "5000000".into(),
                voting_epochs: 50,
            },
            sender: addr(60),
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_governance_create_proposal_non_governable_key() {
    let fixture = ParityFixture {
        name: "governance-create-proposal-non-governable-key",
        seed: |db| fund(db, 60, 1_000),
        transaction: Transaction::Governance(GovernanceTx {
            action: GovernanceAction::CreateProposal {
                title: "sneak in a backdoor".into(),
                param_key: "arbitrary_backdoor_key".into(),
                param_value: "true".into(),
                voting_epochs: 50,
            },
            sender: addr(60),
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_governance_cast_vote_against_missing_proposal() {
    let fixture = ParityFixture {
        name: "governance-cast-vote-against-missing-proposal",
        seed: |db| fund(db, 60, 1_000),
        transaction: Transaction::Governance(GovernanceTx {
            action: GovernanceAction::CastVote {
                proposal_id: 999,
                vote: true,
            },
            sender: addr(60),
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_multisig_happy_path() {
    // Pre-fix divergence: Parallel errored "multisig txs execute in
    // serial phase". Post-fix both succeed identically: nonce bumped.
    let fixture = ParityFixture {
        name: "multisig-happy",
        seed: |db| fund(db, 61, 1_000),
        transaction: Transaction::MultiSig(MultiSigTx {
            multisig_address: addr(61),
            threshold: 1,
            signers: vec![addr(62)],
            inner_tx_bytes: vec![],
            signatures: vec![(addr(62), vec![0u8; 64])],
            public_keys: vec![],
            nonce: 0,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_multisig_insufficient_signatures() {
    let fixture = ParityFixture {
        name: "multisig-insufficient-signatures",
        seed: |db| fund(db, 61, 1_000),
        transaction: Transaction::MultiSig(MultiSigTx {
            multisig_address: addr(61),
            threshold: 2,
            signers: vec![addr(62), addr(63)],
            inner_tx_bytes: vec![],
            signatures: vec![(addr(62), vec![0u8; 64])], // only 1 < threshold 2
            public_keys: vec![],
            nonce: 0,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_multisig_unauthorized_signer() {
    let fixture = ParityFixture {
        name: "multisig-unauthorized-signer",
        seed: |db| fund(db, 61, 1_000),
        transaction: Transaction::MultiSig(MultiSigTx {
            multisig_address: addr(61),
            threshold: 1,
            signers: vec![addr(62)],
            inner_tx_bytes: vec![],
            // addr(99) is NOT in signers list — must reject in both.
            signatures: vec![(addr(99), vec![0u8; 64])],
            public_keys: vec![],
            nonce: 0,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_multisig_duplicate_signer() {
    let fixture = ParityFixture {
        name: "multisig-duplicate-signer",
        seed: |db| fund(db, 61, 1_000),
        transaction: Transaction::MultiSig(MultiSigTx {
            multisig_address: addr(61),
            threshold: 2,
            signers: vec![addr(62), addr(63)],
            inner_tx_bytes: vec![],
            // Same signer twice — must reject identically.
            signatures: vec![
                (addr(62), vec![0u8; 64]),
                (addr(62), vec![0u8; 64]),
            ],
            public_keys: vec![],
            nonce: 0,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}
