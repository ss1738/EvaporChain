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
    MultiSigTx, StakeRecord, Transaction, TransferTx, UndelegateTx, UpgradeContractTx,
    ValidatorStakeTx,
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
    compare_privacy(&simple_db, &parallel_db, &mut divergences);

    divergences
}

// ─── Phase 4: privacy comparator (nullifiers + note commitments + pool) ──

fn compare_privacy(
    simple: &InMemoryStateDB,
    parallel: &InMemoryStateDB,
    out: &mut Vec<Divergence>,
) {
    // Shielded-pool balance — single u64, easy.
    let s_pool = simple.get_shielded_pool_balance();
    let p_pool = parallel.get_shielded_pool_balance();
    if s_pool != p_pool {
        out.push(Divergence {
            domain: "privacy.shielded_pool_balance",
            detail: "block-end pool total".into(),
            simple_value: s_pool.to_string(),
            parallel_value: p_pool.to_string(),
        });
    }

    // Spent-nullifier set — compare as BTreeSet so ordering doesn't
    // matter. Any divergence in membership is a parity failure.
    let s_nulls: std::collections::BTreeSet<[u8; 32]> =
        simple.all_nullifiers().into_iter().collect();
    let p_nulls: std::collections::BTreeSet<[u8; 32]> =
        parallel.all_nullifiers().into_iter().collect();
    for n in s_nulls.difference(&p_nulls) {
        out.push(Divergence {
            domain: "privacy.spent_nullifiers.presence",
            detail: format!("nullifier {:02x}… spent only in SimpleExecutor DB", n[0]),
            simple_value: "spent".into(),
            parallel_value: "not-spent".into(),
        });
    }
    for n in p_nulls.difference(&s_nulls) {
        out.push(Divergence {
            domain: "privacy.spent_nullifiers.presence",
            detail: format!("nullifier {:02x}… spent only in ParallelExecutor DB", n[0]),
            simple_value: "not-spent".into(),
            parallel_value: "spent".into(),
        });
    }

    // Note-commitment set — BTreeSet for stable comparison.
    let s_notes: std::collections::BTreeSet<[u8; 32]> =
        simple.get_all_note_commitments().into_iter().collect();
    let p_notes: std::collections::BTreeSet<[u8; 32]> =
        parallel.get_all_note_commitments().into_iter().collect();
    for c in s_notes.difference(&p_notes) {
        out.push(Divergence {
            domain: "privacy.note_commitments.presence",
            detail: format!(
                "commitment {:02x}… persisted only in SimpleExecutor DB",
                c[0]
            ),
            simple_value: "Some(_)".into(),
            parallel_value: "None".into(),
        });
    }
    for c in p_notes.difference(&s_notes) {
        out.push(Divergence {
            domain: "privacy.note_commitments.presence",
            detail: format!(
                "commitment {:02x}… persisted only in ParallelExecutor DB",
                c[0]
            ),
            simple_value: "None".into(),
            parallel_value: "Some(_)".into(),
        });
    }
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

// ─── Phase 3c: UpgradeContract fixture (closes audit C4 — 6/7) ───────────
//
// UpgradeContract uses the shared `execute_upgrade_contract_impl` so
// parity is guaranteed by construction. The harness can only exercise
// error paths (contract-not-found, bytecode-hash-mismatch) without
// per-executor script_engine state. Happy-path coverage lives in the
// in-impl integration tests where the script_engine is pre-populated.

#[test]
fn parity_upgrade_contract_non_existent_id() {
    // Both executors run through the shared impl and reach the
    // "contract N not found" branch. Pre-fix, ParallelExecutor errored
    // at the partition with "executes in serial phase" instead — a
    // different error variant + no nonce bump. Now both fail
    // identically via execute_upgrade_contract_impl.
    let payload = b"contract Empty { fn noop() { } }".to_vec();
    let hash = *blake3::hash(&payload).as_bytes();
    let fixture = ParityFixture {
        name: "upgrade-contract-non-existent-id",
        seed: |db| fund(db, 63, 1_000),
        transaction: Transaction::UpgradeContract(UpgradeContractTx {
            owner: addr(63),
            contract_id: 999,
            new_bytecode: payload,
            new_bytecode_hash: hash,
            nonce: 0,
            admin_signature: None,
            admin_public_key: None,
            endorser_stakes: vec![],
            required_stake: 0,
            governance_approved: false,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_upgrade_contract_bytecode_hash_mismatch() {
    // Mismatched hash trips the binding check before any contract
    // lookup. Both executors must reject identically via the shared
    // impl.
    let fixture = ParityFixture {
        name: "upgrade-contract-bytecode-hash-mismatch",
        seed: |db| fund(db, 63, 1_000),
        transaction: Transaction::UpgradeContract(UpgradeContractTx {
            owner: addr(63),
            contract_id: 1,
            new_bytecode: b"contract A {}".to_vec(),
            new_bytecode_hash: [0xFF; 32], // not BLAKE3 of the body
            nonce: 0,
            admin_signature: None,
            admin_public_key: None,
            endorser_stakes: vec![],
            required_stake: 0,
            governance_approved: false,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

// ─── Phase 3d: UserOp gas-only envelope fixture (closes audit C4 7/7) ─────
//
// Non-empty call_data fixtures are deferred to Phase 3e — the inner
// dispatch helpers (execute_inner_transfer / execute_call_script /
// execute_call_contract) remain SimpleExecutor-only until that PR.
// Empty call_data exercises the full envelope path on both executors.

#[test]
fn parity_userop_gas_only_no_paymaster() {
    // Pre-fix: ParallelExecutor errored at the partition's "executes in
    // serial phase" arm. SimpleExecutor succeeded. The harness reported
    // a `result.disposition` divergence (Ok vs Err). Post-fix both
    // succeed via the shared envelope impl with identical nonce bumps.
    let fixture = ParityFixture {
        name: "userop-gas-only-no-paymaster",
        seed: |db| fund(db, 62, 1_000),
        transaction: Transaction::UserOp(evaporchain_types::UserOpTx {
            sender: addr(62),
            nonce: 0,
            call_data: vec![],
            call_gas_limit: 0,
            paymaster: None,
            paymaster_nonce: None,
            paymaster_data: None,
            paymaster_signature: None,
            paymaster_public_key: None,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_userop_nonce_mismatch() {
    // Envelope-level nonce check fires on both executors identically.
    let fixture = ParityFixture {
        name: "userop-nonce-mismatch",
        seed: |db| fund(db, 62, 1_000),
        transaction: Transaction::UserOp(evaporchain_types::UserOpTx {
            sender: addr(62),
            nonce: 7, // sender nonce is 0
            call_data: vec![],
            call_gas_limit: 0,
            paymaster: None,
            paymaster_nonce: None,
            paymaster_data: None,
            paymaster_signature: None,
            paymaster_public_key: None,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

// ─── Phase 3e: UserOp with inner Transfer (closes audit C4 fully) ─────────

#[test]
fn parity_userop_with_inner_transfer() {
    // Phase 3d only handled empty call_data (envelope-only). Phase 3e
    // ports the inner-tx dispatch, so a UserOp wrapping an inner
    // Transfer now executes end-to-end on both executors and produces
    // byte-identical post-state.
    let inner_payload = serde_json::to_vec(&Transaction::Transfer(TransferTx {
        from: addr(62),
        to: addr(63),
        amount: 200,
        nonce: 0,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    }))
    .unwrap();
    let fixture = ParityFixture {
        name: "userop-with-inner-transfer",
        seed: |db| fund(db, 62, 5_000),
        transaction: Transaction::UserOp(evaporchain_types::UserOpTx {
            sender: addr(62),
            nonce: 0,
            call_data: inner_payload,
            call_gas_limit: 100_000,
            paymaster: None,
            paymaster_nonce: None,
            paymaster_data: None,
            paymaster_signature: None,
            paymaster_public_key: None,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_userop_inner_transfer_impersonation_rejected() {
    // No-impersonation: inner.from must equal outer UserOp.sender.
    // Both executors must reject identically with ContractError.
    let bad_inner = serde_json::to_vec(&Transaction::Transfer(TransferTx {
        from: addr(99), // not the outer sender
        to: addr(63),
        amount: 100,
        nonce: 0,
        signature: None,
        public_key: None,
        mev_refund_eligible: None,
    }))
    .unwrap();
    let fixture = ParityFixture {
        name: "userop-inner-transfer-impersonation",
        seed: |db| {
            fund(db, 62, 5_000);
            fund(db, 99, 5_000);
        },
        transaction: Transaction::UserOp(evaporchain_types::UserOpTx {
            sender: addr(62),
            nonce: 0,
            call_data: bad_inner,
            call_gas_limit: 100_000,
            paymaster: None,
            paymaster_nonce: None,
            paymaster_data: None,
            paymaster_signature: None,
            paymaster_public_key: None,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

// ─── Phase 4: privacy variants (Shield / Unshield / PrivateTransfer) ─────

#[test]
fn parity_shield_happy_path() {
    // Shield debits transparent balance, credits shielded_pool, and
    // appends a note commitment. Both executors must produce the same
    // post-state across accounts, shielded_pool_balance, and the
    // commitment set.
    let fixture = ParityFixture {
        name: "shield-happy-path",
        seed: |db| fund(db, 10, 10_000),
        transaction: Transaction::Shield(evaporchain_types::ShieldTx {
            from: addr(10),
            amount: 2_500,
            nonce: 0,
            note_owner_hash: [0xAA; 32],
            value_blinding: [0xBB; 32],
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_shield_zero_amount() {
    // Shield with amount=0 must reject identically on both executors.
    let fixture = ParityFixture {
        name: "shield-zero-amount",
        seed: |db| fund(db, 10, 10_000),
        transaction: Transaction::Shield(evaporchain_types::ShieldTx {
            from: addr(10),
            amount: 0,
            nonce: 0,
            note_owner_hash: [0xAA; 32],
            value_blinding: [0xBB; 32],
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_shield_insufficient_balance() {
    let fixture = ParityFixture {
        name: "shield-insufficient-balance",
        seed: |db| fund(db, 10, 100),
        transaction: Transaction::Shield(evaporchain_types::ShieldTx {
            from: addr(10),
            amount: 5_000,
            nonce: 0,
            note_owner_hash: [0xAA; 32],
            value_blinding: [0xBB; 32],
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_shield_nonce_mismatch() {
    let fixture = ParityFixture {
        name: "shield-nonce-mismatch",
        seed: |db| fund(db, 10, 10_000),
        transaction: Transaction::Shield(evaporchain_types::ShieldTx {
            from: addr(10),
            amount: 2_500,
            nonce: 7, // sender nonce is 0
            note_owner_hash: [0xAA; 32],
            value_blinding: [0xBB; 32],
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_unshield_missing_anchor_rejected() {
    // Empty inputs / unknown anchor — both executors must reject
    // identically at the privacy_exec validation gate before any
    // shielded_pool mutation. Happy-path Unshield requires a real
    // Merkle proof which isn't tractable in a unit-level fixture;
    // error-path parity is what matters for the harness.
    let fixture = ParityFixture {
        name: "unshield-missing-anchor",
        seed: |db| fund(db, 10, 100),
        transaction: Transaction::Unshield(evaporchain_types::UnshieldTx {
            to: addr(10),
            amount: 1_000,
            input_nullifiers: vec![[0x11; 32]],
            anchor: [0xFF; 32], // anchor doesn't exist in the empty trie
            balance_binding: [0; 32],
            input_amounts: vec![1_000],
            input_blindings: vec![[0x22; 32]],
            input_value_commitments: vec![[0x33; 32]],
            input_note_commitments: vec![[0x44; 32]],
            input_merkle_proofs: vec![],
            output_blindings: vec![],
            change_commitments: vec![],
            energy_proofs: vec![],
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_private_transfer_missing_anchor_rejected() {
    let fixture = ParityFixture {
        name: "private-transfer-missing-anchor",
        seed: |_db| {},
        transaction: Transaction::PrivateTransfer(evaporchain_types::PrivateTransferTx {
            input_nullifiers: vec![[0x11; 32]],
            output_commitments: vec![[0x22; 32]],
            anchor: [0xFF; 32],
            balance_binding: [0; 32],
            fee: 0,
            input_amounts: vec![100],
            input_blindings: vec![[0x33; 32]],
            input_value_commitments: vec![[0x44; 32]],
            input_note_commitments: vec![[0x55; 32]],
            input_merkle_proofs: vec![],
            output_blindings: vec![[0x66; 32]],
            output_value_commitments: vec![[0x77; 32]],
            output_amounts: vec![100],
            energy_proofs: vec![],
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

// ─── Phase 5: contract + tail variants (closes the 25-variant matrix) ────
//
// For variants where happy-path requires complex pre-conditions
// (deployed contracts, registered scripts, valid signatures), the
// error-path fixtures lock rejection-side parity. The harness goal
// is "same input → same output on both executors" — error parity is
// as important as success parity for the recurrence-proof property.

#[test]
fn parity_refresh_unknown_object() {
    let fixture = ParityFixture {
        name: "refresh-unknown-object",
        seed: |db| fund(db, 10, 1_000),
        transaction: Transaction::Refresh(evaporchain_types::RefreshTx {
            object_id: [0xFF; 32],
            energy_deposit: 100,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_create_object_empty_data() {
    let fixture = ParityFixture {
        name: "create-object-empty-data",
        seed: |db| fund(db, 11, 100_000),
        transaction: Transaction::CreateObject(evaporchain_types::CreateObjectTx {
            creator: addr(11),
            object_id: [0x77; 32],
            energy: 1_000,
            half_life: 100,
            data: vec![],
            decay_curve: None,
            lad_mode: None,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_deploy_contract_unknown_template() {
    let fixture = ParityFixture {
        name: "deploy-contract-unknown-template",
        seed: |db| fund(db, 12, 100_000),
        transaction: Transaction::DeployContract(evaporchain_types::DeployContractTx {
            deployer: addr(12),
            template: "NonExistentTemplate".into(),
            init_args: "{}".into(),
            energy: 1_000,
            half_life: 100,
            rules: None,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_call_contract_unknown_id() {
    let fixture = ParityFixture {
        name: "call-contract-unknown-id",
        seed: |db| fund(db, 13, 1_000),
        transaction: Transaction::CallContract(evaporchain_types::CallContractTx {
            caller: addr(13),
            contract_id: 9999,
            method: "noop".into(),
            args: "[]".into(),
            epoch: 1,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_deploy_script_invalid_source() {
    let fixture = ParityFixture {
        name: "deploy-script-invalid-source",
        seed: |db| fund(db, 14, 100_000),
        transaction: Transaction::DeployScript(evaporchain_types::DeployScriptTx {
            deployer: addr(14),
            source_code: "garbage{}{not valid syntax".into(),
            energy: 1_000,
            half_life: 100,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_call_script_unknown_id() {
    let fixture = ParityFixture {
        name: "call-script-unknown-id",
        seed: |db| fund(db, 15, 1_000),
        transaction: Transaction::CallScript(evaporchain_types::CallScriptTx {
            caller: addr(15),
            contract_id: 9999,
            method: "noop".into(),
            args: "[]".into(),
            epoch: 1,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_validator_exit_no_stake_record() {
    let fixture = ParityFixture {
        name: "validator-exit-no-stake-record",
        seed: |db| fund(db, 16, 1_000),
        transaction: Transaction::ValidatorExit(evaporchain_types::ValidatorExitTx {
            validator_address: addr(16),
            validator_id: 9999,
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
fn parity_validator_claim_stake_no_record() {
    let fixture = ParityFixture {
        name: "validator-claim-stake-no-record",
        seed: |db| fund(db, 17, 1_000),
        transaction: Transaction::ValidatorClaimStake(evaporchain_types::ValidatorClaimStakeTx {
            validator_address: addr(17),
            validator_id: 9999,
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
fn parity_rotate_validator_key_wrong_length() {
    let fixture = ParityFixture {
        name: "rotate-validator-key-wrong-length",
        seed: |db| fund(db, 18, 1_000),
        transaction: Transaction::RotateValidatorKey(evaporchain_types::RotateValidatorKeyTx {
            validator_address: addr(18),
            validator_id: 1,
            new_bls_public_key: vec![0xAA; 32], // must be 48 bytes
            bls_pop_old: vec![0; 96],
            bls_pop_new: vec![0; 96],
            effective_epoch: 5,
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
fn parity_deferred_zero_deposit() {
    let fixture = ParityFixture {
        name: "deferred-zero-deposit",
        seed: |db| fund(db, 19, 1_000),
        transaction: Transaction::Deferred(evaporchain_types::DeferredTx {
            submitter: addr(19),
            nonce: 0,
            deposit: 0,
            guards: vec![],
            inner_tx_bytes: vec![],
            gas_limit: 0,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_blob_empty_data() {
    let fixture = ParityFixture {
        name: "blob-empty-data",
        seed: |db| fund(db, 20, 1_000),
        transaction: Transaction::Blob(evaporchain_types::BlobTx {
            submitter: addr(20),
            data: vec![],
            nonce: 0,
            namespace_id: 1,
            signature: None,
            public_key: None,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

#[test]
fn parity_refund_unknown_observation() {
    let fixture = ParityFixture {
        name: "refund-unknown-observation",
        seed: |db| {
            fund(db, 21, 1_000);
            fund(db, 22, 1_000);
        },
        transaction: Transaction::Refund(evaporchain_types::RefundTx {
            source_block_height: 999,
            source_observation_idx: 999,
            attacker: addr(21),
            victim: addr(22),
            amount: 100,
            settle_block_height: 1,
        }),
        block_number: 1,
        epoch: 1,
    };
    assert_parity(&fixture);
}

// ─── Phase 5: enum-exhaustiveness gate (recurrence-proof) ─────────────────
//
// Adding a new `Transaction` variant must require a parity-fixture
// author to acknowledge it. This gate uses an exhaustive match on
// `&Transaction` — a new variant won't compile until the match has
// an arm for it. The arm body documents which fixture covers the
// variant; an `unimplemented!("…no parity fixture yet…")` arm forces
// the author to add one before merging.
//
// This is what closes the audit's Theme B class permanently: the
// next time someone adds (say) `Transaction::FooBar`, this test
// fails to compile and they MUST either write a parity fixture for
// FooBar or explicitly opt out with a documented reason.
#[test]
fn enum_exhaustiveness_every_tx_variant_has_a_parity_fixture() {
    use evaporchain_types::Transaction as T;

    /// Returns the name of the fixture(s) covering a given variant.
    /// Adding a new variant to `Transaction` causes this match to
    /// fail compilation until an arm is added — the parity-arc's
    /// recurrence-proof property.
    #[allow(unreachable_patterns)]
    fn fixture_for(tx: &T) -> &'static str {
        match tx {
            T::Transfer(_) => "parity_transfer_*",
            T::Refresh(_) => "parity_refresh_unknown_object",
            T::CreateObject(_) => "parity_create_object_empty_data",
            T::DeployContract(_) => "parity_deploy_contract_unknown_template",
            T::CallContract(_) => "parity_call_contract_unknown_id",
            T::DeployScript(_) => "parity_deploy_script_invalid_source",
            T::CallScript(_) => "parity_call_script_unknown_id",
            T::ValidatorStake(_) => "parity_validator_stake_*",
            T::ValidatorExit(_) => "parity_validator_exit_no_stake_record",
            T::ValidatorClaimStake(_) => "parity_validator_claim_stake_no_record",
            T::Shield(_) => "parity_shield_*",
            T::Unshield(_) => "parity_unshield_missing_anchor_rejected",
            T::PrivateTransfer(_) => "parity_private_transfer_missing_anchor_rejected",
            T::Deferred(_) => "parity_deferred_zero_deposit",
            T::Blob(_) => "parity_blob_empty_data",
            T::Governance(_) => "parity_governance_*",
            T::MultiSig(_) => "parity_multisig_*",
            T::UserOp(_) => "parity_userop_*",
            T::UpgradeContract(_) => "parity_upgrade_contract_*",
            T::Delegate(_) => "parity_delegate_*",
            T::Undelegate(_) => "parity_undelegate_*",
            T::RotateValidatorKey(_) => "parity_rotate_validator_key_wrong_length",
            T::ClaimDelegation(_) => "parity_claim_delegation_*",
            T::Refund(_) => "parity_refund_unknown_observation",
        }
    }

    // Build one representative of every variant. Any divergence
    // between this list and the Transaction enum is caught at compile
    // time by `fixture_for`'s match.
    let representatives: Vec<T> = vec![
        T::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 0,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        }),
        T::Refresh(evaporchain_types::RefreshTx {
            object_id: [0; 32],
            energy_deposit: 0,
            signature: None,
            public_key: None,
        }),
        T::CreateObject(evaporchain_types::CreateObjectTx {
            creator: addr(1),
            object_id: [0; 32],
            energy: 0,
            half_life: 0,
            data: vec![],
            decay_curve: None,
            lad_mode: None,
            signature: None,
            public_key: None,
        }),
        T::DeployContract(evaporchain_types::DeployContractTx {
            deployer: addr(1),
            template: String::new(),
            init_args: String::new(),
            energy: 0,
            half_life: 0,
            rules: None,
            signature: None,
            public_key: None,
        }),
        T::CallContract(evaporchain_types::CallContractTx {
            caller: addr(1),
            contract_id: 0,
            method: String::new(),
            args: String::new(),
            epoch: 0,
            signature: None,
            public_key: None,
        }),
        T::DeployScript(evaporchain_types::DeployScriptTx {
            deployer: addr(1),
            source_code: String::new(),
            energy: 0,
            half_life: 0,
            signature: None,
            public_key: None,
        }),
        T::CallScript(evaporchain_types::CallScriptTx {
            caller: addr(1),
            contract_id: 0,
            method: String::new(),
            args: String::new(),
            epoch: 0,
            signature: None,
            public_key: None,
        }),
        T::ValidatorStake(ValidatorStakeTx {
            validator_address: addr(1),
            stake_amount: 0,
            validator_id: 0,
            nonce: 0,
            bls_public_key: None,
            vrf_public_key: None,
            signature: None,
            public_key: None,
        }),
        T::ValidatorExit(evaporchain_types::ValidatorExitTx {
            validator_address: addr(1),
            validator_id: 0,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        T::ValidatorClaimStake(evaporchain_types::ValidatorClaimStakeTx {
            validator_address: addr(1),
            validator_id: 0,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        T::Shield(evaporchain_types::ShieldTx {
            from: addr(1),
            amount: 0,
            nonce: 0,
            note_owner_hash: [0; 32],
            value_blinding: [0; 32],
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        }),
        T::Unshield(evaporchain_types::UnshieldTx {
            to: addr(1),
            amount: 0,
            input_nullifiers: vec![],
            anchor: [0; 32],
            balance_binding: [0; 32],
            input_amounts: vec![],
            input_blindings: vec![],
            input_value_commitments: vec![],
            input_note_commitments: vec![],
            input_merkle_proofs: vec![],
            output_blindings: vec![],
            change_commitments: vec![],
            energy_proofs: vec![],
        }),
        T::PrivateTransfer(evaporchain_types::PrivateTransferTx {
            input_nullifiers: vec![],
            output_commitments: vec![],
            anchor: [0; 32],
            balance_binding: [0; 32],
            fee: 0,
            input_amounts: vec![],
            input_blindings: vec![],
            input_value_commitments: vec![],
            input_note_commitments: vec![],
            input_merkle_proofs: vec![],
            output_blindings: vec![],
            output_value_commitments: vec![],
            output_amounts: vec![],
            energy_proofs: vec![],
        }),
        T::Deferred(evaporchain_types::DeferredTx {
            submitter: addr(1),
            nonce: 0,
            deposit: 0,
            guards: vec![],
            inner_tx_bytes: vec![],
            gas_limit: 0,
            signature: None,
            public_key: None,
        }),
        T::Blob(evaporchain_types::BlobTx {
            submitter: addr(1),
            data: vec![],
            nonce: 0,
            namespace_id: 1,
            signature: None,
            public_key: None,
        }),
        T::Governance(GovernanceTx {
            action: GovernanceAction::CastVote {
                proposal_id: 0,
                vote: true,
            },
            sender: addr(1),
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        T::MultiSig(MultiSigTx {
            multisig_address: addr(1),
            threshold: 1,
            signers: vec![addr(2)],
            inner_tx_bytes: vec![],
            signatures: vec![],
            public_keys: vec![],
            nonce: 0,
        }),
        T::UserOp(evaporchain_types::UserOpTx {
            sender: addr(1),
            nonce: 0,
            call_data: vec![],
            call_gas_limit: 0,
            paymaster: None,
            paymaster_nonce: None,
            paymaster_data: None,
            paymaster_signature: None,
            paymaster_public_key: None,
            signature: None,
            public_key: None,
        }),
        T::UpgradeContract(UpgradeContractTx {
            owner: addr(1),
            contract_id: 0,
            new_bytecode: vec![],
            new_bytecode_hash: [0; 32],
            nonce: 0,
            admin_signature: None,
            admin_public_key: None,
            endorser_stakes: vec![],
            required_stake: 0,
            governance_approved: false,
            signature: None,
            public_key: None,
        }),
        T::Delegate(DelegateTx {
            delegator: addr(1),
            validator_id: 0,
            amount: 0,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        T::Undelegate(UndelegateTx {
            delegator: addr(1),
            validator_id: 0,
            amount: 0,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        T::RotateValidatorKey(evaporchain_types::RotateValidatorKeyTx {
            validator_address: addr(1),
            validator_id: 0,
            new_bls_public_key: vec![],
            bls_pop_old: vec![],
            bls_pop_new: vec![],
            effective_epoch: 0,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        T::ClaimDelegation(ClaimDelegationTx {
            delegator: addr(1),
            validator_id: 0,
            nonce: 0,
            signature: None,
            public_key: None,
        }),
        T::Refund(evaporchain_types::RefundTx {
            source_block_height: 0,
            source_observation_idx: 0,
            attacker: addr(1),
            victim: addr(2),
            amount: 0,
            settle_block_height: 1,
        }),
    ];

    // Every representative resolves to a fixture name (a documentation
    // string; not actually called here). If `Transaction` gains a new
    // variant, `fixture_for`'s match becomes non-exhaustive and the
    // file fails to compile — forcing the new variant's author to add
    // a parity fixture.
    for tx in &representatives {
        let _name = fixture_for(tx);
    }
    assert_eq!(
        representatives.len(),
        24,
        "exhaustiveness baseline: 24 Transaction variants known"
    );
}
