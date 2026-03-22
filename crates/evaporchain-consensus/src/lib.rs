pub mod mempool;

use evaporchain_crypto::hash::blake3_hash;
use evaporchain_execution::{BlockExecutionResult, ExecutionEngine, SimpleExecutor};
use evaporchain_state::db::StateDB;
use evaporchain_types::{Block, Epoch, Transaction};
use mempool::Mempool;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tracing::info;

/// Errors that can occur during consensus.
#[derive(Debug, Error)]
pub enum ConsensusError {
    #[error("block proposal failed: {0}")]
    ProposalFailed(String),
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
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
    executor: SimpleExecutor,
    pub mempool: Mempool,
}

impl MockConsensus {
    /// Create a new consensus engine.
    ///
    /// `grace_period` is forwarded to the evaporation engine inside the executor.
    pub fn new(grace_period: u64) -> Self {
        Self {
            block_number: 0,
            epoch: 0,
            parent_hash: [0u8; 32],
            executor: SimpleExecutor::new(grace_period),
            mempool: Mempool::new(),
        }
    }

    /// Current epoch.
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Current block number.
    pub fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Produce the next block: drain mempool, execute, advance state.
    pub fn produce_block(
        &mut self,
        db: &mut dyn StateDB,
    ) -> Result<BlockProductionResult, ConsensusError> {
        self.epoch += 1;
        self.block_number += 1;

        let txs: Vec<Transaction> = self.mempool.drain();

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut block = Block {
            number: self.block_number,
            epoch: self.epoch,
            parent_hash: self.parent_hash,
            state_root: [0u8; 32],
            transactions: txs,
            timestamp,
        };

        let execution = self
            .executor
            .execute_block(db, &block)
            .map_err(|e| ConsensusError::ExecutionFailed(e.to_string()))?;

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
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn test_produce_empty_block() {
        let mut db = InMemoryStateDB::new();
        let mut consensus = MockConsensus::new(5);

        let result = consensus.produce_block(&mut db).unwrap();
        assert_eq!(result.block.number, 1);
        assert_eq!(result.block.epoch, 1);
        assert_eq!(result.execution.txs_executed, 0);
    }

    #[test]
    fn test_epoch_advances() {
        let mut db = InMemoryStateDB::new();
        let mut consensus = MockConsensus::new(5);

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
        let mut consensus = MockConsensus::new(5);

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
            balance: 1000,
            nonce: 0,
        });

        let mut consensus = MockConsensus::new(5);
        consensus
            .mempool
            .submit(Transaction::Transfer(TransferTx {
                from: addr(1),
                to: addr(2),
                amount: 100,
                nonce: 0,
                signature: None,
                public_key: None,
            }));

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
        });

        let mut consensus = MockConsensus::new(2); // 2-epoch grace

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
        let mut consensus = MockConsensus::new(5);

        consensus
            .mempool
            .submit(Transaction::CreateObject(CreateObjectTx {
                creator: addr(1),
                object_id: obj_id(42),
                energy: 5000,
                half_life: 100,
                data: vec![1, 2, 3],
                signature: None,
                public_key: None,
            }));

        let result = consensus.produce_block(&mut db).unwrap();
        assert_eq!(result.execution.txs_executed, 1);
        assert_eq!(db.object_count(), 1);

        let obj = db.get_object(&obj_id(42)).unwrap();
        assert_eq!(obj.energy, 5000);
        assert_eq!(obj.created_at, 1); // epoch 1
    }
}
