pub mod anchor;
pub mod bridge;
pub mod da_attestation;
pub mod encrypted_mempool;
pub mod finality;
pub mod light_client;
pub mod mempool;
pub mod state_sync;
pub mod persistence;
pub mod tendermint;
pub mod validator_set;
#[cfg(test)]
mod audit_tests;

use encrypted_mempool::EncryptedMempool;
use evaporchain_crypto::hash::blake3_hash;
use evaporchain_da::{BlockDA2D, NamespacedBlob};
use evaporchain_execution::{fees::PidFeeController, BlockExecutionResult, ExecutionEngine};
use evaporchain_execution::parallel::ParallelExecutor;
use evaporchain_state::db::StateDB;
use evaporchain_types::{Block, Epoch, Transaction};
use mempool::Mempool;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tracing::{info, warn};
use validator_set::{ValidatorInfo, ValidatorSet};

/// 2D DA outputs to embed in a produced block. Returned by
/// [`compute_block_da`] and copied directly into the block fields.
struct BlockDaOutputs {
    data_root: Option<[u8; 32]>,
    da_row_roots: Vec<[u8; 32]>,
    da_col_roots: Vec<[u8; 32]>,
    blob_commitments: Vec<[u8; 32]>,
}

/// Compute the data availability commitments for a block's transactions.
///
/// Uses [`BlockDA2D`] (Celestia-style 2D extended data square) to produce a
/// `data_root`, per-row and per-column Merkle roots, and namespace blob
/// commitments in a single call. Empty blocks fall back to a sentinel
/// `data_root` so the DA attestation flow still fires every block.
///
/// On any encoding failure all DA fields are returned empty / `None` and a
/// warning is logged — block production is never aborted by a DA error.
fn compute_block_da(txs: &[Transaction]) -> BlockDaOutputs {
    if txs.is_empty() {
        return BlockDaOutputs {
            data_root: Some(blake3::hash(b"evaporchain:empty_block").into()),
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
        };
    }
    let tx_bytes = match serde_json::to_vec(txs) {
        Ok(b) => b,
        Err(e) => {
            warn!("DA: tx serialization failed: {e} — block produced without data_root");
            return BlockDaOutputs {
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
            };
        }
    };
    let blobs: Vec<NamespacedBlob> = txs
        .iter()
        .map(|tx| {
            let (ns_id, data) = match tx {
                Transaction::Blob(blob_tx) => (blob_tx.namespace_id, blob_tx.data.clone()),
                _ => (0u64, serde_json::to_vec(tx).unwrap_or_default()),
            };
            let mut namespace = [0u8; 8];
            namespace.copy_from_slice(&ns_id.to_be_bytes());
            NamespacedBlob { namespace, data }
        })
        .collect();
    let da2d = BlockDA2D::new();
    match da2d.encode_block_with_blobs(&tx_bytes, &blobs) {
        Ok(package) => BlockDaOutputs {
            data_root: Some(package.header.data_root),
            da_row_roots: package.header.row_roots,
            da_col_roots: package.header.col_roots,
            blob_commitments: package.header.blob_commitments,
        },
        Err(e) => {
            warn!("DA: BlockDA2D encoding failed: {e} — block produced without data_root");
            BlockDaOutputs {
                data_root: None,
                da_row_roots: vec![],
                da_col_roots: vec![],
                blob_commitments: vec![],
            }
        }
    }
}

/// Errors that can occur during consensus.
#[derive(Debug, Error)]
pub enum ConsensusError {
    #[error("block proposal failed: {0}")]
    ProposalFailed(String),
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
    #[error("not leader: validator {validator_id} is not the leader for epoch {epoch} (leader is {leader_id})")]
    NotLeader {
        validator_id: u64,
        epoch: u64,
        leader_id: u64,
    },
    #[error("invalid producer: block produced by validator {producer_id}, expected {expected_id}")]
    InvalidProducer {
        producer_id: u64,
        expected_id: u64,
    },
}

/// Result of producing one block.
pub struct BlockProductionResult {
    pub block: Block,
    pub execution: BlockExecutionResult,
}

/// Mock single-node consensus that produces blocks on a timer.
///
/// Each call to `produce_block` drains the mempool, builds a block,
/// executes it through the execution engine, and advances the epoch.
pub struct MockConsensus {
    block_number: u64,
    epoch: Epoch,
    parent_hash: [u8; 32],
    pub executor: ParallelExecutor,
    pub mempool: Mempool,
    /// MEV-protected encrypted mempool (enabled with --mev-protection).
    pub encrypted_mempool: Option<EncryptedMempool>,
}

impl MockConsensus {
    /// Create a new consensus engine.
    ///
    /// `grace_period` is forwarded to the evaporation engine inside the executor.
    pub fn new(grace_period: u64) -> Self {
        Self::new_with_gas_limit(grace_period, 500_000)
    }

    /// Create with a custom block gas limit (for high-throughput mode).
    pub fn new_with_gas_limit(grace_period: u64, block_gas_limit: u64) -> Self {
        Self {
            block_number: 0,
            epoch: 0,
            parent_hash: [0u8; 32],
            executor: ParallelExecutor::new_production(
                grace_period,
                PidFeeController::testnet_config(),
                block_gas_limit,
            ),
            mempool: Mempool::new(),
            encrypted_mempool: None,
        }
    }

    /// Create a new consensus engine with MEV protection enabled.
    pub fn new_with_mev_protection(grace_period: u64, reveal_delay: u64) -> Self {
        Self {
            block_number: 0,
            epoch: 0,
            parent_hash: [0u8; 32],
            executor: ParallelExecutor::new_with_sig_verification(grace_period),
            mempool: Mempool::new(),
            encrypted_mempool: Some(EncryptedMempool::new(reveal_delay)),
        }
    }

    /// Create a test-friendly consensus engine with a small privacy tree.
    #[cfg(test)]
    pub fn new_for_test(grace_period: u64) -> Self {
        Self {
            block_number: 0,
            epoch: 0,
            parent_hash: [0u8; 32],
            executor: ParallelExecutor::new_with_sig_verification_for_test(grace_period),
            mempool: Mempool::new(),
            encrypted_mempool: None,
        }
    }

    /// Whether MEV protection is enabled.
    pub fn mev_protection_enabled(&self) -> bool {
        self.encrypted_mempool.is_some()
    }

    /// Current epoch.
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Current block number.
    pub fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Restore consensus state from persistent storage (used on restart).
    pub fn restore_state(&mut self, block_number: u64, epoch: Epoch, parent_hash: [u8; 32]) {
        self.block_number = block_number;
        self.epoch = epoch;
        self.parent_hash = parent_hash;
    }

    fn validate_block_header(&self, block: &Block) -> Result<(), ConsensusError> {
        if block.number == 0 {
            return Err(ConsensusError::ProposalFailed("block number cannot be 0".into()));
        }
        if block.number <= self.block_number {
            return Err(ConsensusError::ProposalFailed(
                format!("block height {} not greater than local {}", block.number, self.block_number),
            ));
        }
        if block.epoch < self.epoch {
            return Err(ConsensusError::ProposalFailed(
                format!("block epoch {} less than local {}", block.epoch, self.epoch),
            ));
        }
        if block.parent_hash != self.parent_hash && self.block_number > 0 {
            return Err(ConsensusError::ProposalFailed(
                "parent hash mismatch".into(),
            ));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if block.timestamp > now + 30 {
            return Err(ConsensusError::ProposalFailed(
                "block timestamp too far in the future".into(),
            ));
        }
        Ok(())
    }

    /// Apply a block received from a peer: re-execute transactions and
    /// advance local state to match.  Returns the execution result so the
    /// caller can verify the state root matches.
    pub fn apply_block(
        &mut self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockProductionResult, ConsensusError> {
        self.validate_block_header(block)?;

        let execution = self
            .executor
            .execute_block(db, block)
            .map_err(|e: evaporchain_execution::ExecutionError| ConsensusError::ExecutionFailed(e.to_string()))?;

        // Advance local tracking to stay in sync
        self.block_number = block.number;
        self.epoch = block.epoch;
        self.mempool.set_epoch(block.epoch);

        // Derive parent hash the same way produce_block does
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(&block.number.to_le_bytes());
        hash_input.extend_from_slice(&block.epoch.to_le_bytes());
        hash_input.extend_from_slice(&execution.state_root);
        hash_input.extend_from_slice(&block.parent_hash);
        self.parent_hash = blake3_hash(&hash_input);

        info!(
            block = block.number,
            epoch = block.epoch,
            txs = block.transactions.len(),
            state_root = hex::encode(execution.state_root),
            "Block applied from peer"
        );

        Ok(BlockProductionResult {
            block: block.clone(),
            execution,
        })
    }

    /// Produce the next block: drain mempool, execute, advance state.
    pub fn produce_block(
        &mut self,
        db: &mut dyn StateDB,
    ) -> Result<BlockProductionResult, ConsensusError> {
        self.epoch += 1;
        self.block_number += 1;
        self.mempool.set_epoch(self.epoch);

        let txs: Vec<Transaction> = self.mempool.drain();

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let da = compute_block_da(&txs);
        let mut block = Block {
            number: self.block_number,
            epoch: self.epoch,
            parent_hash: self.parent_hash,
            state_root: [0u8; 32],
            transactions: txs,
            timestamp,
            chain_id: String::new(),
            producer_id: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: da.data_root,
            da_row_roots: da.da_row_roots,
            da_col_roots: da.da_col_roots,
            blob_commitments: da.blob_commitments,
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
        };

        let execution = self
            .executor
            .execute_block(db, &block)
            .map_err(|e: evaporchain_execution::ExecutionError| ConsensusError::ExecutionFailed(e.to_string()))?;

        block.state_root = execution.state_root;

        // Derive parent hash for next block from this block's content.
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(&block.number.to_le_bytes());
        hash_input.extend_from_slice(&block.epoch.to_le_bytes());
        hash_input.extend_from_slice(&block.state_root);
        hash_input.extend_from_slice(&block.parent_hash);
        self.parent_hash = blake3_hash(&hash_input);

        info!(
            block = block.number,
            epoch = block.epoch,
            txs = block.transactions.len(),
            state_root = hex::encode(block.state_root),
            "Block produced"
        );

        Ok(BlockProductionResult { block, execution })
    }

    /// Produce a block with MEV-protected reveals.
    ///
    /// Processes encrypted tx reveals for the current epoch, combines with
    /// plaintext mempool transactions, and produces a block.
    /// `nonces` maps commitment → nonce for encrypted txs being revealed.
    pub fn produce_block_with_reveals(
        &mut self,
        db: &mut dyn StateDB,
        nonces: &[([u8; 32], [u8; 32])],
    ) -> Result<BlockProductionResult, ConsensusError> {
        self.epoch += 1;
        self.block_number += 1;
        self.mempool.set_epoch(self.epoch);

        // Drain plaintext mempool
        let mut txs: Vec<Transaction> = self.mempool.drain();

        // Process encrypted reveals
        if let Some(ref mut enc_pool) = self.encrypted_mempool {
            let revealed = enc_pool.process_reveals(self.epoch, nonces);
            txs.extend(revealed);
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let da = compute_block_da(&txs);
        let mut block = Block {
            number: self.block_number,
            epoch: self.epoch,
            parent_hash: self.parent_hash,
            state_root: [0u8; 32],
            transactions: txs,
            timestamp,
            chain_id: String::new(),
            producer_id: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: da.data_root,
            da_row_roots: da.da_row_roots,
            da_col_roots: da.da_col_roots,
            blob_commitments: da.blob_commitments,
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
        };

        let execution = self
            .executor
            .execute_block(db, &block)
            .map_err(|e: evaporchain_execution::ExecutionError| ConsensusError::ExecutionFailed(e.to_string()))?;

        block.state_root = execution.state_root;

        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(&block.number.to_le_bytes());
        hash_input.extend_from_slice(&block.epoch.to_le_bytes());
        hash_input.extend_from_slice(&block.state_root);
        hash_input.extend_from_slice(&block.parent_hash);
        self.parent_hash = blake3_hash(&hash_input);

        info!(
            block = block.number,
            epoch = block.epoch,
            txs = block.transactions.len(),
            state_root = hex::encode(block.state_root),
            "Block produced (MEV-protected)"
        );

        Ok(BlockProductionResult { block, execution })
    }
}

// ─────────────────── RotatingConsensus ──────────────────────────────────────

/// Multi-producer consensus with energy-weighted leader rotation.
///
/// Wraps a `ValidatorSet` and the existing block production logic from
/// `MockConsensus`.  Each epoch, exactly one validator is the legitimate
/// leader.  Other validators verify that received blocks were produced by
/// the correct leader.
pub struct RotatingConsensus {
    /// This node's validator id.
    my_id: u64,
    block_number: u64,
    epoch: Epoch,
    parent_hash: [u8; 32],
    executor: ParallelExecutor,
    pub mempool: Mempool,
    pub encrypted_mempool: Option<EncryptedMempool>,
    pub validator_set: ValidatorSet,
}

impl RotatingConsensus {
    /// Create a new rotating consensus engine.
    pub fn new(my_id: u64, grace_period: u64, validator_set: ValidatorSet) -> Self {
        let block_gas_limit = 500_000;
        Self {
            my_id,
            block_number: 0,
            epoch: 0,
            parent_hash: [0u8; 32],
            executor: ParallelExecutor::new_production(
                grace_period,
                PidFeeController::testnet_config(),
                block_gas_limit,
            ),
            mempool: Mempool::new(),
            encrypted_mempool: None,
            validator_set,
        }
    }

    /// Create a test-friendly engine with a small privacy tree.
    #[cfg(test)]
    pub fn new_for_test(my_id: u64, grace_period: u64, validator_set: ValidatorSet) -> Self {
        Self {
            my_id,
            block_number: 0,
            epoch: 0,
            parent_hash: [0u8; 32],
            executor: ParallelExecutor::new_for_test(grace_period),
            mempool: Mempool::new(),
            encrypted_mempool: None,
            validator_set,
        }
    }

    /// Create with MEV protection enabled.
    pub fn new_with_mev_protection(
        my_id: u64,
        grace_period: u64,
        reveal_delay: u64,
        validator_set: ValidatorSet,
    ) -> Self {
        Self {
            my_id,
            block_number: 0,
            epoch: 0,
            parent_hash: [0u8; 32],
            executor: ParallelExecutor::new_with_sig_verification(grace_period),
            mempool: Mempool::new(),
            encrypted_mempool: Some(EncryptedMempool::new(reveal_delay)),
            validator_set,
        }
    }

    /// This node's validator id.
    pub fn my_id(&self) -> u64 {
        self.my_id
    }

    /// Current epoch.
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Current block number.
    pub fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Check if validator `id` is the leader for the given epoch.
    pub fn is_my_turn(&self, my_id: u64, epoch: u64) -> bool {
        self.validator_set.is_leader(my_id, epoch)
    }

    /// Produce a block if this node is the leader for the next epoch.
    ///
    /// Returns `None` if this node is not the leader.
    /// Returns `Err` if block production fails.
    pub fn produce_block_if_leader(
        &mut self,
        db: &mut dyn StateDB,
    ) -> Result<Option<BlockProductionResult>, ConsensusError> {
        let next_epoch = self.epoch + 1;

        if !self.is_my_turn(self.my_id, next_epoch) {
            return Ok(None);
        }

        self.epoch = next_epoch;
        self.block_number += 1;
        self.mempool.set_epoch(self.epoch);

        let txs: Vec<Transaction> = self.mempool.drain();

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let da = compute_block_da(&txs);
        let mut block = Block {
            number: self.block_number,
            epoch: self.epoch,
            parent_hash: self.parent_hash,
            state_root: [0u8; 32],
            transactions: txs,
            timestamp,
            chain_id: String::new(),
            producer_id: Some(self.my_id),
            vrf_output: None,
            vrf_proof: None,
            data_root: da.data_root,
            da_row_roots: da.da_row_roots,
            da_col_roots: da.da_col_roots,
            blob_commitments: da.blob_commitments,
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
        };

        let execution = self
            .executor
            .execute_block(db, &block)
            .map_err(|e: evaporchain_execution::ExecutionError| ConsensusError::ExecutionFailed(e.to_string()))?;

        block.state_root = execution.state_root;

        // Update validator health based on evaporations processed
        self.validator_set
            .update_health_score(self.my_id, execution.objects_evaporated);

        // Decay all health scores each epoch
        self.validator_set.decay_health_scores();

        // Derive parent hash for next block
        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(&block.number.to_le_bytes());
        hash_input.extend_from_slice(&block.epoch.to_le_bytes());
        hash_input.extend_from_slice(&block.state_root);
        hash_input.extend_from_slice(&block.parent_hash);
        self.parent_hash = blake3_hash(&hash_input);

        info!(
            block = block.number,
            epoch = block.epoch,
            producer = self.my_id,
            txs = block.transactions.len(),
            state_root = hex::encode(block.state_root),
            "Block produced (rotating consensus)"
        );

        Ok(Some(BlockProductionResult { block, execution }))
    }

    /// Validate that a received block was produced by the legitimate leader.
    pub fn validate_received_block(&self, block: &Block) -> Result<(), ConsensusError> {
        // Structural header validation
        if block.number == 0 {
            return Err(ConsensusError::ProposalFailed("block number cannot be 0".into()));
        }
        if block.number <= self.block_number {
            return Err(ConsensusError::ProposalFailed(
                format!("block height {} not greater than local {}", block.number, self.block_number),
            ));
        }
        if block.epoch < self.epoch {
            return Err(ConsensusError::ProposalFailed(
                format!("block epoch {} less than local {}", block.epoch, self.epoch),
            ));
        }
        if block.parent_hash != self.parent_hash && self.block_number > 0 {
            return Err(ConsensusError::ProposalFailed("parent hash mismatch".into()));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if block.timestamp > now + 30 {
            return Err(ConsensusError::ProposalFailed(
                "block timestamp too far in the future".into(),
            ));
        }

        // Producer legitimacy
        let producer_id = block.producer_id.ok_or(ConsensusError::ProposalFailed(
            "block missing producer_id".to_string(),
        ))?;

        let expected_leader = self
            .validator_set
            .leader_for_epoch(block.epoch)
            .ok_or(ConsensusError::ProposalFailed(
                "no validators in set".to_string(),
            ))?;

        if producer_id != expected_leader.id {
            return Err(ConsensusError::InvalidProducer {
                producer_id,
                expected_id: expected_leader.id,
            });
        }

        Ok(())
    }

    /// Apply a validated block from a peer: re-execute and advance local state.
    pub fn apply_block(
        &mut self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<BlockProductionResult, ConsensusError> {
        // Validate producer legitimacy first
        self.validate_received_block(block)?;

        let execution = self
            .executor
            .execute_block(db, block)
            .map_err(|e: evaporchain_execution::ExecutionError| ConsensusError::ExecutionFailed(e.to_string()))?;

        // Update health score for the block producer
        if let Some(producer_id) = block.producer_id {
            self.validator_set
                .update_health_score(producer_id, execution.objects_evaporated);
        }

        // Decay all health scores
        self.validator_set.decay_health_scores();

        // Advance local tracking
        self.block_number = block.number;
        self.epoch = block.epoch;
        self.mempool.set_epoch(block.epoch);

        let mut hash_input = Vec::new();
        hash_input.extend_from_slice(&block.number.to_le_bytes());
        hash_input.extend_from_slice(&block.epoch.to_le_bytes());
        hash_input.extend_from_slice(&execution.state_root);
        hash_input.extend_from_slice(&block.parent_hash);
        self.parent_hash = blake3_hash(&hash_input);

        info!(
            block = block.number,
            epoch = block.epoch,
            producer = ?block.producer_id,
            txs = block.transactions.len(),
            state_root = hex::encode(execution.state_root),
            "Block applied from peer (rotating consensus)"
        );

        Ok(BlockProductionResult {
            block: block.clone(),
            execution,
        })
    }

    /// Get the leader validator for a given epoch.
    pub fn leader_for_epoch(&self, epoch: u64) -> Option<&ValidatorInfo> {
        self.validator_set.leader_for_epoch(epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_crypto::signatures::{MlDsaKeypair, Signer};
    use evaporchain_state::InMemoryStateDB;
    use evaporchain_types::{Account, CreateObjectTx, ObjectState, StateObject, TransferTx};

    fn addr(b: u8) -> [u8; 32] {
        let mut a = [0u8; 32];
        a[0] = b;
        a
    }

    fn obj_id(b: u8) -> [u8; 32] {
        let mut id = [0u8; 32];
        id[0] = b;
        id
    }

    /// Sign a transaction with the given keypair (empty chain_id for tests).
    fn sign_tx(tx: &mut Transaction, kp: &MlDsaKeypair) {
        let msg = tx.signing_message("");
        let sig = kp.sign(&msg);
        let pk = kp.public_key_bytes();
        match tx {
            Transaction::Transfer(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::CreateObject(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::Refresh(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::DeployContract(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::CallContract(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::DeployScript(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::CallScript(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::ValidatorStake(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::ValidatorExit(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::ValidatorClaimStake(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::Shield(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::Unshield(_) | Transaction::PrivateTransfer(_) => {}
            Transaction::Deferred(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::Blob(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::Governance(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::MultiSig(_) => {}
            Transaction::UserOp(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::UpgradeContract(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::Delegate(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::Undelegate(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::RotateValidatorKey(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
            Transaction::ClaimDelegation(ref mut inner) => {
                inner.signature = Some(sig);
                inner.public_key = Some(pk);
            }
        }
    }

    #[test]
    fn test_produce_empty_block() {
        let mut db = InMemoryStateDB::new();
        let mut consensus = MockConsensus::new_for_test(5);

        let result = consensus.produce_block(&mut db).unwrap();
        assert_eq!(result.block.number, 1);
        assert_eq!(result.block.epoch, 1);
        assert_eq!(result.execution.txs_executed, 0);
    }

    #[test]
    fn test_epoch_advances() {
        let mut db = InMemoryStateDB::new();
        let mut consensus = MockConsensus::new_for_test(5);

        consensus.produce_block(&mut db).unwrap();
        consensus.produce_block(&mut db).unwrap();
        let r = consensus.produce_block(&mut db).unwrap();

        assert_eq!(r.block.number, 3);
        assert_eq!(r.block.epoch, 3);
        assert_eq!(consensus.epoch(), 3);
    }

    #[test]
    fn test_parent_hash_chains() {
        let mut db = InMemoryStateDB::new();
        let mut consensus = MockConsensus::new_for_test(5);

        let r1 = consensus.produce_block(&mut db).unwrap();
        let r2 = consensus.produce_block(&mut db).unwrap();

        assert_eq!(r1.block.parent_hash, [0u8; 32]); // genesis parent
        assert_ne!(r2.block.parent_hash, [0u8; 32]); // chains to block 1
    }

    #[test]
    fn test_mempool_drains_into_block() {
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });

        let kp = MlDsaKeypair::generate();
        let mut consensus = MockConsensus::new_for_test(5);
        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 100,
            nonce: 0,
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx, &kp);
        consensus.mempool.submit(tx);

        let result = consensus.produce_block(&mut db).unwrap();
        assert_eq!(result.execution.txs_executed, 1);
        assert_eq!(result.block.transactions.len(), 1);

        // Mempool should be empty now
        let result2 = consensus.produce_block(&mut db).unwrap();
        assert_eq!(result2.block.transactions.len(), 0);
    }

    #[test]
    fn test_evaporation_through_consensus() {
        let mut db = InMemoryStateDB::new();
        // Object with energy 2, half_life 1 → energy_at(2) = 2>>2 = 0
        db.put_object(StateObject {
            id: obj_id(1),
            owner: addr(1),
            energy: 2,
            half_life: 1,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![0xAB],
            decay_curve: None,
        });

        let mut consensus = MockConsensus::new_for_test(2); // 2-epoch grace

        // Epoch 1: energy = 2>>1 = 1, still alive
        let r1 = consensus.produce_block(&mut db).unwrap();
        assert_eq!(r1.execution.objects_entered_grace, 0);

        // Epoch 2: energy = 2>>2 = 0 → enters grace
        let r2 = consensus.produce_block(&mut db).unwrap();
        assert_eq!(r2.execution.objects_entered_grace, 1);

        // Epoch 3: still in grace (grace_epoch=2, need epoch >= 2+2=4)
        let r3 = consensus.produce_block(&mut db).unwrap();
        assert_eq!(r3.execution.objects_evaporated, 0);

        // Epoch 4: grace expired → evaporated
        let r4 = consensus.produce_block(&mut db).unwrap();
        assert_eq!(r4.execution.objects_evaporated, 1);
        assert_eq!(db.object_count(), 0);
        assert_eq!(db.ghost_count(), 1);
    }

    #[test]
    fn test_create_object_via_consensus() {
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });
        let kp = MlDsaKeypair::generate();
        let mut consensus = MockConsensus::new_for_test(5);

        let mut tx = Transaction::CreateObject(CreateObjectTx {
            creator: addr(1),
            object_id: obj_id(42),
            energy: 5000,
            half_life: 100,
            data: vec![1, 2, 3],
            decay_curve: None,
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx, &kp);
        consensus.mempool.submit(tx);

        let result = consensus.produce_block(&mut db).unwrap();
        assert_eq!(result.execution.txs_executed, 1);
        assert_eq!(db.object_count(), 1);

        let obj = db.get_object(&obj_id(42)).unwrap();
        assert_eq!(obj.energy, 5000);
        assert_eq!(obj.created_at, 1); // epoch 1
    }

    #[test]
    fn test_apply_block_from_peer() {
        // Producer creates a block
        let mut producer_db = InMemoryStateDB::new();
        producer_db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });
        let kp = MlDsaKeypair::generate();
        let mut producer = MockConsensus::new_for_test(5);
        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 500,
            nonce: 0,
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx, &kp);
        producer.mempool.submit(tx);
        let produced = producer.produce_block(&mut producer_db).unwrap();

        // Follower applies the same block
        let mut follower_db = InMemoryStateDB::new();
        follower_db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });
        let mut follower = MockConsensus::new_for_test(5);
        let applied = follower.apply_block(&mut follower_db, &produced.block).unwrap();

        // State roots must match
        assert_eq!(applied.execution.state_root, produced.execution.state_root);
        assert_eq!(applied.execution.txs_executed, 1);
        assert_eq!(follower.epoch(), 1);
        assert_eq!(follower.block_number(), 1);

        // Balances must match
        assert_eq!(
            follower_db.get_account(&addr(1)).unwrap().balance,
            producer_db.get_account(&addr(1)).unwrap().balance,
        );
        assert_eq!(
            follower_db.get_account(&addr(2)).unwrap().balance,
            producer_db.get_account(&addr(2)).unwrap().balance,
        );
    }

    #[test]
    fn test_apply_multiple_blocks_state_convergence() {
        // Producer creates 3 blocks with transactions
        let mut producer_db = InMemoryStateDB::new();
        producer_db.put_account(Account {
            address: addr(1),
            balance: 100_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });
        producer_db.put_object(StateObject {
            id: obj_id(1),
            owner: addr(1),
            energy: 100,
            half_life: 5,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![0xAA],
            decay_curve: None,
        });

        let kp1 = MlDsaKeypair::generate();
        let kp2 = MlDsaKeypair::generate();
        let mut producer = MockConsensus::new_for_test(3);
        let mut blocks = Vec::new();

        // Block 1: transfer
        let mut tx1 = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 1000,
            nonce: 0,
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx1, &kp1);
        producer.mempool.submit(tx1);
        blocks.push(producer.produce_block(&mut producer_db).unwrap().block);

        // Block 2: create object
        let mut tx2 = Transaction::CreateObject(CreateObjectTx {
            creator: addr(2),
            object_id: obj_id(42),
            energy: 500,
            half_life: 10,
            data: vec![0xBB],
            decay_curve: None,
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx2, &kp2);
        producer.mempool.submit(tx2);
        blocks.push(producer.produce_block(&mut producer_db).unwrap().block);

        // Block 3: empty (just evaporation tick)
        blocks.push(producer.produce_block(&mut producer_db).unwrap().block);

        // Follower replays all 3 blocks
        let mut follower_db = InMemoryStateDB::new();
        follower_db.put_account(Account {
            address: addr(1),
            balance: 100_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });
        follower_db.put_object(StateObject {
            id: obj_id(1),
            owner: addr(1),
            energy: 100,
            half_life: 5,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![0xAA],
            decay_curve: None,
        });
        let mut follower = MockConsensus::new_for_test(3);

        for block in &blocks {
            follower.apply_block(&mut follower_db, block).unwrap();
        }

        // State roots must converge
        assert_eq!(
            producer_db.compute_state_root(),
            follower_db.compute_state_root(),
        );
        assert_eq!(follower.epoch(), 3);
        assert_eq!(follower.block_number(), 3);
        assert_eq!(follower_db.object_count(), producer_db.object_count());
    }

    // ─────────────── RotatingConsensus tests ───────────────────────────────

    fn make_validator(id: u64, stake: u64) -> ValidatorInfo {
        let mut address = [0u8; 32];
        address[0] = id as u8;
        ValidatorInfo::new(id, stake, address)
    }

    fn make_rotating(my_id: u64, validator_ids: &[u64]) -> RotatingConsensus {
        let validators: Vec<_> = validator_ids
            .iter()
            .map(|&id| make_validator(id, 1000))
            .collect();
        RotatingConsensus::new_for_test(my_id, 5, ValidatorSet::with_validators(validators))
    }

    #[test]
    fn test_rotating_produce_when_leader() {
        let mut db = InMemoryStateDB::new();
        // Try many epochs — we must be leader for at least one
        let mut rc = make_rotating(1, &[1, 2, 3, 4]);
        let mut produced = false;
        for _ in 0..100 {
            if let Ok(Some(result)) = rc.produce_block_if_leader(&mut db) {
                assert_eq!(result.block.producer_id, Some(1));
                produced = true;
                break;
            } else {
                // Not our turn — advance epoch manually
                rc.epoch += 1;
            }
        }
        assert!(produced, "Validator 1 should be leader at least once in 100 epochs");
    }

    #[test]
    fn test_rotating_skip_when_not_leader() {
        let mut db = InMemoryStateDB::new();
        let mut rc = make_rotating(1, &[1, 2, 3, 4]);
        let mut skipped = false;
        for _ in 0..100 {
            let next_epoch = rc.epoch + 1;
            if !rc.is_my_turn(1, next_epoch) {
                let result = rc.produce_block_if_leader(&mut db).unwrap();
                assert!(result.is_none());
                skipped = true;
                break;
            }
            rc.epoch += 1;
        }
        assert!(skipped, "Validator 1 should NOT be leader for at least one epoch");
    }

    #[test]
    fn test_rotating_validate_legitimate_block() {
        let mut db = InMemoryStateDB::new();
        // Producer produces a block
        let mut producer = make_rotating(1, &[1, 2, 3, 4]);
        let mut block = None;
        for _ in 0..100 {
            if let Ok(Some(result)) = producer.produce_block_if_leader(&mut db) {
                block = Some(result.block);
                break;
            } else {
                producer.epoch += 1;
            }
        }
        let block = block.expect("should produce at least one block");

        // Follower validates it
        let follower = make_rotating(2, &[1, 2, 3, 4]);
        assert!(follower.validate_received_block(&block).is_ok());
    }

    #[test]
    fn test_rotating_reject_wrong_producer() {
        let rc = make_rotating(1, &[1, 2, 3, 4]);

        // Find which validator is leader for epoch 1
        let leader = rc.leader_for_epoch(1).unwrap().id;
        let wrong_id = if leader == 1 { 2 } else { 1 };

        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: Some(wrong_id),
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
        };

        let result = rc.validate_received_block(&block);
        assert!(result.is_err());
        match result.unwrap_err() {
            ConsensusError::InvalidProducer { producer_id, expected_id } => {
                assert_eq!(producer_id, wrong_id);
                assert_eq!(expected_id, leader);
            }
            e => panic!("Expected InvalidProducer, got {:?}", e),
        }
    }

    #[test]
    fn test_rotating_reject_missing_producer_id() {
        let rc = make_rotating(1, &[1, 2, 3, 4]);
        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: None,
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
        };
        assert!(rc.validate_received_block(&block).is_err());
    }

    #[test]
    fn test_rotating_apply_block_from_peer() {
        let mut producer_db = InMemoryStateDB::new();
        producer_db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });

        let mut producer = make_rotating(1, &[1, 2, 3, 4]);
        // Find an epoch where validator 1 is leader
        let mut produced_block = None;
        for _ in 0..100 {
            if let Ok(Some(result)) = producer.produce_block_if_leader(&mut producer_db) {
                produced_block = Some(result.block);
                break;
            } else {
                producer.epoch += 1;
            }
        }
        let block = produced_block.expect("should produce");

        // Follower applies
        let mut follower_db = InMemoryStateDB::new();
        follower_db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });
        let mut follower = make_rotating(2, &[1, 2, 3, 4]);
        let applied = follower.apply_block(&mut follower_db, &block).unwrap();
        assert_eq!(applied.execution.state_root, block.state_root);
        assert_eq!(follower.epoch(), block.epoch);
    }

    #[test]
    fn test_rotating_apply_rejects_invalid_producer() {
        let mut db = InMemoryStateDB::new();
        let mut follower = make_rotating(2, &[1, 2, 3, 4]);

        let leader = follower.leader_for_epoch(1).unwrap().id;
        let wrong_id = if leader == 1 { 2 } else { 1 };

        let block = Block {
            number: 1,
            epoch: 1,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
            timestamp: 0,
            chain_id: String::new(),
            producer_id: Some(wrong_id),
            vrf_output: None,
            vrf_proof: None,
            data_root: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
            blob_commitments: vec![],
            da_certificate: None,
            commit_certificate: None,
            nova_proof: None,
            anchor_hash: None,
            state_function_commitment: None,
            oracle_state_root: None,
            shard_count: None,
        };

        assert!(follower.apply_block(&mut db, &block).is_err());
    }

    #[test]
    fn test_rotating_producer_id_set_in_block() {
        let mut db = InMemoryStateDB::new();
        let mut rc = make_rotating(3, &[1, 2, 3, 4]);

        for _ in 0..100 {
            if let Ok(Some(result)) = rc.produce_block_if_leader(&mut db) {
                assert_eq!(result.block.producer_id, Some(3));
                return;
            } else {
                rc.epoch += 1;
            }
        }
        panic!("Validator 3 never got a turn");
    }

    #[test]
    fn test_rotating_health_score_updates_on_produce() {
        let mut db = InMemoryStateDB::new();
        // Add an object that will evaporate
        db.put_object(StateObject {
            id: obj_id(1),
            owner: addr(1),
            energy: 2,
            half_life: 1,
            created_at: 0,
            last_refreshed: 0,
            state: ObjectState::Active,
            grace_epoch: None,
            data: vec![0xAB],
            decay_curve: None,
        });

        let mut rc = make_rotating(1, &[1, 2]);
        let initial_health = rc.validator_set.get(1).unwrap().health_score;

        // Produce blocks until evaporation happens
        for _ in 0..20 {
            if rc.produce_block_if_leader(&mut db).is_ok() {
                // produced or skipped
            }
            if rc.validator_set.get(1).unwrap().health_score != initial_health {
                break;
            }
            rc.epoch += 1;
        }
        // Health score changed (either increased from evaporation or decayed)
        // The point is the system tracks it
        assert!(rc.validator_set.get(1).unwrap().blocks_produced > 0
            || rc.validator_set.get(1).unwrap().health_score != initial_health);
    }

    #[test]
    fn test_rotating_all_validators_produce() {
        let mut db = InMemoryStateDB::new();
        let ids = [1u64, 2, 3, 4];
        let mut produced_by = std::collections::HashSet::new();

        for &my_id in &ids {
            let mut rc = make_rotating(my_id, &ids);
            for _ in 0..200 {
                if let Ok(Some(result)) = rc.produce_block_if_leader(&mut db) {
                    produced_by.insert(result.block.producer_id.unwrap());
                } else {
                    rc.epoch += 1;
                }
            }
        }

        assert_eq!(
            produced_by.len(),
            4,
            "All 4 validators should produce at least one block: {:?}",
            produced_by
        );
    }

    #[test]
    fn test_rotating_epoch_advances_only_on_produce() {
        let mut db = InMemoryStateDB::new();
        let mut rc = make_rotating(1, &[1, 2, 3, 4]);

        // When not leader, epoch should not advance
        let epoch_before = rc.epoch();
        let result = rc.produce_block_if_leader(&mut db).unwrap();
        if result.is_none() {
            assert_eq!(rc.epoch(), epoch_before);
        }
    }

    #[test]
    fn test_rotating_is_my_turn() {
        let rc = make_rotating(1, &[1, 2, 3, 4]);
        let mut my_turns = 0;
        let mut not_my_turns = 0;
        for epoch in 1..=100 {
            if rc.is_my_turn(1, epoch) {
                my_turns += 1;
            } else {
                not_my_turns += 1;
            }
        }
        // With 4 equal-stake validators, should get roughly 25% of turns
        assert!(my_turns > 10, "Expected at least 10 turns, got {}", my_turns);
        assert!(not_my_turns > 50, "Expected mostly not-my-turns, got {}", not_my_turns);
    }

    #[test]
    fn test_rotating_with_transactions() {
        let mut db = InMemoryStateDB::new();
        db.put_account(Account {
            address: addr(1),
            balance: 1_000_000,
            nonce: 0,
        storage_deposit: 0,
        storage_bytes: 0,
        });

        let kp = MlDsaKeypair::generate();
        let mut rc = make_rotating(1, &[1, 2]);
        let mut tx = Transaction::Transfer(TransferTx {
            from: addr(1),
            to: addr(2),
            amount: 500,
            nonce: 0,
            signature: None,
            public_key: None,
        });
        sign_tx(&mut tx, &kp);
        rc.mempool.submit(tx);

        // Produce until our turn
        for _ in 0..100 {
            if let Ok(Some(result)) = rc.produce_block_if_leader(&mut db) {
                assert_eq!(result.execution.txs_executed, 1);
                assert_eq!(result.block.transactions.len(), 1);
                return;
            } else {
                rc.epoch += 1;
            }
        }
        panic!("Never got to produce");
    }
}
