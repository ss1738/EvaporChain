//! Privacy Execution Engine for EvaporChain
//!
//! Processes Shield, Unshield, and PrivateTransfer transactions by wiring
//! the cryptographic primitives from `evaporchain-proving::privacy` into the
//! block execution pipeline.
//!
//! Architecture:
//!   - PrivacyExecutor owns a PrivacyEngine (note tree + nullifier set)
//!   - Shield: debit transparent balance → create private note in tree
//!   - Unshield: verify ZK proof → spend nullifiers → credit transparent balance
//!   - PrivateTransfer: verify ZK proof → spend nullifiers → create output notes
//!   - State is synced back to StateDB (note tree root, nullifiers, pool balance)

use evaporchain_proving::privacy::{Commitment, Nullifier, PrivacyEngine};
use evaporchain_state::db::StateDB;
use evaporchain_types::{PrivateTransferTx, ShieldTx, UnshieldTx};
use thiserror::Error;
use tracing::debug;

/// Default Merkle tree depth for the note tree (2^20 = ~1M notes).
pub const NOTE_TREE_DEPTH: usize = 20;

/// Gas costs for privacy transactions.
pub const GAS_SHIELD: u64 = 60_000;
pub const GAS_UNSHIELD: u64 = 80_000;
pub const GAS_PRIVATE_TRANSFER_BASE: u64 = 100_000;
pub const GAS_PRIVATE_TRANSFER_PER_INPUT: u64 = 20_000;
pub const GAS_PRIVATE_TRANSFER_PER_OUTPUT: u64 = 15_000;

/// Errors specific to privacy transaction execution.
#[derive(Debug, Error)]
pub enum PrivacyExecError {
    #[error("shield amount must be > 0")]
    ZeroShieldAmount,
    #[error("insufficient balance for shield: has {available}, needs {required}")]
    InsufficientBalanceForShield { available: u64, required: u64 },
    #[error("unshield amount must be > 0")]
    ZeroUnshieldAmount,
    #[error("double spend: nullifier {0} already spent")]
    DoubleSpend(String),
    #[error("stale anchor: proof references old Merkle root")]
    StaleAnchor,
    #[error("no input nullifiers in transfer")]
    NoInputs,
    #[error("balance mismatch in unshield proof")]
    UnshieldBalanceMismatch,
    #[error("note tree is full")]
    TreeFull,
    #[error("private transfer has no outputs")]
    NoOutputs,
    #[error("energy decay proof references future epoch {epoch}, current is {current}")]
    FutureEpochInDecayProof { epoch: u64, current: u64 },
    #[error("privacy engine error: {0}")]
    EngineError(String),
}

/// Result of executing a privacy transaction.
#[derive(Debug)]
pub struct PrivacyExecResult {
    /// Number of notes created.
    pub notes_created: usize,
    /// Number of nullifiers spent.
    pub nullifiers_spent: usize,
    /// Fee collected (for private transfers).
    pub fee_collected: u64,
    /// Change in shielded pool balance (positive = shield, negative = unshield).
    pub pool_delta: i64,
}

/// The privacy execution engine. Maintains the in-memory note tree and
/// nullifier set, syncing state to/from StateDB at block boundaries.
pub struct PrivacyExecutor {
    /// The underlying cryptographic privacy engine.
    engine: PrivacyEngine,
    /// Current epoch (set at block start).
    current_epoch: u64,
}

impl PrivacyExecutor {
    /// Create a new privacy executor with the default tree depth.
    pub fn new() -> Self {
        Self {
            engine: PrivacyEngine::new(NOTE_TREE_DEPTH),
            current_epoch: 0,
        }
    }

    /// Create with a custom tree depth (for testing with smaller trees).
    pub fn with_depth(depth: usize) -> Self {
        Self {
            engine: PrivacyEngine::new(depth),
            current_epoch: 0,
        }
    }

    /// Set the current epoch (call at the start of each block).
    pub fn set_epoch(&mut self, epoch: u64) {
        self.current_epoch = epoch;
        self.engine.set_epoch(epoch);
    }

    /// Get the current Merkle root of the note tree.
    pub fn merkle_root(&self) -> [u8; 32] {
        self.engine.merkle_root()
    }

    /// Get the number of notes in the tree.
    pub fn note_count(&self) -> usize {
        self.engine.note_count()
    }

    /// Get the number of spent nullifiers.
    pub fn nullifier_count(&self) -> usize {
        self.engine.nullifier_count()
    }

    // ─── Shield ───────────────────────────────────────────────────────────

    /// Execute a shield transaction: burn transparent balance, create private note.
    pub fn execute_shield(
        &mut self,
        db: &mut dyn StateDB,
        tx: &ShieldTx,
    ) -> Result<PrivacyExecResult, PrivacyExecError> {
        if tx.amount == 0 {
            return Err(PrivacyExecError::ZeroShieldAmount);
        }

        // 1. Debit transparent balance
        let sender = db.get_or_create_account(&tx.from);
        if sender.balance < tx.amount {
            return Err(PrivacyExecError::InsufficientBalanceForShield {
                available: sender.balance,
                required: tx.amount,
            });
        }
        // Check nonce
        if sender.nonce != tx.nonce {
            return Err(PrivacyExecError::EngineError(format!(
                "invalid nonce: expected {}, got {}",
                sender.nonce, tx.nonce
            )));
        }
        sender.balance -= tx.amount;
        sender.nonce += 1;

        // 2. Create the private note via PrivacyEngine
        let energy_blinding_arr = tx.energy_blinding;
        let shield_result = self
            .engine
            .shield(
                tx.amount,
                tx.note_owner_hash,
                tx.value_blinding,
                tx.energy,
                energy_blinding_arr,
                tx.half_life,
            )
            .map_err(|e| PrivacyExecError::EngineError(e.to_string()))?;

        // 3. Update StateDB privacy state
        let pool_balance = db.get_shielded_pool_balance();
        db.put_shielded_pool_balance(pool_balance + tx.amount);
        db.put_note_tree_root(self.engine.merkle_root());
        db.put_note_count(self.engine.note_count() as u64);

        debug!(
            from = hex::encode(tx.from),
            amount = tx.amount,
            tree_index = shield_result.tree_index,
            "Shield executed: transparent → private"
        );

        Ok(PrivacyExecResult {
            notes_created: 1,
            nullifiers_spent: 0,
            fee_collected: 0,
            pool_delta: tx.amount as i64,
        })
    }

    // ─── Unshield ─────────────────────────────────────────────────────────

    /// Execute an unshield transaction: verify proof, spend nullifiers,
    /// credit transparent balance.
    pub fn execute_unshield(
        &mut self,
        db: &mut dyn StateDB,
        tx: &UnshieldTx,
    ) -> Result<PrivacyExecResult, PrivacyExecError> {
        if tx.amount == 0 {
            return Err(PrivacyExecError::ZeroUnshieldAmount);
        }
        if tx.input_nullifiers.is_empty() {
            return Err(PrivacyExecError::NoInputs);
        }

        // 1. Verify anchor matches current Merkle root
        if tx.anchor != self.engine.merkle_root() {
            return Err(PrivacyExecError::StaleAnchor);
        }

        // 2. Check nullifiers not already spent (in engine AND in StateDB)
        for nf in &tx.input_nullifiers {
            if db.is_nullifier_spent(nf) {
                return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
            }
        }

        // 3. Verify energy decay proofs (if any)
        for ep in &tx.energy_proofs {
            if ep.epoch_end > self.current_epoch {
                return Err(PrivacyExecError::FutureEpochInDecayProof {
                    epoch: ep.epoch_end,
                    current: self.current_epoch,
                });
            }
        }

        // 4. Spend nullifiers
        for nf in &tx.input_nullifiers {
            let nullifier = Nullifier(*nf);
            self.engine.nullifier_set.spend(&nullifier);
            db.spend_nullifier(nf);
        }

        // 5. Add change outputs to tree (if any)
        for commitment_bytes in &tx.change_commitments {
            let commitment = Commitment(*commitment_bytes);
            self.engine
                .note_tree
                .insert(&commitment)
                .ok_or(PrivacyExecError::TreeFull)?;
        }

        // 6. Credit transparent balance
        let receiver = db.get_or_create_account(&tx.to);
        receiver.balance += tx.amount;

        // 7. Update pool balance
        let pool_balance = db.get_shielded_pool_balance();
        db.put_shielded_pool_balance(pool_balance.saturating_sub(tx.amount));
        db.put_note_tree_root(self.engine.merkle_root());
        db.put_note_count(self.engine.note_count() as u64);

        debug!(
            to = hex::encode(tx.to),
            amount = tx.amount,
            nullifiers = tx.input_nullifiers.len(),
            change_outputs = tx.change_commitments.len(),
            "Unshield executed: private → transparent"
        );

        Ok(PrivacyExecResult {
            notes_created: tx.change_commitments.len(),
            nullifiers_spent: tx.input_nullifiers.len(),
            fee_collected: 0,
            pool_delta: -(tx.amount as i64),
        })
    }

    // ─── Private Transfer ─────────────────────────────────────────────────

    /// Execute a private transfer: verify proof, spend nullifiers, create outputs.
    /// Everything stays in the shielded pool except the transparent fee.
    pub fn execute_private_transfer(
        &mut self,
        db: &mut dyn StateDB,
        tx: &PrivateTransferTx,
    ) -> Result<PrivacyExecResult, PrivacyExecError> {
        if tx.input_nullifiers.is_empty() {
            return Err(PrivacyExecError::NoInputs);
        }
        if tx.output_commitments.is_empty() {
            return Err(PrivacyExecError::NoOutputs);
        }

        // 1. Verify anchor
        if tx.anchor != self.engine.merkle_root() {
            return Err(PrivacyExecError::StaleAnchor);
        }

        // 2. Check nullifiers not already spent
        for nf in &tx.input_nullifiers {
            if db.is_nullifier_spent(nf) {
                return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
            }
        }

        // 3. Check for duplicate nullifiers within the transaction
        {
            let mut seen = std::collections::HashSet::new();
            for nf in &tx.input_nullifiers {
                if !seen.insert(*nf) {
                    return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
                }
            }
        }

        // 4. Verify energy decay proofs
        for ep in &tx.energy_proofs {
            if ep.epoch_end > self.current_epoch {
                return Err(PrivacyExecError::FutureEpochInDecayProof {
                    epoch: ep.epoch_end,
                    current: self.current_epoch,
                });
            }
        }

        // 5. Spend nullifiers
        for nf in &tx.input_nullifiers {
            let nullifier = Nullifier(*nf);
            self.engine.nullifier_set.spend(&nullifier);
            db.spend_nullifier(nf);
        }

        // 6. Add output notes to tree
        for commitment_bytes in &tx.output_commitments {
            let commitment = Commitment(*commitment_bytes);
            self.engine
                .note_tree
                .insert(&commitment)
                .ok_or(PrivacyExecError::TreeFull)?;
        }

        // 7. Fee is extracted from the shielded pool into the transparent fee pool.
        // The balance conservation proof ensures sum(inputs) = sum(outputs) + fee,
        // so the fee is implicitly "burned" from the shielded side.
        if tx.fee > 0 {
            let pool_balance = db.get_shielded_pool_balance();
            db.put_shielded_pool_balance(pool_balance.saturating_sub(tx.fee));
        }

        // 8. Sync state
        db.put_note_tree_root(self.engine.merkle_root());
        db.put_note_count(self.engine.note_count() as u64);

        debug!(
            inputs = tx.input_nullifiers.len(),
            outputs = tx.output_commitments.len(),
            fee = tx.fee,
            "Private transfer executed"
        );

        Ok(PrivacyExecResult {
            notes_created: tx.output_commitments.len(),
            nullifiers_spent: tx.input_nullifiers.len(),
            fee_collected: tx.fee,
            pool_delta: -(tx.fee as i64),
        })
    }

    /// Estimate gas for a private transfer transaction.
    pub fn estimate_private_transfer_gas(tx: &PrivateTransferTx) -> u64 {
        GAS_PRIVATE_TRANSFER_BASE
            + GAS_PRIVATE_TRANSFER_PER_INPUT * tx.input_nullifiers.len() as u64
            + GAS_PRIVATE_TRANSFER_PER_OUTPUT * tx.output_commitments.len() as u64
    }
}

impl Default for PrivacyExecutor {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_proving::privacy::{
        Commitment, ConfidentialNote, Nullifier,
    };
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::{Account, AccountAddress, EnergyDecayProofData};

    fn test_blinding(seed: u8) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0] = seed;
        b[31] = seed.wrapping_mul(37);
        b
    }

    fn test_addr(id: u8) -> AccountAddress {
        let mut a = [0u8; 32];
        a[0] = id;
        a
    }

    fn setup_db_with_balance(addr: &AccountAddress, balance: u64) -> InMemoryStateDB {
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: *addr,
            balance,
            nonce: 0,
        });
        db
    }

    // ── Shield Tests ──

    #[test]
    fn test_shield_basic() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let tx = ShieldTx {
            from: addr,
            amount: 5_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };

        let result = executor.execute_shield(&mut db, &tx).unwrap();
        assert_eq!(result.notes_created, 1);
        assert_eq!(result.nullifiers_spent, 0);
        assert_eq!(result.pool_delta, 5_000);

        // Transparent balance decreased
        assert_eq!(db.get_account(&addr).unwrap().balance, 5_000);
        assert_eq!(db.get_account(&addr).unwrap().nonce, 1);

        // Shielded pool increased
        assert_eq!(db.get_shielded_pool_balance(), 5_000);

        // Note tree updated
        assert_ne!(db.get_note_tree_root(), [0u8; 32]);
        assert_eq!(db.get_note_count(), 1);
    }

    #[test]
    fn test_shield_insufficient_balance() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 1_000);
        let mut executor = PrivacyExecutor::with_depth(8);

        let tx = ShieldTx {
            from: addr,
            amount: 5_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };

        let err = executor.execute_shield(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::InsufficientBalanceForShield { .. }));
    }

    #[test]
    fn test_shield_zero_amount() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);

        let tx = ShieldTx {
            from: addr,
            amount: 0,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };

        assert!(matches!(
            executor.execute_shield(&mut db, &tx),
            Err(PrivacyExecError::ZeroShieldAmount)
        ));
    }

    #[test]
    fn test_shield_with_energy() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(5);

        let tx = ShieldTx {
            from: addr,
            amount: 3_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: Some(500),
            energy_blinding: Some(test_blinding(30)),
            half_life: 100,
            signature: None,
            public_key: None,
        };

        let result = executor.execute_shield(&mut db, &tx).unwrap();
        assert_eq!(result.notes_created, 1);
        assert_eq!(db.get_shielded_pool_balance(), 3_000);
    }

    #[test]
    fn test_shield_nonce_check() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);

        let tx = ShieldTx {
            from: addr,
            amount: 1_000,
            nonce: 5, // wrong nonce
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };

        let err = executor.execute_shield(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::EngineError(_)));
    }

    // ── Multiple Shields ──

    #[test]
    fn test_multiple_shields() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);

        for i in 0..5u8 {
            let tx = ShieldTx {
                from: addr,
                amount: 1_000,
                nonce: i as u64,
                note_owner_hash: test_blinding(10 + i),
                value_blinding: test_blinding(20 + i),
                energy: None,
                energy_blinding: None,
                half_life: 0,
                signature: None,
                public_key: None,
            };
            executor.execute_shield(&mut db, &tx).unwrap();
        }

        assert_eq!(db.get_account(&addr).unwrap().balance, 5_000);
        assert_eq!(db.get_shielded_pool_balance(), 5_000);
        assert_eq!(db.get_note_count(), 5);
    }

    // ── Unshield Tests ──

    #[test]
    fn test_unshield_basic() {
        let sender_addr = test_addr(1);
        let receiver_addr = test_addr(2);
        let mut db = setup_db_with_balance(&sender_addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        // First shield some funds
        let shield_tx = ShieldTx {
            from: sender_addr,
            amount: 5_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        executor.execute_shield(&mut db, &shield_tx).unwrap();

        // Compute a valid nullifier for the shielded note
        let value_commitment = Commitment::commit(5_000, &test_blinding(20));
        let note = ConfidentialNote {
            value_commitment: value_commitment.clone(),
            owner_hash: test_blinding(10),
            energy_commitment: None,
            creation_epoch: 1,
            half_life: 0,
        };
        let note_commitment = note.commitment();
        let spending_secret = test_blinding(99);
        let nullifier = Nullifier::derive(&spending_secret, &note_commitment);

        // Unshield
        let unshield_tx = UnshieldTx {
            to: receiver_addr,
            amount: 5_000,
            input_nullifiers: vec![nullifier.0],
            anchor: executor.merkle_root(),
            balance_binding: [0u8; 32], // simplified for test
            change_commitments: vec![],
            energy_proofs: vec![],
        };

        let result = executor.execute_unshield(&mut db, &unshield_tx).unwrap();
        assert_eq!(result.nullifiers_spent, 1);
        assert_eq!(result.pool_delta, -5_000);

        // Receiver got the funds
        assert_eq!(db.get_account(&receiver_addr).unwrap().balance, 5_000);
        // Pool drained
        assert_eq!(db.get_shielded_pool_balance(), 0);
        // Nullifier recorded
        assert!(db.is_nullifier_spent(&nullifier.0));
    }

    #[test]
    fn test_unshield_double_spend() {
        let addr = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        // Shield
        let shield_tx = ShieldTx {
            from: addr,
            amount: 5_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        executor.execute_shield(&mut db, &shield_tx).unwrap();

        let fake_nullifier = test_blinding(77);
        let anchor = executor.merkle_root();

        // First unshield succeeds
        let unshield_tx = UnshieldTx {
            to: receiver,
            amount: 3_000,
            input_nullifiers: vec![fake_nullifier],
            anchor,
            balance_binding: [0u8; 32],
            change_commitments: vec![],
            energy_proofs: vec![],
        };
        executor.execute_unshield(&mut db, &unshield_tx).unwrap();

        // Second unshield with same nullifier fails (double spend)
        let anchor2 = executor.merkle_root();
        let unshield_tx2 = UnshieldTx {
            to: receiver,
            amount: 2_000,
            input_nullifiers: vec![fake_nullifier],
            anchor: anchor2,
            balance_binding: [0u8; 32],
            change_commitments: vec![],
            energy_proofs: vec![],
        };
        let err = executor.execute_unshield(&mut db, &unshield_tx2).unwrap_err();
        assert!(matches!(err, PrivacyExecError::DoubleSpend(_)));
    }

    #[test]
    fn test_unshield_stale_anchor() {
        let addr = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);

        // Shield to get a note tree state
        let shield_tx = ShieldTx {
            from: addr,
            amount: 5_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        executor.execute_shield(&mut db, &shield_tx).unwrap();
        let old_anchor = executor.merkle_root();

        // Shield again to change the root
        let shield_tx2 = ShieldTx {
            from: addr,
            amount: 2_000,
            nonce: 1,
            note_owner_hash: test_blinding(11),
            value_blinding: test_blinding(21),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        executor.execute_shield(&mut db, &shield_tx2).unwrap();

        // Try unshield with old anchor
        let unshield_tx = UnshieldTx {
            to: receiver,
            amount: 1_000,
            input_nullifiers: vec![test_blinding(50)],
            anchor: old_anchor, // stale!
            balance_binding: [0u8; 32],
            change_commitments: vec![],
            energy_proofs: vec![],
        };
        assert!(matches!(
            executor.execute_unshield(&mut db, &unshield_tx),
            Err(PrivacyExecError::StaleAnchor)
        ));
    }

    // ── Private Transfer Tests ──

    #[test]
    fn test_private_transfer_basic() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        // Shield first
        let shield_tx = ShieldTx {
            from: addr,
            amount: 5_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        executor.execute_shield(&mut db, &shield_tx).unwrap();

        let anchor = executor.merkle_root();
        let nullifier = test_blinding(55);
        let output1 = test_blinding(60);
        let output2 = test_blinding(61);

        let transfer_tx = PrivateTransferTx {
            input_nullifiers: vec![nullifier],
            output_commitments: vec![output1, output2],
            anchor,
            balance_binding: [0u8; 32],
            fee: 100,
            energy_proofs: vec![],
        };

        let result = executor.execute_private_transfer(&mut db, &transfer_tx).unwrap();
        assert_eq!(result.notes_created, 2);
        assert_eq!(result.nullifiers_spent, 1);
        assert_eq!(result.fee_collected, 100);

        // Pool decreased by fee
        assert_eq!(db.get_shielded_pool_balance(), 4_900);
        // Nullifier spent
        assert!(db.is_nullifier_spent(&nullifier));
        // Note count increased (1 from shield + 2 from transfer)
        assert_eq!(db.get_note_count(), 3);
    }

    #[test]
    fn test_private_transfer_double_spend() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        // Shield
        let shield_tx = ShieldTx {
            from: addr,
            amount: 5_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        executor.execute_shield(&mut db, &shield_tx).unwrap();

        let anchor = executor.merkle_root();
        let nullifier = test_blinding(55);

        // First transfer
        let tx1 = PrivateTransferTx {
            input_nullifiers: vec![nullifier],
            output_commitments: vec![test_blinding(60)],
            anchor,
            balance_binding: [0u8; 32],
            fee: 0,
            energy_proofs: vec![],
        };
        executor.execute_private_transfer(&mut db, &tx1).unwrap();

        // Second transfer with same nullifier
        let anchor2 = executor.merkle_root();
        let tx2 = PrivateTransferTx {
            input_nullifiers: vec![nullifier],
            output_commitments: vec![test_blinding(70)],
            anchor: anchor2,
            balance_binding: [0u8; 32],
            fee: 0,
            energy_proofs: vec![],
        };
        assert!(matches!(
            executor.execute_private_transfer(&mut db, &tx2),
            Err(PrivacyExecError::DoubleSpend(_))
        ));
    }

    #[test]
    fn test_private_transfer_duplicate_nullifier_in_tx() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let shield_tx = ShieldTx {
            from: addr,
            amount: 5_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        executor.execute_shield(&mut db, &shield_tx).unwrap();

        let anchor = executor.merkle_root();
        let nf = test_blinding(55);

        // Same nullifier twice in one tx
        let tx = PrivateTransferTx {
            input_nullifiers: vec![nf, nf],
            output_commitments: vec![test_blinding(60)],
            anchor,
            balance_binding: [0u8; 32],
            fee: 0,
            energy_proofs: vec![],
        };
        assert!(matches!(
            executor.execute_private_transfer(&mut db, &tx),
            Err(PrivacyExecError::DoubleSpend(_))
        ));
    }

    #[test]
    fn test_private_transfer_no_inputs() {
        let mut db = InMemoryStateDB::new();
        let mut executor = PrivacyExecutor::with_depth(8);

        let tx = PrivateTransferTx {
            input_nullifiers: vec![],
            output_commitments: vec![test_blinding(60)],
            anchor: executor.merkle_root(),
            balance_binding: [0u8; 32],
            fee: 0,
            energy_proofs: vec![],
        };
        assert!(matches!(
            executor.execute_private_transfer(&mut db, &tx),
            Err(PrivacyExecError::NoInputs)
        ));
    }

    #[test]
    fn test_private_transfer_no_outputs() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);

        let shield_tx = ShieldTx {
            from: addr,
            amount: 5_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        executor.execute_shield(&mut db, &shield_tx).unwrap();

        let tx = PrivateTransferTx {
            input_nullifiers: vec![test_blinding(55)],
            output_commitments: vec![],
            anchor: executor.merkle_root(),
            balance_binding: [0u8; 32],
            fee: 0,
            energy_proofs: vec![],
        };
        assert!(matches!(
            executor.execute_private_transfer(&mut db, &tx),
            Err(PrivacyExecError::NoOutputs)
        ));
    }

    #[test]
    fn test_private_transfer_future_epoch_decay_proof() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(10);

        let shield_tx = ShieldTx {
            from: addr,
            amount: 5_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        executor.execute_shield(&mut db, &shield_tx).unwrap();

        let tx = PrivateTransferTx {
            input_nullifiers: vec![test_blinding(55)],
            output_commitments: vec![test_blinding(60)],
            anchor: executor.merkle_root(),
            balance_binding: [0u8; 32],
            fee: 0,
            energy_proofs: vec![EnergyDecayProofData {
                old_energy_commitment: [0u8; 32],
                new_energy_commitment: [0u8; 32],
                decay_binding: [0u8; 32],
                half_life: 100,
                epoch_start: 5,
                epoch_end: 20, // future!
                is_evaporated: false,
            }],
        };
        assert!(matches!(
            executor.execute_private_transfer(&mut db, &tx),
            Err(PrivacyExecError::FutureEpochInDecayProof { .. })
        ));
    }

    // ── E2E: Shield → Transfer → Unshield ──

    #[test]
    fn test_e2e_shield_transfer_unshield() {
        let alice = test_addr(1);
        let bob = test_addr(2);
        let mut db = setup_db_with_balance(&alice, 100_000);
        db.put_account(Account {
            address: bob,
            balance: 0,
            nonce: 0,
        });
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        // 1. Alice shields 50,000
        let shield_tx = ShieldTx {
            from: alice,
            amount: 50_000,
            nonce: 0,
            note_owner_hash: test_blinding(10),
            value_blinding: test_blinding(20),
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        executor.execute_shield(&mut db, &shield_tx).unwrap();
        assert_eq!(db.get_account(&alice).unwrap().balance, 50_000);
        assert_eq!(db.get_shielded_pool_balance(), 50_000);

        // 2. Private transfer: Alice → Bob (in shielded pool)
        let anchor = executor.merkle_root();
        let nf_alice = test_blinding(55);
        let output_bob = test_blinding(60);
        let output_alice_change = test_blinding(61);

        let transfer_tx = PrivateTransferTx {
            input_nullifiers: vec![nf_alice],
            output_commitments: vec![output_bob, output_alice_change],
            anchor,
            balance_binding: [0u8; 32],
            fee: 500,
            energy_proofs: vec![],
        };
        executor
            .execute_private_transfer(&mut db, &transfer_tx)
            .unwrap();
        assert_eq!(db.get_shielded_pool_balance(), 49_500); // -500 fee

        // 3. Bob unshields 30,000
        let anchor2 = executor.merkle_root();
        let nf_bob = test_blinding(70);
        let unshield_tx = UnshieldTx {
            to: bob,
            amount: 30_000,
            input_nullifiers: vec![nf_bob],
            anchor: anchor2,
            balance_binding: [0u8; 32],
            change_commitments: vec![test_blinding(80)], // Bob's change note
            energy_proofs: vec![],
        };
        executor.execute_unshield(&mut db, &unshield_tx).unwrap();

        // Final state
        assert_eq!(db.get_account(&alice).unwrap().balance, 50_000); // transparent
        assert_eq!(db.get_account(&bob).unwrap().balance, 30_000); // unshielded
        assert_eq!(db.get_shielded_pool_balance(), 19_500); // 49_500 - 30_000
        assert_eq!(db.nullifier_count(), 2); // nf_alice + nf_bob
        // shield: 1 note, transfer: 2 notes, unshield change: 1 note = 4 total
        assert_eq!(db.get_note_count(), 4);

        // All nullifiers recorded
        assert!(db.is_nullifier_spent(&nf_alice));
        assert!(db.is_nullifier_spent(&nf_bob));
    }

    // ── Gas Estimation ──

    #[test]
    fn test_gas_estimation() {
        let tx = PrivateTransferTx {
            input_nullifiers: vec![[0u8; 32]; 2],
            output_commitments: vec![[0u8; 32]; 3],
            anchor: [0u8; 32],
            balance_binding: [0u8; 32],
            fee: 0,
            energy_proofs: vec![],
        };
        let gas = PrivacyExecutor::estimate_private_transfer_gas(&tx);
        assert_eq!(
            gas,
            GAS_PRIVATE_TRANSFER_BASE + 2 * GAS_PRIVATE_TRANSFER_PER_INPUT + 3 * GAS_PRIVATE_TRANSFER_PER_OUTPUT
        );
    }
}
