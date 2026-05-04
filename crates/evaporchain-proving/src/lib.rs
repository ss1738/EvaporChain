pub mod async_fold;
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
//
// H-19 FIX: MockProver is gated behind `#[cfg(any(test, feature = "test-utils"))]`
// so it cannot be compiled into release/production binaries. This prevents
// accidental fallback to a prover that accepts any proof.

/// Mock prover that skips actual proof generation.
/// Only available in test builds or when the `test-utils` feature is enabled.
///
/// # Safety
/// This prover performs NO cryptographic verification. It must NEVER be used
/// in production. The `#[cfg]` gate enforces this at compile time.
#[cfg(any(test, feature = "test-utils"))]
pub struct MockProver {
    num_folded: usize,
}

#[cfg(any(test, feature = "test-utils"))]
impl MockProver {
    pub fn new() -> Self {
        Self { num_folded: 0 }
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl Default for MockProver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-utils"))]
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
            chain_id: String::new(),
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
            protocol_version: 0,
            state_root_version: 0,
            submit_epoch_hints: vec![],
            parents: vec![],
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
        prover
            .fold_block(&dummy_block(1, 1), [0; 32], [1; 32])
            .unwrap();

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
            prover
                .fold_block(&dummy_block(i, i), [0; 32], [1; 32])
                .unwrap();
        }
        let proof = prover.get_proof().unwrap();

        assert!(prover.verify_proof(&proof, 3, [0; 32]).unwrap());
        assert!(!prover.verify_proof(&proof, 5, [0; 32]).unwrap());
    }

    #[test]
    fn test_mock_as_trait_object() {
        let mut prover: Box<dyn ProvingEngine> = Box::new(MockProver::new());
        prover
            .fold_block(&dummy_block(1, 1), [0; 32], [1; 32])
            .unwrap();
        assert_eq!(prover.num_blocks_folded(), 1);
    }

    /// H-19: Verify MockProver is only available under cfg(test) or test-utils.
    /// This test exists to document the security invariant: MockProver must
    /// never be reachable in production release builds. The compile-time gate
    /// `#[cfg(any(test, feature = "test-utils"))]` on the MockProver struct
    /// ensures this — if someone removes the gate, this doc-test serves as
    /// a reminder of WHY it exists.
    #[test]
    fn test_mock_prover_is_cfg_gated() {
        // If this test compiles, MockProver is available — which is correct
        // because we are in #[cfg(test)]. The real protection is that WITHOUT
        // cfg(test) or feature="test-utils", MockProver does not exist at all.
        let prover = MockProver::new();
        assert_eq!(prover.num_blocks_folded(), 0);

        // MockProver should accept proofs in test context (it's a test helper).
        let mut prover = MockProver::new();
        prover
            .fold_block(&dummy_block(1, 1), [0; 32], [1; 32])
            .unwrap();
        let proof = prover.get_proof().unwrap();
        assert!(
            prover.verify_proof(&proof, 1, [0; 32]).unwrap(),
            "MockProver should accept valid proofs in test context"
        );
    }
}
