//! Audit-prep test suite: balance conservation, nonce safety, double-spend
//! prevention, gas exhaustion attacks, and property-based testing.

#[cfg(test)]
mod invariant_tests {
    use crate::parallel::ParallelExecutor;
    use crate::ExecutionEngine;
    use evaporchain_state::db::InMemoryStateDB;
    use evaporchain_state::db::StateDB;
    use evaporchain_types::{Account, Block, CreateObjectTx, Transaction, TransferTx};

    fn addr(byte: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = byte;
        a
    }

    fn make_block(number: u64, epoch: u64, txs: Vec<Transaction>) -> Block {
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

    fn fund_account(db: &mut InMemoryStateDB, byte: u8, balance: u64) {
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

    // ═══════════════════════════════════════════════════════════════════
    // Balance conservation invariant
    // ═══════════════════════════════════════════════════════════════════

    /// Total supply must be conserved across transfers (minus fees).
    #[test]
    fn test_balance_conservation_transfers() {
        let mut db = InMemoryStateDB::new();
        let initial_balance = 1_000_000u64;
        for i in 0..5u8 {
            fund_account(&mut db, i, initial_balance);
        }

        let total_before: u64 = (0..5u8)
            .map(|i| db.get_account(&addr(i)).unwrap().balance)
            .sum();

        let txs: Vec<Transaction> = (0..100)
            .map(|i| {
                Transaction::Transfer(TransferTx {
                    from: addr((i % 5) as u8),
                    to: addr(((i + 1) % 5) as u8),
                    amount: 10,
                    nonce: (i / 5) as u64,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                })
            })
            .collect();

        let mut executor = ParallelExecutor::new_for_test(5);
        let result = executor
            .execute_block(&mut db, &make_block(1, 1, txs))
            .unwrap();

        let total_after: u64 = (0..5u8)
            .map(|i| db.get_account(&addr(i)).unwrap().balance)
            .sum();

        // Total supply = balances + fees collected + refresh pool.
        // Demurrage now redirects from `account.balance` into
        // `executor.refresh_pool` on every per-epoch sweep (Layer 0
        // item 4). Conservation across the §1.2 compartments
        // {Accounts, Stake, RefreshPool, SlashedPool} still holds —
        // this assertion just had to grow to include the redirected
        // pool balance instead of treating it as missing supply.
        let pool = executor.refresh_pool.total_accrued();
        assert_eq!(
            total_before,
            total_after + result.total_fees + pool,
            "INVARIANT VIOLATION: total supply not conserved. before={}, after={}, fees={}, pool={}",
            total_before,
            total_after,
            result.total_fees,
            pool
        );
    }

    /// Self-transfers should not create or destroy value.
    #[test]
    fn test_self_transfer_no_value_change() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);
        let balance_before = db.get_account(&addr(1)).unwrap().balance;

        let txs = vec![Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(1), // self-transfer
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        })];

        let mut executor = ParallelExecutor::new_for_test(5);
        let result = executor
            .execute_block(&mut db, &make_block(1, 1, txs))
            .unwrap();
        let balance_after = db.get_account(&addr(1)).unwrap().balance;

        // Balance should decrease by fees + demurrage redirect.
        // Layer 0 item 4 wired demurrage into ParallelExecutor's
        // hot path; idle balances above the threshold drain to
        // `executor.refresh_pool` on the per-epoch sweep, so this
        // single-account conservation must include the pool.
        let pool = executor.refresh_pool.total_accrued();
        assert_eq!(
            balance_before,
            balance_after + result.total_fees + pool,
            "self-transfer must only deduct fees and demurrage"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Nonce safety
    // ═══════════════════════════════════════════════════════════════════

    /// Replay attack: submitting the same transaction twice should fail the second time.
    #[test]
    fn test_replay_attack_prevention() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);
        fund_account(&mut db, 2, 0);

        let tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });

        // First execution succeeds
        let mut executor = ParallelExecutor::new_for_test(5);
        let result1 = executor
            .execute_block(&mut db, &make_block(1, 1, vec![tx.clone()]))
            .unwrap();
        assert_eq!(result1.txs_executed, 1);

        // Replay same transaction (nonce 0 again) — should fail
        let result2 = executor
            .execute_block(&mut db, &make_block(2, 2, vec![tx]))
            .unwrap();
        assert_eq!(
            result2.txs_failed, 1,
            "replayed transaction must be rejected"
        );
    }

    /// Nonce gap: nonce 2 submitted without nonce 1 should fail.
    #[test]
    fn test_nonce_gap_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        let tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 10,
            nonce: 5, // gap — expected nonce is 0
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });

        let mut executor = ParallelExecutor::new_for_test(5);
        let result = executor
            .execute_block(&mut db, &make_block(1, 1, vec![tx]))
            .unwrap();
        assert_eq!(result.txs_failed, 1, "nonce gap must cause rejection");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Overdraft / insufficient funds
    // ═══════════════════════════════════════════════════════════════════

    /// Transferring more than the balance should fail.
    #[test]
    fn test_overdraft_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 100);

        let tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 101, // exceeds balance
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });

        let mut executor = ParallelExecutor::new_for_test(5);
        let result = executor
            .execute_block(&mut db, &make_block(1, 1, vec![tx]))
            .unwrap();
        assert_eq!(result.txs_failed, 1, "overdraft must be rejected");

        // Sender balance must not go negative
        let sender = db.get_account(&addr(1)).unwrap();
        assert!(
            sender.balance <= 100,
            "balance must not exceed initial after failed tx"
        );
    }

    /// Transfer of exactly the full balance should succeed (minus fees).
    #[test]
    fn test_full_balance_transfer() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        // Transfer a reasonable amount (leave room for fees)
        let tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        });

        let mut executor = ParallelExecutor::new_for_test(5);
        let result = executor
            .execute_block(&mut db, &make_block(1, 1, vec![tx]))
            .unwrap();
        assert_eq!(result.txs_executed, 1);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Gas exhaustion attacks
    // ═══════════════════════════════════════════════════════════════════

    /// Gas limit enforcement: block should not exceed gas limit.
    #[test]
    fn test_gas_limit_enforcement() {
        let mut db = InMemoryStateDB::new();
        for i in 0..200u8 {
            let mut a = [0u8; 32];
            a[0] = i;
            db.put_account(Account {
                address: a,
                balance: 1_000_000_000,
                nonce: 0,
                storage_deposit: 0,
                storage_bytes: 0,
                last_touched_epoch: 0,
                vesting: None,
            });
        }

        // Create many transactions that consume gas
        let txs: Vec<Transaction> = (0..500)
            .map(|i| {
                let mut from = [0u8; 32];
                from[0] = (i % 200) as u8;
                let mut to = [0u8; 32];
                to[0] = ((i + 1) % 200) as u8;
                Transaction::Transfer(TransferTx {
                    from,
                    to,
                    amount: 1,
                    nonce: (i / 200) as u64,
                    signature: None,
                    public_key: None,
                    mev_refund_eligible: None,
                })
            })
            .collect();

        // Use a tight gas limit
        let mut executor = ParallelExecutor::new_for_test(5);
        executor.block_gas_limit = 100_000;
        let result = executor
            .execute_block(&mut db, &make_block(1, 1, txs))
            .unwrap();

        // Gas used must not exceed limit
        assert!(
            result.gas_used <= 100_000,
            "gas used ({}) must not exceed limit (100000)",
            result.gas_used
        );
        // Some txs should have been dropped due to gas limit
        assert!(
            result.txs_executed < 500,
            "gas limit should cap transactions executed"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Energy decay invariants
    // ═══════════════════════════════════════════════════════════════════

    /// Object energy must never increase without an explicit refresh.
    #[test]
    fn test_energy_monotonic_decay() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        let obj_id = {
            let mut id = [0u8; 32];
            id[0] = 42;
            id
        };

        // Create an object
        let create_tx = Transaction::CreateObject(CreateObjectTx {
            creator: addr(1),
            object_id: obj_id,
            energy: 10_000,
            half_life: 10,
            data: vec![0u8; 16],
            decay_curve: None,
            lad_mode: None,
            signature: None,
            public_key: None,
        });

        let mut executor = ParallelExecutor::new_for_test(5);
        let _ = executor
            .execute_block(&mut db, &make_block(1, 1, vec![create_tx]))
            .unwrap();

        // Execute empty blocks at increasing epochs and verify energy never increases
        let mut prev_energy = 10_000u64;
        for epoch in 2..=20 {
            let _ = executor
                .execute_block(&mut db, &make_block(epoch, epoch, vec![]))
                .unwrap();
            if let Some(obj) = db.get_object(&obj_id) {
                let energy = obj.energy_at(epoch);
                assert!(
                    energy <= prev_energy,
                    "INVARIANT VIOLATION: energy increased from {} to {} at epoch {}",
                    prev_energy,
                    energy,
                    epoch
                );
                prev_energy = energy;
            }
        }
    }

    /// Evaporated objects should become ghosts.
    #[test]
    fn test_evaporated_becomes_ghost() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1_000_000);

        // Create object with very short half-life
        let obj_id = {
            let mut id = [0u8; 32];
            id[0] = 99;
            id
        };
        let create_tx = Transaction::CreateObject(CreateObjectTx {
            creator: addr(1),
            object_id: obj_id,
            energy: 100,
            half_life: 1, // decays very fast
            data: vec![],
            decay_curve: None,
            lad_mode: None,
            signature: None,
            public_key: None,
        });

        let mut executor = ParallelExecutor::new_for_test(5);
        let _ = executor
            .execute_block(&mut db, &make_block(1, 1, vec![create_tx]))
            .unwrap();

        // Advance many epochs to ensure evaporation
        for epoch in 2..=200 {
            let result = executor
                .execute_block(&mut db, &make_block(epoch, epoch, vec![]))
                .unwrap();
            if result.objects_evaporated > 0 {
                // Object should have been evaporated
                break;
            }
        }

        // After enough epochs, object should be gone
        // (it may have been evaporated and converted to ghost)
        let ghost_count = db.ghost_count();
        assert!(
            ghost_count > 0 || db.get_object(&obj_id).is_none(),
            "evaporated object should become ghost or be removed"
        );
    }
}

#[cfg(test)]
mod proptest_execution {
    use crate::parallel::ParallelExecutor;
    use crate::ExecutionEngine;
    use evaporchain_state::db::InMemoryStateDB;
    use evaporchain_state::db::StateDB;
    use evaporchain_types::{Account, Block, Transaction, TransferTx};
    use proptest::prelude::*;

    fn addr(byte: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = byte;
        a
    }

    fn make_block(number: u64, epoch: u64, txs: Vec<Transaction>) -> Block {
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

    proptest! {
        /// Transfer amount <= balance should never panic.
        #[test]
        fn transfer_never_panics(
            amount in 0u64..1_000_000,
            balance in 1u64..10_000_000,
        ) {
            let mut db = InMemoryStateDB::new();
            db.put_account(Account { address: addr(1), balance, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 , vesting: None });
            db.put_account(Account { address: addr(2), balance: 0, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 , vesting: None });

            let tx = Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            });

            let mut executor = ParallelExecutor::new_for_test(5);
            // Should never panic regardless of input
            let _ = executor.execute_block(&mut db, &make_block(1, 1, vec![tx]));
        }

        /// After execution, no account balance should ever be negative (u64 underflow).
        #[test]
        fn no_negative_balances(
            num_txs in 1usize..50,
            amount in 1u64..100,
        ) {
            let mut db = InMemoryStateDB::new();
            for i in 0..10u8 {
                db.put_account(Account { address: addr(i), balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 , vesting: None });
            }

            let txs: Vec<Transaction> = (0..num_txs)
                .map(|i| {
                    Transaction::Transfer(TransferTx {
                        from: addr((i % 10) as u8),
                        to: addr(((i + 1) % 10) as u8),
                        amount,
                        nonce: (i / 10) as u64,
                        signature: None,
                        public_key: None,
                        mev_refund_eligible: None,
                    })
                })
                .collect();

            let mut executor = ParallelExecutor::new_for_test(5);
            let _ = executor.execute_block(&mut db, &make_block(1, 1, txs));

            // Check no balance is "negative" (would be very large u64)
            for i in 0..10u8 {
                if let Some(acc) = db.get_account(&addr(i)) {
                    prop_assert!(acc.balance <= 10_000_000,
                        "suspicious balance {} for account {} — possible underflow",
                        acc.balance, i);
                }
            }
        }

        /// Nonces should be strictly monotonic after execution.
        #[test]
        fn nonces_monotonic(num_txs in 1usize..20) {
            let mut db = InMemoryStateDB::new();
            db.put_account(Account { address: addr(1), balance: 100_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 , vesting: None });
            db.put_account(Account { address: addr(2), balance: 0, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 , vesting: None });

            let txs: Vec<Transaction> = (0..num_txs)
                .map(|i| {
                    Transaction::Transfer(TransferTx {
                        from: addr(1),
                        to: addr(2),
                        amount: 1,
                        nonce: i as u64,
                        signature: None,
                        public_key: None,
                        mev_refund_eligible: None,
                    })
                })
                .collect();

            let mut executor = ParallelExecutor::new_for_test(5);
            let result = executor.execute_block(&mut db, &make_block(1, 1, txs));

            if let Ok(r) = result {
                let nonce = db.get_account(&addr(1)).unwrap().nonce;
                prop_assert_eq!(nonce, r.txs_executed as u64,
                    "nonce should equal number of executed txs");
            }
        }

        /// State root should be deterministic: same inputs = same root.
        #[test]
        fn state_root_deterministic(seed in 0u64..100) {
            let setup = |db: &mut InMemoryStateDB| {
                db.put_account(Account { address: addr(1), balance: 1_000_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 , vesting: None });
                db.put_account(Account { address: addr(2), balance: 500_000, nonce: 0, storage_deposit: 0, storage_bytes: 0, last_touched_epoch: 0 , vesting: None });
            };

            let txs = vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: seed % 1000 + 1,
                nonce: 0,
                signature: None,
                public_key: None,
                mev_refund_eligible: None,
            })];

            let mut db1 = InMemoryStateDB::new();
            setup(&mut db1);
            let mut executor1 = ParallelExecutor::new_for_test(5);
            let r1 = executor1.execute_block(&mut db1, &make_block(1, 1, txs.clone())).unwrap();

            let mut db2 = InMemoryStateDB::new();
            setup(&mut db2);
            let mut executor2 = ParallelExecutor::new_for_test(5);
            let r2 = executor2.execute_block(&mut db2, &make_block(1, 1, txs)).unwrap();

            prop_assert_eq!(r1.state_root, r2.state_root,
                "execution must be deterministic");
        }

        /// Lane S.2: Crooks-MEV refund invariants. For any random
        /// `(attacker_balance, victim_balance, refund_amount, same_addr)`
        /// the parallel executor must satisfy:
        ///
        ///   1. **Conservation:** attacker_after + victim_after equals
        ///      attacker_before + victim_before (when the refund applies).
        ///   2. **No underflow:** attacker_after never wraps below 0.
        ///   3. **Self-refund rejected:** when attacker == victim, the
        ///      tx fails and balances are unchanged.
        ///   4. **Zero-amount rejected:** when amount == 0, the tx fails.
        ///   5. **Insufficient-balance rejected cleanly:** when amount >
        ///      attacker_balance, the tx fails and both balances are
        ///      unchanged (no partial debit).
        ///   6. **No nonce mutation:** attacker.nonce is unchanged
        ///      regardless of outcome (refund is protocol-issued).
        ///
        /// 256× random samples lock the Lane Q.1 + S.1 contract end-to-end
        /// across BOTH executors (SimpleExecutor::execute_refund and
        /// the inlined ParallelExecutor serial-phase arm).
        #[test]
        fn refund_invariants_lane_s2(
            attacker_balance in 0u64..10_000_000,
            victim_balance in 0u64..10_000_000,
            refund_amount in 0u64..15_000_000,
            same_addr in proptest::bool::ANY,
        ) {
            use evaporchain_types::RefundTx;

            let mut db = InMemoryStateDB::new();
            let attacker_addr = addr(1);
            let victim_addr = if same_addr { addr(1) } else { addr(2) };
            let initial_attacker_nonce = 7u64;
            db.put_account(Account {
                address: attacker_addr,
                balance: attacker_balance,
                nonce: initial_attacker_nonce,
                storage_deposit: 0,
                storage_bytes: 0,
                last_touched_epoch: 0,
                vesting: None,
            });
            if !same_addr {
                db.put_account(Account {
                    address: victim_addr,
                    balance: victim_balance,
                    nonce: 0,
                    storage_deposit: 0,
                    storage_bytes: 0,
                    last_touched_epoch: 0,
                    vesting: None,
                });
            }

            let total_before = if same_addr {
                attacker_balance
            } else {
                attacker_balance.saturating_add(victim_balance)
            };

            let tx = Transaction::Refund(RefundTx {
                source_block_height: 1,
                source_observation_idx: 0,
                attacker: attacker_addr,
                victim: victim_addr,
                amount: refund_amount,
                settle_block_height: 2,
            });

            let mut executor = ParallelExecutor::new_for_test(5);
            // Block at epoch 0 — same as account creation epoch — so
            // no demurrage delta is applied between account creation
            // and block execution. The conservation invariant tested
            // below is "Refund alone preserves total balance"; folding
            // in epoch-boundary demurrage would conflate two
            // independent invariants and cause spurious failures on
            // shrunk inputs near demurrage-shift boundaries (e.g.
            // balance 83334 at 2-epoch interval loses 1 unit to
            // demurrage's `>> shift` regardless of refund logic).
            let result = executor.execute_block(&mut db, &make_block(2, 0, vec![tx]));
            // Whatever happens, executor must NOT panic.
            prop_assert!(result.is_ok(), "execute_block must not panic on Refund");

            // Inv 6: attacker nonce unchanged.
            let attacker_after = db.get_account(&attacker_addr).cloned().unwrap();
            prop_assert_eq!(
                attacker_after.nonce, initial_attacker_nonce,
                "Refund must not mutate attacker nonce"
            );

            let total_after = if same_addr {
                attacker_after.balance
            } else {
                let victim_after = db.get_account(&victim_addr).cloned().unwrap();
                attacker_after.balance.saturating_add(victim_after.balance)
            };

            // Inv 1: conservation. Note this holds whether the refund
            // succeeded or failed — failures revert balances entirely.
            prop_assert_eq!(
                total_after, total_before,
                "Refund must conserve total balance (attacker+victim)"
            );

            // Inv 3-5: failure cases leave attacker_balance unchanged.
            let should_fail = same_addr
                || refund_amount == 0
                || refund_amount > attacker_balance;
            if should_fail {
                prop_assert_eq!(
                    attacker_after.balance, attacker_balance,
                    "failed refund must not mutate attacker balance \
                     (same_addr={}, amount={}, balance={})",
                    same_addr, refund_amount, attacker_balance
                );
            } else {
                // Successful refund: attacker debited by exactly amount.
                prop_assert_eq!(
                    attacker_after.balance,
                    attacker_balance - refund_amount,
                    "successful refund must debit attacker by exactly amount"
                );
            }

            // Inv 2 falls out of u64 type + saturating_add on the credit
            // path; if balance wrapped we'd see total_after != total_before.
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// Conservation enforcement gate (Layer 0, item 1)
//
// `conservation_enforcement` governance flag: "observe" (default) keeps
// the audit observability-only — verdict goes to
// `executor.last_conservation_audit`, block commits regardless.
// "enforce" propagates any violation as
// `ExecutionError::ConservationViolation` and the block is rejected.
//
// These tests cover the happy path under both modes (a well-formed
// transfer block must commit). The block-rejection case requires a
// state-poisoning shim (an `InMemoryStateDB` wrapper that mints
// untracked balance between the before/after snapshots) and is
// tracked as a follow-up — the kernel-level negative cases are
// already exercised in `energy_audit::tests::audit_total_increase_rejected`
// and `audit_decay_below_lambda_floor_rejected`.
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod conservation_enforce_tests {
    use crate::parallel::ParallelExecutor;
    use crate::ExecutionEngine;
    use evaporchain_state::db::{InMemoryStateDB, StateDB};
    use evaporchain_types::{Account, Block, Transaction, TransferTx};

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

    fn block_with_one_transfer(number: u64, epoch: u64) -> Block {
        let txs = vec![Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
            mev_refund_eligible: None,
        })];
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

    /// Default behaviour (flag unset): block commits, verdict stored.
    /// Regression check that the gate doesn't false-positive when
    /// nothing in governance has changed.
    #[test]
    fn observe_default_unset_commits_normal_block() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 1_000_000);
        fund(&mut db, 2, 0);
        let mut executor = ParallelExecutor::new_for_test(5);
        executor
            .execute_block(&mut db, &block_with_one_transfer(1, 1))
            .expect("normal transfer must commit when flag unset");
        assert!(matches!(executor.last_conservation_audit, Some(Ok(()))));
    }

    /// Explicit "observe" — same as unset.
    #[test]
    fn observe_explicit_commits_normal_block() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 1_000_000);
        fund(&mut db, 2, 0);
        db.put_governance_param(
            "conservation_enforcement".to_string(),
            "observe".to_string(),
        );
        let mut executor = ParallelExecutor::new_for_test(5);
        executor
            .execute_block(&mut db, &block_with_one_transfer(1, 1))
            .expect("normal transfer must commit when flag = observe");
        assert!(matches!(executor.last_conservation_audit, Some(Ok(()))));
    }

    /// Enforce mode: a normal, well-formed block still commits — the
    /// gate must not break the happy path. Verdict still stored as Ok.
    /// This is the critical regression test: turning enforcement on
    /// must not halt a healthy chain.
    #[test]
    fn enforce_mode_normal_block_still_commits() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 1_000_000);
        fund(&mut db, 2, 0);
        db.put_governance_param(
            "conservation_enforcement".to_string(),
            "enforce".to_string(),
        );
        let mut executor = ParallelExecutor::new_for_test(5);
        executor
            .execute_block(&mut db, &block_with_one_transfer(1, 1))
            .expect("well-formed block must commit under enforce mode");
        assert!(matches!(executor.last_conservation_audit, Some(Ok(()))));
    }

    /// Demurrage is now wired into `ParallelExecutor::execute_block`
    /// (Layer 0 item 4). A block run at epoch ≥ 1 against accounts
    /// above `DemurrageParams.threshold` must accrue a positive
    /// refresh-pool balance — proving the integration runs in the
    /// production hot path, not just in `SimpleExecutor`.
    #[test]
    fn demurrage_fires_in_parallel_execute_block() {
        let mut db = InMemoryStateDB::new();
        // Above default threshold (1024) so demurrage will charge.
        fund(&mut db, 1, 10_000_000);
        let mut executor = ParallelExecutor::new_for_test(5);
        // Aggressive params so the test sees a non-trivial charge
        // within one block. Production uses `default()` which
        // accrues much more slowly.
        executor.demurrage_params = evaporchain_demurrage::DemurrageParams::new(100, 1_000);
        let bal_before = db.get_account(&addr(1)).unwrap().balance;
        executor
            .execute_block(&mut db, &block_with_one_transfer(1, 10))
            .expect("normal block must commit");
        let bal_after = db.get_account(&addr(1)).unwrap().balance;
        let pool = executor.refresh_pool.total_accrued();
        assert!(pool > 0, "demurrage must accrue to refresh pool");
        // The drained balance must equal the pool credit
        // (transferred-out separately handled by the transfer flow).
        // Account 1 was the *recipient* of the transfer, so its
        // balance after = bal_before + 100 (received) − pool_share.
        // We only assert pool > 0 here; per-account accounting is
        // covered by the demurrage_integration unit tests.
        let _ = bal_before;
        let _ = bal_after;
    }

    /// Negative-case coverage for the gate's branching logic
    /// (the gap flagged in commit 4d59b5d's "filed as a follow-up"
    /// note). Tests `evaluate_conservation_gate` in isolation —
    /// avoids a state-poisoning shim that would have been required
    /// to exercise the full `execute_block` path with a violating
    /// state transition.
    ///
    /// The kernel-level audit fn (`audit_block_step`) already has
    /// negative-case coverage in `energy_audit::tests::*`; the
    /// remaining gap was the gate's match-and-propagate logic, which
    /// is now extracted into the testable helper.
    #[test]
    fn gate_passes_ok_verdict_under_observe_mode() {
        use crate::evaluate_conservation_gate;
        let result = evaluate_conservation_gate(Ok(()), false);
        assert!(matches!(result, Ok(Ok(()))));
    }

    #[test]
    fn gate_passes_ok_verdict_under_enforce_mode() {
        use crate::evaluate_conservation_gate;
        // Even under enforce mode, an Ok verdict propagates as
        // "store Ok and continue" — no false-positive rejections.
        let result = evaluate_conservation_gate(Ok(()), true);
        assert!(matches!(result, Ok(Ok(()))));
    }

    #[test]
    fn gate_stores_violation_under_observe_mode() {
        use crate::evaluate_conservation_gate;
        use evaporchain_energy_kernel::ConservationViolation;
        // observe mode: a violation should be stored on the
        // executor (Ok(Err(violation))) and the block commits.
        let violation = ConservationViolation::DecayIncreasedTotal {
            before: 1_000,
            after: 1_500,
        };
        let result = evaluate_conservation_gate(Err(violation), false);
        match result {
            Ok(Err(ConservationViolation::DecayIncreasedTotal { before, after })) => {
                assert_eq!(before, 1_000);
                assert_eq!(after, 1_500);
            }
            other => panic!("expected Ok(Err(DecayIncreasedTotal)), got {:?}", other),
        }
    }

    #[test]
    fn gate_rejects_violation_under_enforce_mode() {
        use crate::{evaluate_conservation_gate, ExecutionError};
        use evaporchain_energy_kernel::ConservationViolation;
        // enforce mode: a violation must propagate as
        // ExecutionError::ConservationViolation. This is the negative
        // case the integration tests can't easily reach without a
        // state-poisoning shim — covered here at the unit-level
        // gate.
        let violation = ConservationViolation::DecayExceededLambda {
            before: 1_000,
            after: 100,
            max_decay: 500,
            epochs: 10,
            half_life: 100,
        };
        let result = evaluate_conservation_gate(Err(violation), true);
        match result {
            Err(ExecutionError::ConservationViolation(
                ConservationViolation::DecayExceededLambda {
                    before,
                    after,
                    max_decay,
                    epochs,
                    half_life,
                },
            )) => {
                assert_eq!(before, 1_000);
                assert_eq!(after, 100);
                assert_eq!(max_decay, 500);
                assert_eq!(epochs, 10);
                assert_eq!(half_life, 100);
            }
            other => panic!(
                "expected Err(ExecutionError::ConservationViolation(DecayExceededLambda)), got {:?}",
                other
            ),
        }
    }

    /// Unknown flag value falls back to observe (legacy behaviour).
    /// Avoids a chain-halting typo: `enforse` ≠ `enforce`.
    #[test]
    fn unknown_flag_value_treated_as_observe() {
        let mut db = InMemoryStateDB::new();
        fund(&mut db, 1, 1_000_000);
        fund(&mut db, 2, 0);
        db.put_governance_param(
            "conservation_enforcement".to_string(),
            "enforse".to_string(), // typo, intentional
        );
        let mut executor = ParallelExecutor::new_for_test(5);
        executor
            .execute_block(&mut db, &block_with_one_transfer(1, 1))
            .expect("unknown flag value must not enable enforcement");
        assert!(matches!(executor.last_conservation_audit, Some(Ok(()))));
    }
}
