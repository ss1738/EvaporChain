use async_trait::async_trait;
use evaporchain_types::{Block, Transaction};
use thiserror::Error;

pub mod service;

pub use service::{NetworkConfig, P2pNetworkService, ShardCache, cache_da_package};

/// Errors that can occur in the network layer.
#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("broadcast failed: {0}")]
    BroadcastFailed(String),
    #[error("connection error: {0}")]
    ConnectionError(String),
}

/// Trait for P2P network services.
#[async_trait]
pub trait NetworkService: Send + Sync {
    /// Broadcast a transaction to the network.
    async fn broadcast_tx(&self, tx: &Transaction) -> Result<(), NetworkError>;
    /// Broadcast a block to the network.
    async fn broadcast_block(&self, block: &Block) -> Result<(), NetworkError>;
}

/// Mock network that logs operations.
pub struct MockNetwork;

#[async_trait]
impl NetworkService for MockNetwork {
    async fn broadcast_tx(&self, _tx: &Transaction) -> Result<(), NetworkError> {
        tracing::info!("Mock: transaction broadcast");
        Ok(())
    }

    async fn broadcast_block(&self, _block: &Block) -> Result<(), NetworkError> {
        tracing::info!("Mock: block broadcast");
        Ok(())
    }
}
