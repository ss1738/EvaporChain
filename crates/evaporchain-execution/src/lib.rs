use evaporchain_crypto::signatures::{MlDsaVerifier, Verifier};
use evaporchain_state::db::StateDB;
use evaporchain_state::{EvaporationEngine, RefreshEngine};
use evaporchain_types::{
    Block, CreateObjectTx, Epoch, ObjectState, RefreshTx, StateObject, Transaction, TransferTx,
};
use thiserror::Error;
use tracing::{debug, info};

/// Errors that can occur during transaction execution.
#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("insufficient balance: account {account} has {available}, needs {required}")]
    InsufficientBalance {
        account: String,
        available: u64,
        required: u64,
    },
    #[error("invalid nonce: expected {expected}, got {got}")]
    InvalidNonce { expected: u64, got: u64 },
    #[error("object already exists: {0}")]
    ObjectAlreadyExists(String),
    #[error("object not found: {0}")]
    ObjectNotFound(String),
    #[error("self-transfer not allowed")]
    SelfTransfer,
    #[error("zero amount transfer")]
    ZeroAmount,
    #[error("refresh failed: {0}")]
    RefreshFailed(String),
    #[error("invalid signature")]
    InvalidSignature,
    #[error("missing signature")]
    MissingSignature,
}

/// Result of executing a single block.
#[derive(Debug)]
pub struct BlockExecutionResult {
    pub state_root: [u8; 32],
    pub txs_executed: usize,
    pub txs_failed: usize,
    pub objects_entered_grace: usize,
    pub objects_evaporated: usize,
}

/// Trait for block/transaction execution engines.
pub trait ExecutionEngine: Send + Sync {
    /// Execute all transactions in a block, returning the execution result.
    fn execute_block(
        &self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockExecutionResult, ExecutionError>;
}

/// Simple executor that processes transactions sequentially and runs
/// evaporation at the end of each block.
pub struct SimpleExecutor {
    evaporation_engine: EvaporationEngine,
    verify_signatures: bool,
}

impl SimpleExecutor {
    /// Create a new executor with the given grace period for evaporation.
    /// Signature verification is OFF by default (for backward compatibility).
    pub fn new(grace_period: u64) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            verify_signatures: false,
        }
    }

    /// Create a new executor with signature verification enabled.
    pub fn new_with_sig_verification(grace_period: u64) -> Self {
        Self {
            evaporation_engine: EvaporationEngine::new(grace_period),
            verify_signatures: true,
        }
    }

    /// Verify the ML-DSA signature on a transaction (if verification is enabled).
    fn verify_tx_signature(&self, tx: &Transaction) -> Result<(), ExecutionError> {
        if !self.verify_signatures {
            return Ok(());
        }

        let sig = tx.signature().ok_or(ExecutionError::MissingSignature)?;
        let pk = tx.public_key().ok_or(ExecutionError::MissingSignature)?;
        let msg = tx.signable_bytes();

        if !MlDsaVerifier::verify(&msg, sig, pk) {
            return Err(ExecutionError::InvalidSignature);
        }

        Ok(())
    }

    /// Execute a single transfer transaction.
    fn execute_transfer(
        &self,
        db: &mut dyn StateDB,
        tx: &TransferTx,
    ) -> Result<(), ExecutionError> {
        if tx.from == tx.to {
            return Err(ExecutionError::SelfTransfer);
        }
        if tx.amount == 0 {
            return Err(ExecutionError::ZeroAmount);
        }

        // Check sender nonce
        let sender = db.get_or_create_account(&tx.from);
        if sender.nonce != tx.nonce {
            return Err(ExecutionError::InvalidNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        if sender.balance < tx.amount {
            return Err(ExecutionError::InsufficientBalance {
                account: hex::encode(tx.from),
                available: sender.balance,
                required: tx.amount,
            });
        }

        // Debit sender
        sender.balance -= tx.amount;
        sender.nonce += 1;

        // Credit receiver
        let receiver = db.get_or_create_account(&tx.to);
        receiver.balance += tx.amount;

        debug!(
            from = hex::encode(tx.from),
            to = hex::encode(tx.to),
            amount = tx.amount,
            "Transfer executed"
        );

        Ok(())
    }

    /// Execute an object creation transaction.
    fn execute_create_object(
        &self,
        db: &mut dyn StateDB,
        tx: &CreateObjectTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        // Check if object already exists
        if db.get_object(&tx.object_id).is_some() {
            return Err(ExecutionError::ObjectAlreadyExists(hex::encode(tx.object_id)));
        }

        let obj = StateObject {
            id: tx.object_id,
            owner: tx.creator,
            energy: tx.energy,
            half_life: tx.half_life,
            created_at: epoch,
            last_refreshed: epoch,
            state: ObjectState::Active,
            grace_epoch: None,
            data: tx.data.clone(),
        };

        db.put_object(obj);

        debug!(
            object_id = hex::encode(tx.object_id),
            energy = tx.energy,
            half_life = tx.half_life,
            "Object created"
        );

        Ok(())
    }

    /// Execute an energy refresh transaction.
    fn execute_refresh(
        &self,
        db: &mut dyn StateDB,
        tx: &RefreshTx,
        epoch: Epoch,
    ) -> Result<(), ExecutionError> {
        // Try refresh on active/grace object first
        if db.get_object(&tx.object_id).is_some() {
            RefreshEngine::refresh(db, &tx.object_id, tx.energy_deposit, epoch)
                .map_err(|e| ExecutionError::RefreshFailed(e.to_string()))?;
            return Ok(());
        }

        // Try resurrection from ghost
        if db.get_ghost(&tx.object_id).is_some() {
            RefreshEngine::resurrect(db, &tx.object_id, tx.energy_deposit, epoch)
                .map_err(|e| ExecutionError::RefreshFailed(e.to_string()))?;
            return Ok(());
        }

        Err(ExecutionError::ObjectNotFound(hex::encode(tx.object_id)))
    }
}

impl ExecutionEngine for SimpleExecutor {
    fn execute_block(
        &self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockExecutionResult, ExecutionError> {
        let mut txs_executed = 0;
        let mut txs_failed = 0;

        // Execute transactions
        for tx in &block.transactions {
            // Signature verification (if enabled)
            if let Err(e) = self.verify_tx_signature(tx) {
                debug!(error = %e, "Signature verification failed");
                txs_failed += 1;
                continue;
            }

            let result = match tx {
                Transaction::Transfer(transfer) => self.execute_transfer(db, transfer),
                Transaction::CreateObject(create) => {
                    self.execute_create_object(db, create, block.epoch)
                }
                Transaction::Refresh(refresh) => self.execute_refresh(db, refresh, block.epoch),
            };

            match result {
                Ok(()) => txs_executed += 1,
                Err(e) => {
                    debug!(error = %e, "Transaction failed");
                    txs_failed += 1;
                }
            }
        }

        // Run evaporation at end of block
        let evap_result = self.evaporation_engine.process_epoch(db, block.epoch);

        let state_root = db.compute_state_root();

        info!(
            block = block.number,
            epoch = block.epoch,
            txs_executed,
            txs_failed,
            entered_grace = evap_result.entered_grace.len(),
            evaporated = evap_result.evaporated.len(),
            state_root = hex::encode(state_root),
            "Block executed"
        );

        Ok(BlockExecutionResult {
            state_root,
            txs_executed,
            txs_failed,
            objects_entered_grace: evap_result.entered_grace.len(),
            objects_evaporated: evap_result.evaporated.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::Account;

    fn addr(byte: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = byte;
        a
    }

    fn obj_id(byte: u8) -> [u8; 32] {
        let mut id = [0u8; 32];
        id[0] = byte;
        id
    }

    fn make_block(number: u64, epoch: Epoch, txs: Vec<Transaction>) -> Block {
        Block {
            number,
            epoch,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: txs,
            timestamp: 0,
        }
    }

    fn fund_account(db: &mut InMemoryStateDB, byte: u8, balance: u64) {
        db.put_account(Account {
            address: addr(byte),
            balance,
            nonce: 0,
        });
    }

    /// Helper: sign a transaction with the given keypair.
    fn sign_tx(tx: &mut Transaction, kp: &MlDsaKeypair) {
        let msg = tx.signable_bytes();
        let sig = kp.sign(&msg);
        let pk = kp.public_key_bytes();
        match tx {
            Transaction::Transfer(t) => {
                t.signature = Some(sig);
                t.public_key = Some(pk);
            }
            Transaction::Refresh(r) => {
                r.signature = Some(sig);
                r.public_key = Some(pk);
            }
            Transaction::CreateObject(c) => {
                c.signature = Some(sig);
                c.public_key = Some(pk);
            }
        }
    }

    // ─── Basic Transfer ───

    #[test]
    fn test_basic_transfer() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let executor = SimpleExecutor::new(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 300,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(result.txs_failed, 0);

        let sender = db.get_account(&addr(1)).unwrap();
        assert_eq!(sender.balance, 700);
        assert_eq!(sender.nonce, 1);

        let receiver = db.get_account(&addr(2)).unwrap();
        assert_eq!(receiver.balance, 300);
    }

    // ─── Insufficient Balance ───

    #[test]
    fn test_insufficient_balance() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 100);

        let executor = SimpleExecutor::new(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 500,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);

        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 100);
        assert!(db.get_account(&addr(2)).is_none());
    }

    // ─── Self-Transfer Rejected ───

    #[test]
    fn test_self_transfer_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let executor = SimpleExecutor::new(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(1),
                amount: 100,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
    }

    // ─── Invalid Nonce ───

    #[test]
    fn test_invalid_nonce() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let executor = SimpleExecutor::new(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 100,
                nonce: 5,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
    }

    // ─── Object Creation with Energy ───

    #[test]
    fn test_create_object_with_energy() {
        let mut db = InMemoryStateDB::new();
        let executor = SimpleExecutor::new(7);

        let block = make_block(
            1,
            10,
            vec![Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(42),
                energy: 5000,
                half_life: 100,
                data: vec![0xDE, 0xAD],
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);

        let obj = db.get_object(&obj_id(42)).unwrap();
        assert_eq!(obj.energy, 5000);
        assert_eq!(obj.half_life, 100);
        assert_eq!(obj.created_at, 10);
        assert_eq!(obj.last_refreshed, 10);
        assert_eq!(obj.state, ObjectState::Active);
        assert_eq!(obj.data, vec![0xDE, 0xAD]);
        assert_eq!(obj.owner, addr(1));
    }

    // ─── Duplicate Object Creation Fails ───

    #[test]
    fn test_duplicate_object_creation_fails() {
        let mut db = InMemoryStateDB::new();
        let executor = SimpleExecutor::new(7);

        let create_tx = Transaction::CreateObject(CreateObjectTx {
            creator: addr(1),
            object_id: obj_id(42),
            energy: 1000,
            half_life: 50,
            data: vec![],
            signature: None,
            public_key: None,
        });

        let block1 = make_block(1, 1, vec![create_tx.clone()]);
        let block2 = make_block(2, 2, vec![create_tx]);

        executor.execute_block(&mut db, &block1).unwrap();
        let result = executor.execute_block(&mut db, &block2).unwrap();
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.object_count(), 1);
    }

    // ─── Block Execution with Multiple Txs ───

    #[test]
    fn test_block_with_multiple_txs() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);
        fund_account(&mut db, 2, 5_000);

        let executor = SimpleExecutor::new(7);
        let block = make_block(
            1,
            1,
            vec![
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(2),
                    amount: 2000,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(2),
                    to: addr(3),
                    amount: 1000,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                }),
                Transaction::CreateObject(CreateObjectTx {
                    creator: addr(1),
                    object_id: obj_id(10),
                    energy: 500,
                    half_life: 50,
                    data: vec![1],
                    signature: None,
                    public_key: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(3),
                    amount: 500,
                    nonce: 1,
                    signature: None,
                    public_key: None,
                }),
            ],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 4);
        assert_eq!(result.txs_failed, 0);

        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 7500);
        assert_eq!(db.get_account(&addr(1)).unwrap().nonce, 2);
        assert_eq!(db.get_account(&addr(2)).unwrap().balance, 6000);
        assert_eq!(db.get_account(&addr(3)).unwrap().balance, 1500);
        assert_eq!(db.object_count(), 1);
        assert_ne!(result.state_root, [0u8; 32]);
    }

    // ─── Partial Block Failure ───

    #[test]
    fn test_partial_block_failure() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 500);

        let executor = SimpleExecutor::new(7);
        let block = make_block(
            1,
            1,
            vec![
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(2),
                    amount: 200,
                    nonce: 0,
                    signature: None,
                    public_key: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(3),
                    amount: 400,
                    nonce: 1,
                    signature: None,
                    public_key: None,
                }),
                Transaction::Transfer(TransferTx {
                    from: addr(1),
                    to: addr(4),
                    amount: 100,
                    nonce: 1,
                    signature: None,
                    public_key: None,
                }),
            ],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 2);
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 200);
    }

    // ─── Evaporation Triggered by Block Execution ───

    #[test]
    fn test_evaporation_triggered_by_block() {
        let mut db = InMemoryStateDB::new();

        db.put_object(StateObject {
            id: obj_id(1),
            owner: addr(1),
            energy: 4,
            half_life: 1,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![0xAB],
        });

        let executor = SimpleExecutor::new(3);

        let block1 = make_block(1, 3, vec![]);
        let r1 = executor.execute_block(&mut db, &block1).unwrap();
        assert_eq!(r1.objects_entered_grace, 1);
        assert_eq!(r1.objects_evaporated, 0);
        assert_eq!(db.object_count(), 1);
        assert_eq!(db.get_object(&obj_id(1)).unwrap().state, ObjectState::Grace);

        let block2 = make_block(2, 5, vec![]);
        let r2 = executor.execute_block(&mut db, &block2).unwrap();
        assert_eq!(r2.objects_evaporated, 0);

        let block3 = make_block(3, 6, vec![]);
        let r3 = executor.execute_block(&mut db, &block3).unwrap();
        assert_eq!(r3.objects_evaporated, 1);
        assert_eq!(db.object_count(), 0);
        assert_eq!(db.ghost_count(), 1);

        let ghost = db.get_ghost(&obj_id(1)).unwrap();
        assert_eq!(ghost.evaporated_at, 6);
        assert_eq!(ghost.original_data, vec![0xAB]);
    }

    // ─── Refresh Saves Object from Evaporation ───

    #[test]
    fn test_refresh_saves_object_from_evaporation() {
        let mut db = InMemoryStateDB::new();

        db.put_object(StateObject {
            id: obj_id(1),
            owner: addr(1),
            energy: 4,
            half_life: 1,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
        });

        let executor = SimpleExecutor::new(5);

        let block1 = make_block(1, 3, vec![]);
        let r1 = executor.execute_block(&mut db, &block1).unwrap();
        assert_eq!(r1.objects_entered_grace, 1);

        let block2 = make_block(
            2,
            4,
            vec![Transaction::Refresh(RefreshTx {
                object_id: obj_id(1),
                energy_deposit: 10_000,
                signature: None,
                public_key: None,
            })],
        );
        let r2 = executor.execute_block(&mut db, &block2).unwrap();
        assert_eq!(r2.txs_executed, 1);

        let obj = db.get_object(&obj_id(1)).unwrap();
        assert_eq!(obj.state, ObjectState::Active);
        assert_eq!(obj.energy, 10_000);
        assert_eq!(obj.last_refreshed, 4);

        let block3 = make_block(3, 10, vec![]);
        let r3 = executor.execute_block(&mut db, &block3).unwrap();
        assert_eq!(r3.objects_entered_grace, 0);
        assert_eq!(r3.objects_evaporated, 0);
    }

    // ─── Resurrection via Refresh ───

    #[test]
    fn test_resurrection_via_refresh_in_block() {
        let mut db = InMemoryStateDB::new();

        db.put_ghost(evaporchain_types::GhostRecord {
            object_id: obj_id(1),
            owner: addr(1),
            evaporated_at: 50,
            data_hash: [0u8; 32],
            original_data: vec![0xCA, 0xFE],
        });

        let executor = SimpleExecutor::new(5);
        let block = make_block(
            10,
            60,
            vec![Transaction::Refresh(RefreshTx {
                object_id: obj_id(1),
                energy_deposit: 8000,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(db.ghost_count(), 0);
        assert_eq!(db.object_count(), 1);

        let obj = db.get_object(&obj_id(1)).unwrap();
        assert_eq!(obj.state, ObjectState::Resurrected);
        assert_eq!(obj.energy, 8000);
        assert_eq!(obj.data, vec![0xCA, 0xFE]);
    }

    // ─── State Root Changes Between Blocks ───

    #[test]
    fn test_state_root_changes_between_blocks() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 10_000);

        let executor = SimpleExecutor::new(7);

        let block1 = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 500,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );
        let r1 = executor.execute_block(&mut db, &block1).unwrap();

        let block2 = make_block(
            2,
            2,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(3),
                amount: 300,
                nonce: 1,
                signature: None,
                public_key: None,
            })],
        );
        let r2 = executor.execute_block(&mut db, &block2).unwrap();

        assert_ne!(r1.state_root, r2.state_root);
        assert_ne!(r1.state_root, [0u8; 32]);
        assert_ne!(r2.state_root, [0u8; 32]);
    }

    // ─── Zero Amount Transfer Rejected ───

    #[test]
    fn test_zero_amount_transfer_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let executor = SimpleExecutor::new(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 0,
                nonce: 0,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
    }

    // ─── Refresh Nonexistent Object Fails ───

    #[test]
    fn test_refresh_nonexistent_object_fails() {
        let mut db = InMemoryStateDB::new();

        let executor = SimpleExecutor::new(7);
        let block = make_block(
            1,
            1,
            vec![Transaction::Refresh(RefreshTx {
                object_id: obj_id(99),
                energy_deposit: 1000,
                signature: None,
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
    }

    // ═══════════════════ Signature Verification Tests ═══════════════════

    #[test]
    fn test_signed_transfer_succeeds() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let executor = SimpleExecutor::new_with_sig_verification(7);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 200,
            nonce: 0,
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx, &kp);

        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(result.txs_failed, 0);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 800);
    }

    #[test]
    fn test_unsigned_tx_rejected_when_verification_on() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let executor = SimpleExecutor::new_with_sig_verification(7);

        let block = make_block(
            1,
            1,
            vec![Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 200,
                nonce: 0,
                signature: None, // no signature
                public_key: None,
            })],
        );

        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
        // Balance unchanged
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
    }

    #[test]
    fn test_invalid_signature_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let executor = SimpleExecutor::new_with_sig_verification(7);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 200,
            nonce: 0,
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx, &kp);

        // Tamper with the signature
        if let Transaction::Transfer(ref mut t) = tx {
            if let Some(ref mut sig) = t.signature {
                sig[0] ^= 0xFF;
            }
        }

        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 0);
        assert_eq!(result.txs_failed, 1);
        assert_eq!(db.get_account(&addr(1)).unwrap().balance, 1000);
    }

    #[test]
    fn test_wrong_key_signature_rejected() {
        let mut db = InMemoryStateDB::new();
        fund_account(&mut db, 1, 1000);

        let executor = SimpleExecutor::new_with_sig_verification(7);
        let kp1 = MlDsaKeypair::generate();
        let kp2 = MlDsaKeypair::generate();

        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 200,
            nonce: 0,
            signature: None,
            public_key: None,
        });
        // Sign with kp1 but replace public key with kp2's
        sign_tx(&mut tx, &kp1);
        if let Transaction::Transfer(ref mut t) = tx {
            t.public_key = Some(kp2.public_key_bytes());
        }

        let block = make_block(1, 1, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_failed, 1);
    }

    #[test]
    fn test_signed_create_object_succeeds() {
        let mut db = InMemoryStateDB::new();
        let executor = SimpleExecutor::new_with_sig_verification(7);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::CreateObject(CreateObjectTx {
            creator: addr(1),
            object_id: obj_id(42),
            energy: 5000,
            half_life: 100,
            data: vec![0xDE, 0xAD],
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx, &kp);

        let block = make_block(1, 10, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
        assert_eq!(db.object_count(), 1);
    }

    #[test]
    fn test_signed_refresh_succeeds() {
        let mut db = InMemoryStateDB::new();
        db.put_object(StateObject {
            id: obj_id(1),
            owner: addr(1),
            energy: 100,
            half_life: 10,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![],
        });

        let executor = SimpleExecutor::new_with_sig_verification(7);
        let kp = MlDsaKeypair::generate();

        let mut tx = Transaction::Refresh(RefreshTx {
            object_id: obj_id(1),
            energy_deposit: 500,
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx, &kp);

        let block = make_block(1, 5, vec![tx]);
        let result = executor.execute_block(&mut db, &block).unwrap();
        assert_eq!(result.txs_executed, 1);
    }
}
