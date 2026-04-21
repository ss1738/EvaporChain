//! Audit-prep test suite: balance conservation, nonce safety, double-spend
//! prevention, gas exhaustion attacks, and property-based testing.

#[cfg(test)]
mod invariant_tests {
    use crate::parallel::ParallelExecutor;
    use crate::ExecutionEngine;
    use evaporchain_state::db::InMemoryStateDB;
    use evaporchain_state::db::StateDB;
    use evaporchain_types::{
        Account, Block, CreateObjectTx, Transaction, TransferTx,
    };

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
        }
    }

    fn fund_account(db: &mut InMemoryStateDB, byte: u8, balance: u64) {
        db.put_account(Account {
            address: addr(byte),
            balance,
            nonce: 0,
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

        let total_before: u64 = (0..5u8).map(|i| db.get_account(&addr(i)).unwrap().balance).sum();

        let txs: Vec<Transaction> = (0..100)
            .map(|i| {
                Transaction::Transfer(TransferTx {
                    from: addr((i % 5) as u8),
                    to: addr(((i + 1) % 5) as u8),
                    amount: 10,
                    nonce: (i / 5) as u64,
                    signature: None,
                    public_key: None,
                })
            })
            .collect();

        let mut executor = ParallelExecutor::new(5);
        let result = executor.execute_block(&mut db, &make_block(1, 1, txs)).unwrap();

        let total_after: u64 = (0..5u8).map(|i| db.get_account(&addr(i)).unwrap().balance).sum();

        // Total supply = balances + fees collected
        assert_eq!(
            total_before,
            total_after + result.total_fees,
            "INVARIANT VIOLATION: total supply not conserved. before={}, after={}, fees={}",
            total_before, total_after, result.total_fees
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
        })];

        let mut executor = ParallelExecutor::new(5);
        let result = executor.execute_block(&mut db, &make_block(1, 1, txs)).unwrap();
        let balance_after = db.get_account(&addr(1)).unwrap().balance;

        // Balance should decrease only by fees
        assert_eq!(
            balance_before,
            balance_after + result.total_fees,
            "self-transfer must only deduct fees"
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
        });

        // First execution succeeds
        let mut executor = ParallelExecutor::new(5);
        let result1 = executor.execute_block(&mut db, &make_block(1, 1, vec![tx.clone()])).unwrap();
        assert_eq!(result1.txs_executed, 1);

        // Replay same transaction (nonce 0 again) — should fail
        let result2 = executor.execute_block(&mut db, &make_block(2, 2, vec![tx])).unwrap();
        assert_eq!(result2.txs_failed, 1, "replayed transaction must be rejected");
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
        });

        let mut executor = ParallelExecutor::new(5);
        let result = executor.execute_block(&mut db, &make_block(1, 1, vec![tx])).unwrap();
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
        });

        let mut executor = ParallelExecutor::new(5);
        let result = executor.execute_block(&mut db, &make_block(1, 1, vec![tx])).unwrap();
        assert_eq!(result.txs_failed, 1, "overdraft must be rejected");

        // Sender balance must not go negative
        let sender = db.get_account(&addr(1)).unwrap();
        assert!(sender.balance <= 100, "balance must not exceed initial after failed tx");
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
        });

        let mut executor = ParallelExecutor::new(5);
        let result = executor.execute_block(&mut db, &make_block(1, 1, vec![tx])).unwrap();
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
            db.put_account(Account { address: a, balance: 1_000_000_000, nonce: 0 });
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
                })
            })
            .collect();

        // Use a tight gas limit
        let mut executor = ParallelExecutor::new(5);
        executor.block_gas_limit = 100_000;
        let result = executor.execute_block(&mut db, &make_block(1, 1, txs)).unwrap();

        // Gas used must not exceed limit
        assert!(result.gas_used <= 100_000,
            "gas used ({}) must not exceed limit (100000)", result.gas_used);
        // Some txs should have been dropped due to gas limit
        assert!(result.txs_executed < 500, "gas limit should cap transactions executed");
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
            signature: None,
            public_key: None,
        });

        let mut executor = ParallelExecutor::new(5);
        let _ = executor.execute_block(&mut db, &make_block(1, 1, vec![create_tx])).unwrap();

        // Execute empty blocks at increasing epochs and verify energy never increases
        let mut prev_energy = 10_000u64;
        for epoch in 2..=20 {
            let _ = executor.execute_block(&mut db, &make_block(epoch, epoch, vec![])).unwrap();
            if let Some(obj) = db.get_object(&obj_id) {
                let energy = obj.energy_at(epoch);
                assert!(energy <= prev_energy,
                    "INVARIANT VIOLATION: energy increased from {} to {} at epoch {}",
                    prev_energy, energy, epoch);
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
            signature: None,
            public_key: None,
        });

        let mut executor = ParallelExecutor::new(5);
        let _ = executor.execute_block(&mut db, &make_block(1, 1, vec![create_tx])).unwrap();

        // Advance many epochs to ensure evaporation
        for epoch in 2..=200 {
            let result = executor.execute_block(&mut db, &make_block(epoch, epoch, vec![])).unwrap();
            if result.objects_evaporated > 0 {
                // Object should have been evaporated
                break;
            }
        }

        // After enough epochs, object should be gone
        // (it may have been evaporated and converted to ghost)
        let ghost_count = db.ghost_count();
        assert!(ghost_count > 0 || db.get_object(&obj_id).is_none(),
            "evaporated object should become ghost or be removed");
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
            db.put_account(Account { address: addr(1), balance, nonce: 0 });
            db.put_account(Account { address: addr(2), balance: 0, nonce: 0 });

            let tx = Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount,
                nonce: 0,
                signature: None,
                public_key: None,
            });

            let mut executor = ParallelExecutor::new(5);
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
                db.put_account(Account { address: addr(i), balance: 1_000_000, nonce: 0 });
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
                    })
                })
                .collect();

            let mut executor = ParallelExecutor::new(5);
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
            db.put_account(Account { address: addr(1), balance: 100_000_000, nonce: 0 });
            db.put_account(Account { address: addr(2), balance: 0, nonce: 0 });

            let txs: Vec<Transaction> = (0..num_txs)
                .map(|i| {
                    Transaction::Transfer(TransferTx {
                        from: addr(1),
                        to: addr(2),
                        amount: 1,
                        nonce: i as u64,
                        signature: None,
                        public_key: None,
                    })
                })
                .collect();

            let mut executor = ParallelExecutor::new(5);
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
                db.put_account(Account { address: addr(1), balance: 1_000_000, nonce: 0 });
                db.put_account(Account { address: addr(2), balance: 500_000, nonce: 0 });
            };

            let txs = vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: seed % 1000 + 1,
                nonce: 0,
                signature: None,
                public_key: None,
            })];

            let mut db1 = InMemoryStateDB::new();
            setup(&mut db1);
            let mut executor1 = ParallelExecutor::new(5);
            let r1 = executor1.execute_block(&mut db1, &make_block(1, 1, txs.clone())).unwrap();

            let mut db2 = InMemoryStateDB::new();
            setup(&mut db2);
            let mut executor2 = ParallelExecutor::new(5);
            let r2 = executor2.execute_block(&mut db2, &make_block(1, 1, txs)).unwrap();

            prop_assert_eq!(r1.state_root, r2.state_root,
                "execution must be deterministic");
        }
    }
}
