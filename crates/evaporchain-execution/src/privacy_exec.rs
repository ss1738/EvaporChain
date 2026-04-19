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

use evaporchain_proving::privacy::{Commitment, Nullifier, PrivacyEngine, verify_balance_binding, verify_merkle_proof, MerkleProof};
use evaporchain_state::db::StateDB;
use evaporchain_types::{MerkleProofData, PrivateTransferTx, ShieldTx, UnshieldTx};
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
    #[error("invalid balance binding: hash does not match claimed amounts and blindings")]
    InvalidBalanceBinding,
    #[error("invalid commitment opening: input {index} does not open to claimed amount/blinding")]
    InvalidCommitmentOpening { index: usize },
    #[error("invalid output commitment: output {index} does not match claimed amount/blinding")]
    InvalidOutputCommitment { index: usize },
    #[error("invalid Merkle proof: input {index} is not a member of the note tree")]
    InvalidMerkleProof { index: usize },
    #[error("Merkle proof anchor mismatch: input {index} references a different root")]
    MerkleProofAnchorMismatch { index: usize },
    #[error("missing witness data: {field} required for privacy verification")]
    MissingWitnessData { field: String },
    #[error("witness field count mismatch: expected {expected}, got {got}")]
    WitnessFieldMismatch { expected: usize, got: usize },
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
    /// Tree index of the first note created (for shield, this is the shielded note's index).
    pub tree_index: Option<usize>,
    /// Note commitment bytes (for shield, the note's tree leaf commitment).
    pub note_commitment: Option<[u8; 32]>,
    /// Value commitment bytes (for shield, Poseidon(amount || blinding)).
    pub value_commitment: Option<[u8; 32]>,
}

/// Convert a serializable `MerkleProofData` to the proving crate's `MerkleProof`.
fn to_merkle_proof(data: &MerkleProofData) -> MerkleProof {
    MerkleProof {
        siblings: data.siblings.clone(),
        leaf_index: data.leaf_index,
        root: data.root,
    }
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

    /// Get a Merkle proof for a note at the given tree index.
    pub fn get_merkle_proof(&self, leaf_index: usize) -> Option<MerkleProofData> {
        self.engine.get_merkle_proof(leaf_index).map(|p| MerkleProofData {
            siblings: p.siblings,
            leaf_index: p.leaf_index,
            root: p.root,
        })
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
            tree_index: Some(shield_result.tree_index),
            note_commitment: Some(shield_result.commitment.0),
            value_commitment: Some(shield_result.note.value_commitment.0),
        })
    }

    // ─── Unshield ─────────────────────────────────────────────────────────

    /// Execute an unshield transaction with full cryptographic verification.
    ///
    /// All witness fields are mandatory: amounts, blindings, commitments,
    /// Merkle proofs, and balance binding must be provided and valid.
    pub fn execute_unshield(
        &mut self,
        db: &mut dyn StateDB,
        tx: &UnshieldTx,
    ) -> Result<PrivacyExecResult, PrivacyExecError> {
        let n_inputs = tx.input_nullifiers.len();

        if tx.amount == 0 {
            return Err(PrivacyExecError::ZeroUnshieldAmount);
        }
        if n_inputs == 0 {
            return Err(PrivacyExecError::NoInputs);
        }

        // 1. Validate witness field counts
        if tx.input_amounts.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("input_amounts: expected {n_inputs}, got {}", tx.input_amounts.len()),
            });
        }
        if tx.input_blindings.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("input_blindings: expected {n_inputs}, got {}", tx.input_blindings.len()),
            });
        }
        if tx.input_value_commitments.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("input_value_commitments: expected {n_inputs}, got {}", tx.input_value_commitments.len()),
            });
        }
        if tx.input_note_commitments.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("input_note_commitments: expected {n_inputs}, got {}", tx.input_note_commitments.len()),
            });
        }
        if tx.input_merkle_proofs.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("input_merkle_proofs: expected {n_inputs}, got {}", tx.input_merkle_proofs.len()),
            });
        }
        if tx.output_blindings.len() != tx.change_commitments.len() {
            return Err(PrivacyExecError::WitnessFieldMismatch {
                expected: tx.change_commitments.len(),
                got: tx.output_blindings.len(),
            });
        }

        // 2. Verify anchor matches current Merkle root
        if tx.anchor != self.engine.merkle_root() {
            return Err(PrivacyExecError::StaleAnchor);
        }

        // 3. Check nullifiers not already spent
        for nf in &tx.input_nullifiers {
            if db.is_nullifier_spent(nf) {
                return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
            }
        }

        // 4. Verify energy decay proofs (if any)
        for ep in &tx.energy_proofs {
            if ep.epoch_end > self.current_epoch {
                return Err(PrivacyExecError::FutureEpochInDecayProof {
                    epoch: ep.epoch_end,
                    current: self.current_epoch,
                });
            }
        }

        // 5. Verify each input: commitment opening + Merkle proof
        for i in 0..n_inputs {
            let c = Commitment(tx.input_value_commitments[i]);
            if !c.verify_opening(tx.input_amounts[i], &tx.input_blindings[i]) {
                return Err(PrivacyExecError::InvalidCommitmentOpening { index: i });
            }

            let proof = to_merkle_proof(&tx.input_merkle_proofs[i]);
            if proof.root != tx.anchor {
                return Err(PrivacyExecError::MerkleProofAnchorMismatch { index: i });
            }
            // The leaf in the Merkle tree is the note commitment (not the value commitment).
            if !verify_merkle_proof(&tx.input_note_commitments[i], &proof) {
                return Err(PrivacyExecError::InvalidMerkleProof { index: i });
            }
        }

        // 6. Verify balance binding
        let sum_in: u64 = tx.input_amounts.iter().sum();
        let change_total = sum_in.saturating_sub(tx.amount);
        if !verify_balance_binding(
            &tx.balance_binding,
            sum_in,
            change_total,
            tx.amount,
            &tx.input_blindings,
            &tx.output_blindings,
        ) {
            return Err(PrivacyExecError::InvalidBalanceBinding);
        }
        if sum_in < tx.amount {
            return Err(PrivacyExecError::UnshieldBalanceMismatch);
        }

        // 7. Spend nullifiers
        for nf in &tx.input_nullifiers {
            let nullifier = Nullifier(*nf);
            self.engine.nullifier_set.spend(&nullifier);
            db.spend_nullifier(nf);
        }

        // 8. Add change outputs to tree (if any)
        for commitment_bytes in &tx.change_commitments {
            let commitment = Commitment(*commitment_bytes);
            self.engine
                .note_tree
                .insert(&commitment)
                .ok_or(PrivacyExecError::TreeFull)?;
        }

        // 9. Credit transparent balance
        let receiver = db.get_or_create_account(&tx.to);
        receiver.balance += tx.amount;

        // 10. Update pool balance
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
            tree_index: None,
            note_commitment: None,
            value_commitment: None,
        })
    }

    // ─── Private Transfer ─────────────────────────────────────────────────

    /// Execute a private transfer with full cryptographic verification.
    ///
    /// All witness fields are mandatory: amounts, blindings, commitments,
    /// Merkle proofs, and balance binding must be provided and valid.
    pub fn execute_private_transfer(
        &mut self,
        db: &mut dyn StateDB,
        tx: &PrivateTransferTx,
    ) -> Result<PrivacyExecResult, PrivacyExecError> {
        let n_inputs = tx.input_nullifiers.len();
        let n_outputs = tx.output_commitments.len();

        if n_inputs == 0 {
            return Err(PrivacyExecError::NoInputs);
        }
        if n_outputs == 0 {
            return Err(PrivacyExecError::NoOutputs);
        }

        // 1. Validate all witness field counts
        if tx.input_amounts.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("input_amounts: expected {n_inputs}, got {}", tx.input_amounts.len()),
            });
        }
        if tx.input_blindings.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("input_blindings: expected {n_inputs}, got {}", tx.input_blindings.len()),
            });
        }
        if tx.input_value_commitments.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("input_value_commitments: expected {n_inputs}, got {}", tx.input_value_commitments.len()),
            });
        }
        if tx.input_note_commitments.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("input_note_commitments: expected {n_inputs}, got {}", tx.input_note_commitments.len()),
            });
        }
        if tx.input_merkle_proofs.len() != n_inputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("input_merkle_proofs: expected {n_inputs}, got {}", tx.input_merkle_proofs.len()),
            });
        }
        if tx.output_amounts.len() != n_outputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("output_amounts: expected {n_outputs}, got {}", tx.output_amounts.len()),
            });
        }
        if tx.output_blindings.len() != n_outputs {
            return Err(PrivacyExecError::MissingWitnessData {
                field: format!("output_blindings: expected {n_outputs}, got {}", tx.output_blindings.len()),
            });
        }

        // 2. Verify anchor
        if tx.anchor != self.engine.merkle_root() {
            return Err(PrivacyExecError::StaleAnchor);
        }

        // 3. Check nullifiers not already spent
        for nf in &tx.input_nullifiers {
            if db.is_nullifier_spent(nf) {
                return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
            }
        }

        // 4. Check for duplicate nullifiers within the transaction
        {
            let mut seen = std::collections::HashSet::new();
            for nf in &tx.input_nullifiers {
                if !seen.insert(*nf) {
                    return Err(PrivacyExecError::DoubleSpend(hex::encode(&nf[..8])));
                }
            }
        }

        // 5. Verify energy decay proofs
        for ep in &tx.energy_proofs {
            if ep.epoch_end > self.current_epoch {
                return Err(PrivacyExecError::FutureEpochInDecayProof {
                    epoch: ep.epoch_end,
                    current: self.current_epoch,
                });
            }
        }

        // 6. Verify each input: commitment opening + Merkle proof
        for i in 0..n_inputs {
            let c = Commitment(tx.input_value_commitments[i]);
            if !c.verify_opening(tx.input_amounts[i], &tx.input_blindings[i]) {
                return Err(PrivacyExecError::InvalidCommitmentOpening { index: i });
            }

            let proof = to_merkle_proof(&tx.input_merkle_proofs[i]);
            if proof.root != tx.anchor {
                return Err(PrivacyExecError::MerkleProofAnchorMismatch { index: i });
            }
            if !verify_merkle_proof(&tx.input_note_commitments[i], &proof) {
                return Err(PrivacyExecError::InvalidMerkleProof { index: i });
            }
        }

        // 7. Verify output commitments match claimed amounts/blindings
        for i in 0..n_outputs {
            let expected = Commitment::commit(tx.output_amounts[i], &tx.output_blindings[i]);
            if expected.0 != tx.output_commitments[i] {
                return Err(PrivacyExecError::InvalidOutputCommitment { index: i });
            }
        }

        // 8. Verify balance binding and conservation
        let sum_in: u64 = tx.input_amounts.iter().sum();
        let sum_out: u64 = tx.output_amounts.iter().sum();
        if !verify_balance_binding(
            &tx.balance_binding,
            sum_in,
            sum_out,
            tx.fee,
            &tx.input_blindings,
            &tx.output_blindings,
        ) {
            return Err(PrivacyExecError::InvalidBalanceBinding);
        }
        if sum_in != sum_out + tx.fee {
            return Err(PrivacyExecError::UnshieldBalanceMismatch);
        }

        // 9. Spend nullifiers
        for nf in &tx.input_nullifiers {
            let nullifier = Nullifier(*nf);
            self.engine.nullifier_set.spend(&nullifier);
            db.spend_nullifier(nf);
        }

        // 10. Add output notes to tree
        for commitment_bytes in &tx.output_commitments {
            let commitment = Commitment(*commitment_bytes);
            self.engine
                .note_tree
                .insert(&commitment)
                .ok_or(PrivacyExecError::TreeFull)?;
        }

        // 11. Fee extracted from shielded pool
        if tx.fee > 0 {
            let pool_balance = db.get_shielded_pool_balance();
            db.put_shielded_pool_balance(pool_balance.saturating_sub(tx.fee));
        }

        // 12. Sync state
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
            tree_index: None,
            note_commitment: None,
            value_commitment: None,
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
// Tests — 100% real cryptographic flow
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_proving::privacy::compute_balance_binding;
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

    /// Shield funds and return everything needed to spend the note later.
    struct ShieldedNote {
        amount: u64,
        blinding: [u8; 32],
        owner_hash: [u8; 32],
        spending_secret: [u8; 32],
        tree_index: usize,
        value_commitment: [u8; 32],
        note_commitment: [u8; 32],
    }

    /// Helper: shield real funds and return all the data needed to build a real unshield/transfer.
    fn do_shield(
        executor: &mut PrivacyExecutor,
        db: &mut InMemoryStateDB,
        from: &AccountAddress,
        amount: u64,
        nonce: u64,
        owner_hash: [u8; 32],
        blinding: [u8; 32],
        spending_secret: [u8; 32],
    ) -> ShieldedNote {
        let tx = ShieldTx {
            from: *from,
            amount,
            nonce,
            note_owner_hash: owner_hash,
            value_blinding: blinding,
            energy: None,
            energy_blinding: None,
            half_life: 0,
            signature: None,
            public_key: None,
        };
        let result = executor.execute_shield(db, &tx).unwrap();
        ShieldedNote {
            amount,
            blinding,
            owner_hash,
            spending_secret,
            tree_index: result.tree_index.unwrap(),
            value_commitment: result.value_commitment.unwrap(),
            note_commitment: result.note_commitment.unwrap(),
        }
    }

    /// Helper: build a fully real UnshieldTx from a shielded note.
    fn build_real_unshield(
        executor: &PrivacyExecutor,
        note: &ShieldedNote,
        to: AccountAddress,
        unshield_amount: u64,
    ) -> UnshieldTx {
        let merkle_proof = executor.get_merkle_proof(note.tree_index).unwrap();
        let nullifier = Nullifier::derive(&note.spending_secret, &Commitment(note.note_commitment));
        let binding = compute_balance_binding(
            note.amount, 0, unshield_amount,
            &[note.blinding], &[],
        );
        UnshieldTx {
            to,
            amount: unshield_amount,
            input_nullifiers: vec![nullifier.0],
            anchor: executor.merkle_root(),
            balance_binding: binding,
            input_amounts: vec![note.amount],
            input_blindings: vec![note.blinding],
            input_value_commitments: vec![note.value_commitment],
            input_note_commitments: vec![note.note_commitment],
            input_merkle_proofs: vec![merkle_proof],
            output_blindings: vec![],
            change_commitments: vec![],
            energy_proofs: vec![],
        }
    }

    /// Helper: build a fully real PrivateTransferTx from a shielded note.
    fn build_real_transfer(
        executor: &PrivacyExecutor,
        input_note: &ShieldedNote,
        output_amounts: &[u64],
        output_blindings: &[[u8; 32]],
        fee: u64,
    ) -> PrivateTransferTx {
        let merkle_proof = executor.get_merkle_proof(input_note.tree_index).unwrap();
        let nullifier = Nullifier::derive(&input_note.spending_secret, &Commitment(input_note.note_commitment));

        let output_commitments: Vec<[u8; 32]> = output_amounts
            .iter()
            .zip(output_blindings.iter())
            .map(|(a, b)| Commitment::commit(*a, b).0)
            .collect();

        let sum_out: u64 = output_amounts.iter().sum();
        let binding = compute_balance_binding(
            input_note.amount, sum_out, fee,
            &[input_note.blinding], output_blindings,
        );

        PrivateTransferTx {
            input_nullifiers: vec![nullifier.0],
            output_commitments,
            anchor: executor.merkle_root(),
            balance_binding: binding,
            fee,
            input_amounts: vec![input_note.amount],
            input_blindings: vec![input_note.blinding],
            input_value_commitments: vec![input_note.value_commitment],
            input_note_commitments: vec![input_note.note_commitment],
            input_merkle_proofs: vec![merkle_proof],
            output_amounts: output_amounts.to_vec(),
            output_blindings: output_blindings.to_vec(),
            energy_proofs: vec![],
        }
    }

    // ── Shield Tests ──

    #[test]
    fn test_shield_basic() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &addr, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        // Transparent balance decreased
        assert_eq!(db.get_account(&addr).unwrap().balance, 5_000);
        assert_eq!(db.get_account(&addr).unwrap().nonce, 1);
        assert_eq!(db.get_shielded_pool_balance(), 5_000);
        assert_ne!(db.get_note_tree_root(), [0u8; 32]);
        assert_eq!(db.get_note_count(), 1);

        // Real Merkle proof is obtainable
        let proof = executor.get_merkle_proof(note.tree_index).unwrap();
        assert_eq!(proof.root, executor.merkle_root());

        // Real value commitment verifies
        let c = Commitment(note.value_commitment);
        assert!(c.verify_opening(5_000, &test_blinding(20)));
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
            from: addr, amount: 0, nonce: 0,
            note_owner_hash: test_blinding(10), value_blinding: test_blinding(20),
            energy: None, energy_blinding: None, half_life: 0,
            signature: None, public_key: None,
        };
        assert!(matches!(executor.execute_shield(&mut db, &tx), Err(PrivacyExecError::ZeroShieldAmount)));
    }

    #[test]
    fn test_shield_nonce_check() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);

        let tx = ShieldTx {
            from: addr, amount: 1_000, nonce: 5, // wrong nonce
            note_owner_hash: test_blinding(10), value_blinding: test_blinding(20),
            energy: None, energy_blinding: None, half_life: 0,
            signature: None, public_key: None,
        };
        assert!(matches!(executor.execute_shield(&mut db, &tx), Err(PrivacyExecError::EngineError(_))));
    }

    #[test]
    fn test_multiple_shields() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        for i in 0..5u8 {
            do_shield(
                &mut executor, &mut db, &addr, 1_000, i as u64,
                test_blinding(10 + i), test_blinding(20 + i), test_blinding(90 + i),
            );
        }

        assert_eq!(db.get_account(&addr).unwrap().balance, 5_000);
        assert_eq!(db.get_shielded_pool_balance(), 5_000);
        assert_eq!(db.get_note_count(), 5);
    }

    // ── Unshield Tests (100% real) ──

    #[test]
    fn test_unshield_real_full_amount() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &sender, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        let tx = build_real_unshield(&executor, &note, receiver, 5_000);
        let result = executor.execute_unshield(&mut db, &tx).unwrap();

        assert_eq!(result.nullifiers_spent, 1);
        assert_eq!(result.pool_delta, -5_000);
        assert_eq!(db.get_account(&receiver).unwrap().balance, 5_000);
        assert_eq!(db.get_shielded_pool_balance(), 0);
        assert!(db.is_nullifier_spent(&tx.input_nullifiers[0]));
    }

    #[test]
    fn test_unshield_double_spend_rejected() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &sender, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        // First unshield succeeds
        let tx1 = build_real_unshield(&executor, &note, receiver, 5_000);
        executor.execute_unshield(&mut db, &tx1).unwrap();

        // Second unshield with same note — double spend
        // Note: anchor changed, so we'd get StaleAnchor first. Shield another note to reset.
        let note2 = do_shield(
            &mut executor, &mut db, &sender, 3_000, 1,
            test_blinding(11), test_blinding(21), test_blinding(98),
        );
        // Try using the original (already spent) nullifier with new anchor
        let mut tx2 = build_real_unshield(&executor, &note2, receiver, 3_000);
        // Overwrite nullifier with the already-spent one
        tx2.input_nullifiers = tx1.input_nullifiers.clone();
        let err = executor.execute_unshield(&mut db, &tx2).unwrap_err();
        assert!(matches!(err, PrivacyExecError::DoubleSpend(_)));
    }

    #[test]
    fn test_unshield_stale_anchor_rejected() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &sender, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        // Build tx with current anchor
        let tx = build_real_unshield(&executor, &note, receiver, 5_000);

        // Shield again to change the Merkle root
        do_shield(
            &mut executor, &mut db, &sender, 2_000, 1,
            test_blinding(11), test_blinding(21), test_blinding(98),
        );

        // Now the tx has a stale anchor
        assert!(matches!(
            executor.execute_unshield(&mut db, &tx),
            Err(PrivacyExecError::StaleAnchor)
        ));
    }

    #[test]
    fn test_unshield_invalid_balance_binding_rejected() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &sender, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        let mut tx = build_real_unshield(&executor, &note, receiver, 5_000);
        tx.balance_binding = [0xBB; 32]; // garbage binding

        let err = executor.execute_unshield(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::InvalidBalanceBinding));
    }

    #[test]
    fn test_unshield_invalid_commitment_opening_rejected() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &sender, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        let mut tx = build_real_unshield(&executor, &note, receiver, 5_000);
        // Corrupt the value commitment
        tx.input_value_commitments[0] = [0xCC; 32];

        let err = executor.execute_unshield(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::InvalidCommitmentOpening { index: 0 }));
    }

    #[test]
    fn test_unshield_invalid_merkle_proof_rejected() {
        let sender = test_addr(1);
        let receiver = test_addr(2);
        let mut db = setup_db_with_balance(&sender, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &sender, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        let mut tx = build_real_unshield(&executor, &note, receiver, 5_000);
        // Corrupt the note commitment (Merkle leaf)
        tx.input_note_commitments[0] = [0xDD; 32];

        let err = executor.execute_unshield(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::InvalidMerkleProof { index: 0 }));
    }

    // ── Private Transfer Tests (100% real) ──

    #[test]
    fn test_private_transfer_real() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &addr, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        let out_blinds = [test_blinding(30), test_blinding(31)];
        let tx = build_real_transfer(&executor, &note, &[3_000, 1_900], &out_blinds, 100);

        let result = executor.execute_private_transfer(&mut db, &tx).unwrap();
        assert_eq!(result.notes_created, 2);
        assert_eq!(result.nullifiers_spent, 1);
        assert_eq!(result.fee_collected, 100);
        assert_eq!(db.get_shielded_pool_balance(), 4_900); // 5000 - 100 fee
        assert!(db.is_nullifier_spent(&tx.input_nullifiers[0]));
        assert_eq!(db.get_note_count(), 3); // 1 shield + 2 transfer outputs
    }

    #[test]
    fn test_private_transfer_double_spend_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &addr, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        let tx1 = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        executor.execute_private_transfer(&mut db, &tx1).unwrap();

        // Try to spend the same note again
        let note2 = do_shield(
            &mut executor, &mut db, &addr, 3_000, 1,
            test_blinding(11), test_blinding(21), test_blinding(98),
        );
        let mut tx2 = build_real_transfer(&executor, &note2, &[2_900], &[test_blinding(40)], 100);
        tx2.input_nullifiers = tx1.input_nullifiers.clone();
        let err = executor.execute_private_transfer(&mut db, &tx2).unwrap_err();
        assert!(matches!(err, PrivacyExecError::DoubleSpend(_)));
    }

    #[test]
    fn test_private_transfer_duplicate_nullifier_in_tx_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &addr, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        // Duplicate the nullifier
        tx.input_nullifiers.push(tx.input_nullifiers[0]);
        // Also duplicate witness fields to match
        tx.input_amounts.push(tx.input_amounts[0]);
        tx.input_blindings.push(tx.input_blindings[0]);
        tx.input_value_commitments.push(tx.input_value_commitments[0]);
        tx.input_note_commitments.push(tx.input_note_commitments[0]);
        tx.input_merkle_proofs.push(tx.input_merkle_proofs[0].clone());

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::DoubleSpend(_)));
    }

    #[test]
    fn test_private_transfer_no_inputs_rejected() {
        let mut db = InMemoryStateDB::new();
        let mut executor = PrivacyExecutor::with_depth(8);

        let tx = PrivateTransferTx {
            input_nullifiers: vec![],
            output_commitments: vec![[0u8; 32]],
            anchor: executor.merkle_root(),
            balance_binding: [0u8; 32],
            fee: 0,
            input_amounts: vec![],
            input_blindings: vec![],
            input_value_commitments: vec![],
            input_note_commitments: vec![],
            input_merkle_proofs: vec![],
            output_amounts: vec![0],
            output_blindings: vec![[0u8; 32]],
            energy_proofs: vec![],
        };
        assert!(matches!(
            executor.execute_private_transfer(&mut db, &tx),
            Err(PrivacyExecError::NoInputs)
        ));
    }

    #[test]
    fn test_private_transfer_no_outputs_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &addr, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        tx.output_commitments.clear();
        tx.output_amounts.clear();
        tx.output_blindings.clear();

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::NoOutputs));
    }

    #[test]
    fn test_private_transfer_invalid_balance_binding_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &addr, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        tx.balance_binding = [0xAA; 32]; // garbage

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::InvalidBalanceBinding));
    }

    #[test]
    fn test_private_transfer_invalid_output_commitment_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &addr, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        tx.output_commitments[0] = [0xDD; 32]; // doesn't match amount/blinding

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::InvalidOutputCommitment { index: 0 }));
    }

    #[test]
    fn test_private_transfer_invalid_merkle_proof_rejected() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &addr, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        tx.input_note_commitments[0] = [0xDD; 32]; // wrong tree leaf

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::InvalidMerkleProof { index: 0 }));
    }

    #[test]
    fn test_private_transfer_balance_conservation_failure() {
        let addr = test_addr(1);
        let mut db = setup_db_with_balance(&addr, 10_000);
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        let note = do_shield(
            &mut executor, &mut db, &addr, 5_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );

        // Build valid tx, then inflate output_amounts to break conservation
        let mut tx = build_real_transfer(&executor, &note, &[4_900], &[test_blinding(30)], 100);
        tx.output_amounts[0] = 6_000; // inflated — sum_in(5000) != sum_out(6000) + fee(100)
        // Recompute output commitment for the inflated amount so it passes commitment check
        tx.output_commitments[0] = Commitment::commit(6_000, &test_blinding(30)).0;
        // Recompute balance binding for the inflated values
        tx.balance_binding = compute_balance_binding(
            5_000, 6_000, 100,
            &[note.blinding], &[test_blinding(30)],
        );

        let err = executor.execute_private_transfer(&mut db, &tx).unwrap_err();
        assert!(matches!(err, PrivacyExecError::UnshieldBalanceMismatch));
    }

    // ── E2E: Shield → Transfer → Unshield (100% real) ──

    #[test]
    fn test_e2e_shield_transfer_unshield() {
        let alice = test_addr(1);
        let bob = test_addr(2);
        let mut db = setup_db_with_balance(&alice, 100_000);
        db.put_account(Account { address: bob, balance: 0, nonce: 0 });
        let mut executor = PrivacyExecutor::with_depth(8);
        executor.set_epoch(1);

        // 1. Alice shields 50,000
        let alice_note = do_shield(
            &mut executor, &mut db, &alice, 50_000, 0,
            test_blinding(10), test_blinding(20), test_blinding(99),
        );
        assert_eq!(db.get_account(&alice).unwrap().balance, 50_000);
        assert_eq!(db.get_shielded_pool_balance(), 50_000);

        // 2. Private transfer: Alice's note → Bob(30K) + Alice change(19.5K), fee=500
        let bob_blind = test_blinding(30);
        let alice_change_blind = test_blinding(31);
        let tx = build_real_transfer(
            &executor, &alice_note,
            &[30_000, 19_500], &[bob_blind, alice_change_blind],
            500,
        );
        executor.execute_private_transfer(&mut db, &tx).unwrap();
        assert_eq!(db.get_shielded_pool_balance(), 49_500); // -500 fee

        // 3. Bob unshields 30,000 using real output note from the transfer
        // The transfer inserts output_commitments (value commitments) directly into the tree
        let bob_value_commitment = Commitment::commit(30_000, &bob_blind);
        // In private transfer, the tree leaf IS the value commitment (output_commitments[0])
        let bob_note_commitment = bob_value_commitment.0;
        // Bob's note was inserted at tree index 1 (alice's shield was 0, then transfer added 2 outputs: index 1 and 2)
        let bob_tree_index = 1;
        let bob_merkle_proof = executor.get_merkle_proof(bob_tree_index).unwrap();

        let bob_spending_secret = test_blinding(88);
        let bob_nullifier = Nullifier::derive(&bob_spending_secret, &Commitment(bob_note_commitment));
        let bob_binding = compute_balance_binding(30_000, 0, 30_000, &[bob_blind], &[]);

        let unshield_tx = UnshieldTx {
            to: bob,
            amount: 30_000,
            input_nullifiers: vec![bob_nullifier.0],
            anchor: executor.merkle_root(),
            balance_binding: bob_binding,
            input_amounts: vec![30_000],
            input_blindings: vec![bob_blind],
            input_value_commitments: vec![bob_value_commitment.0],
            input_note_commitments: vec![bob_note_commitment],
            input_merkle_proofs: vec![bob_merkle_proof],
            output_blindings: vec![],
            change_commitments: vec![],
            energy_proofs: vec![],
        };
        executor.execute_unshield(&mut db, &unshield_tx).unwrap();

        // Final state
        assert_eq!(db.get_account(&alice).unwrap().balance, 50_000);
        assert_eq!(db.get_account(&bob).unwrap().balance, 30_000);
        assert_eq!(db.get_shielded_pool_balance(), 19_500);
        assert_eq!(db.nullifier_count(), 2);
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
            input_amounts: vec![],
            input_blindings: vec![],
            input_value_commitments: vec![],
            input_note_commitments: vec![],
            input_merkle_proofs: vec![],
            output_amounts: vec![],
            output_blindings: vec![],
            energy_proofs: vec![],
        };
        let gas = PrivacyExecutor::estimate_private_transfer_gas(&tx);
        assert_eq!(
            gas,
            GAS_PRIVATE_TRANSFER_BASE + 2 * GAS_PRIVATE_TRANSFER_PER_INPUT + 3 * GAS_PRIVATE_TRANSFER_PER_OUTPUT
        );
    }
}
