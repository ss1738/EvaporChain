use evaporchain_state::db::StateDB;
use evaporchain_types::Block;
use thiserror::Error;

/// Errors that can occur during transaction execution.
#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("execution failed: {0}")]
    Failed(String),
    #[error("invalid transaction: {0}")]
    InvalidTransaction(String),
}

/// Trait for block/transaction execution engines.
pub trait ExecutionEngine: Send + Sync {
    /// Execute all transactions in a block, returning the new state root.
    fn execute_block(
        &self,
        db: &mut dyn StateDB,
        block: &Block,
    ) -> Result<[u8; 32], ExecutionError>;
}

/// Simple executor for development.
pub struct SimpleExecutor;

impl ExecutionEngine for SimpleExecutor {
    fn execute_block(
        &self,
        _db: &mut dyn StateDB,
        _block: &Block,
    ) -> Result<[u8; 32], ExecutionError> {
        todo!("Block execution not yet implemented")
    }
}
