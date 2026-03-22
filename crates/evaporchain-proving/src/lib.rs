use evaporchain_types::Block;
use thiserror::Error;

/// Errors that can occur during proof generation/verification.
#[derive(Debug, Error)]
pub enum ProvingError {
    #[error("folding failed: {0}")]
    FoldingFailed(String),
    #[error("compression failed: {0}")]
    CompressionFailed(String),
    #[error("verification failed: {0}")]
    VerificationFailed(String),
}

/// Trait for ZK proving engines (IVC/folding-based).
pub trait ProvingEngine: Send + Sync {
    /// Fold a new block into the running IVC proof.
    fn fold_block(&mut self, block: &Block) -> Result<(), ProvingError>;
    /// Compress the folded proof into a succinct SNARK.
    fn compress(&self) -> Result<Vec<u8>, ProvingError>;
    /// Verify a compressed proof.
    fn verify(proof: &[u8]) -> Result<bool, ProvingError>;
}

/// Mock prover that skips actual proof generation.
pub struct MockProver;

impl ProvingEngine for MockProver {
    fn fold_block(&mut self, _block: &Block) -> Result<(), ProvingError> {
        tracing::info!("Mock: block folded (no-op)");
        Ok(())
    }

    fn compress(&self) -> Result<Vec<u8>, ProvingError> {
        tracing::info!("Mock: proof compressed (no-op)");
        Ok(vec![0u8; 32])
    }

    fn verify(_proof: &[u8]) -> Result<bool, ProvingError> {
        tracing::info!("Mock: proof verified (no-op)");
        Ok(true)
    }
}
