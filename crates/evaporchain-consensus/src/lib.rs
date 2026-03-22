use async_trait::async_trait;
use evaporchain_types::{Block, Transaction};
use thiserror::Error;

/// Errors that can occur during consensus.
#[derive(Debug, Error)]
pub enum ConsensusError {
    #[error("block proposal failed: {0}")]
    ProposalFailed(String),
    #[error("block finalization failed: {0}")]
    FinalizationFailed(String),
}

/// Trait for consensus engine implementations.
#[async_trait]
pub trait ConsensusEngine: Send + Sync {
    /// Propose a new block containing the given transactions.
    async fn propose_block(&self, txs: Vec<Transaction>) -> Result<Block, ConsensusError>;
    /// Finalize a block (mark as canonical).
    async fn finalize_block(&self, block: &Block) -> Result<(), ConsensusError>;
}

/// Simple sequential consensus for development.
pub struct MockConsensus {
    pub current_epoch: u64,
}

impl MockConsensus {
    pub fn new() -> Self {
        Self { current_epoch: 0 }
    }
}

impl Default for MockConsensus {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConsensusEngine for MockConsensus {
    async fn propose_block(&self, txs: Vec<Transaction>) -> Result<Block, ConsensusError> {
        Ok(Block {
            number: self.current_epoch,
            epoch: self.current_epoch,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: txs,
            timestamp: 0,
        })
    }

    async fn finalize_block(&self, _block: &Block) -> Result<(), ConsensusError> {
        tracing::info!("Mock: block finalized");
        Ok(())
    }
}
