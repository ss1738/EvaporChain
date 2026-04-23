pub mod chain_proof;
pub mod evaporation_proof;
#[cfg(feature = "nova")]
pub mod nova;
pub mod privacy;

use evaporchain_types::Block;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─────────────────────────── Errors ─────────────────────────────────────

#[derive(Debug, Error)]
pub enum ProvingError {
    #[error("folding failed: {0}")]
    FoldingFailed(String),
    #[error("compression failed: {0}")]
    CompressionFailed(String),
    #[error("verification failed: {0}")]
    VerificationFailed(String),
    #[error("no blocks folded yet")]
    NoBlocksFolded,
    #[error("Nova feature not enabled — recompile with --features nova")]
    NovaNotAvailable,
}

// ─────────────────────────── Compressed Proof ───────────────────────────

/// Serialized compressed proof produced by any proving engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressedProof {
    /// Serialized proof bytes (format depends on the backend).
    pub proof_bytes: Vec<u8>,
    /// Number of blocks (IVC steps) folded into this proof.
    pub num_steps: usize,
    /// Serialized initial state (z0) for verification.
    pub z0_bytes: Vec<u8>,
}

impl CompressedProof {
    /// Size of the proof in bytes.
    pub fn size(&self) -> usize {
        self.proof_bytes.len()
    }
}

// ─────────────────────────── Trait ───────────────────────────────────────

/// Trait for IVC/folding-based proving engines.
pub trait ProvingEngine: Send + Sync {
    /// Fold a new block's state transition into the running proof accumulator.
    fn fold_block(
        &mut self,
        block: &Block,
        old_state_root: [u8; 32],
        new_state_root: [u8; 32],
    ) -> Result<(), ProvingError>;

    /// Compress the accumulated IVC proof into a succinct SNARK.
    fn get_proof(&self) -> Result<CompressedProof, ProvingError>;

    /// Verify a compressed proof against expected parameters.
    fn verify_proof(
        &self,
        proof: &CompressedProof,
        num_blocks: usize,
        genesis_state: [u8; 32],
    ) -> Result<bool, ProvingError>;

    /// Size of the running IVC accumulator in bytes.
    fn accumulator_size(&self) -> usize;

    /// Number of blocks folded so far.
    fn num_blocks_folded(&self) -> usize;

    /// Duration of the last fold operation in microseconds (0 for mock).
    fn last_fold_time_us(&self) -> u64;
}

// ─────────────────────────── MockProver ──────────────────────────────────

/// Mock prover that skips actual proof generation.
/// Used for fast testing and the demo node.
pub struct MockProver {
    num_folded: usize,
}

impl MockProver {
    pub fn new() -> Self {
        Self { num_folded: 0 }
    }
}

impl Default for MockProver {
    fn default() -> Self {
        Self::new()
    }
}

impl ProvingEngine for MockProver {
    fn fold_block(
        &mut self,
        _block: &Block,
        _old_state_root: [u8; 32],
        _new_state_root: [u8; 32],
    ) -> Result<(), ProvingError> {
        self.num_folded += 1;
        Ok(())
    }

    fn get_proof(&self) -> Result<CompressedProof, ProvingError> {
        if self.num_folded == 0 {
            return Err(ProvingError::NoBlocksFolded);
        }
        Ok(CompressedProof {
            proof_bytes: vec![0u8; 32],
            num_steps: self.num_folded,
            z0_bytes: vec![0u8; 16],
        })
    }

    fn verify_proof(
        &self,
        proof: &CompressedProof,
        num_blocks: usize,
        _genesis_state: [u8; 32],
    ) -> Result<bool, ProvingError> {
        Ok(proof.num_steps == num_blocks)
    }

    fn accumulator_size(&self) -> usize {
        0
    }

    fn num_blocks_folded(&self) -> usize {
        self.num_folded
    }

    fn last_fold_time_us(&self) -> u64 {
        0
    }
}

// ─────────────────────────── Tests ───────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_types::Block;

    fn dummy_block(num: u64, epoch: u64) -> Block {
        Block {
            number: num,
            epoch,
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            transactions: vec![],
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
            oracle_state_root: None,
            shard_count: None,
            da_row_roots: vec![],
            da_col_roots: vec![],
        }
    }

    #[test]
    fn test_mock_fold_succeeds() {
        let mut prover = MockProver::new();
        let block = dummy_block(1, 1);
        assert!(prover.fold_block(&block, [0; 32], [1; 32]).is_ok());
        assert_eq!(prover.num_blocks_folded(), 1);
    }

    #[test]
    fn test_mock_multiple_folds() {
        let mut prover = MockProver::new();
        for i in 1..=5 {
            let block = dummy_block(i, i);
            prover.fold_block(&block, [0; 32], [1; 32]).unwrap();
        }
        assert_eq!(prover.num_blocks_folded(), 5);
        assert_eq!(prover.accumulator_size(), 0);
        assert_eq!(prover.last_fold_time_us(), 0);
    }

    #[test]
    fn test_mock_get_proof() {
        let mut prover = MockProver::new();
        prover.fold_block(&dummy_block(1, 1), [0; 32], [1; 32]).unwrap();

        let proof = prover.get_proof().unwrap();
        assert_eq!(proof.num_steps, 1);
        assert!(!proof.proof_bytes.is_empty());
    }

    #[test]
    fn test_mock_no_blocks_folded_error() {
        let prover = MockProver::new();
        assert!(prover.get_proof().is_err());
    }

    #[test]
    fn test_mock_verify_correct_count() {
        let mut prover = MockProver::new();
        for i in 1..=3 {
            prover.fold_block(&dummy_block(i, i), [0; 32], [1; 32]).unwrap();
        }
        let proof = prover.get_proof().unwrap();

        assert!(prover.verify_proof(&proof, 3, [0; 32]).unwrap());
        assert!(!prover.verify_proof(&proof, 5, [0; 32]).unwrap());
    }

    #[test]
    fn test_mock_as_trait_object() {
        let mut prover: Box<dyn ProvingEngine> = Box::new(MockProver::new());
        prover.fold_block(&dummy_block(1, 1), [0; 32], [1; 32]).unwrap();
        assert_eq!(prover.num_blocks_folded(), 1);
    }
}
