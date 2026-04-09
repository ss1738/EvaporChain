//! Recursive Proof Compression for Instant Chain Sync
//!
//! Compresses the entire EvaporChain state history into a single succinct proof.
//! A light client can verify one proof instead of replaying thousands of blocks.
//!
//! Architecture:
//!   1. **ChainProver** — Incrementally folds blocks into a running IVC accumulator
//!   2. **ProofCheckpoint** — Serializable snapshot at a block height (save/resume)
//!   3. **ChainProof** — Self-contained proof: "genesis → block N produced state S"
//!   4. **ProofChain** — Links range proofs: [0→1000] + [1000→2000] = [0→2000]
//!   5. **LightClientVerifier** — Standalone verifier for instant sync
//!
//! Key innovation: the proof binds thermodynamic state (energy decay, evaporation)
//! so light clients trust not just balances but the entire decay model.

use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::{debug, info};

use crate::{CompressedProof, ProvingEngine, ProvingError};
use evaporchain_types::Block;

// ─── Chain Proof ───────────────────────────────────────────────────────────

/// A self-contained proof that the chain transitioned from genesis to a
/// specific block height, producing a specific state root.
///
/// This is what gets sent to light clients for instant sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainProof {
    /// The compressed SNARK proof.
    pub proof: CompressedProof,
    /// Genesis state root (public input — verifier must know this).
    pub genesis_state_root: [u8; 32],
    /// Final state root at `block_height` (the trusted output).
    pub final_state_root: [u8; 32],
    /// Block height this proof covers (genesis=0 → block_height).
    pub block_height: u64,
    /// Epoch at the final block.
    pub final_epoch: u64,
    /// Proof generation timestamp (unix seconds).
    pub created_at: u64,
    /// Size of the proof in bytes (for bandwidth estimation).
    pub proof_size_bytes: usize,
    /// Number of IVC steps (blocks folded).
    pub num_steps: usize,
}

impl ChainProof {
    /// Proof size in bytes.
    pub fn size(&self) -> usize {
        self.proof_size_bytes
    }

    /// Compression ratio: blocks proven per byte of proof.
    pub fn compression_ratio(&self) -> f64 {
        if self.proof_size_bytes == 0 {
            return 0.0;
        }
        self.block_height as f64 / self.proof_size_bytes as f64
    }
}

// ─── Proof Checkpoint ──────────────────────────────────────────────────────

/// Serializable snapshot of the proving state at a specific block height.
/// Allows pausing and resuming proof generation without re-proving from genesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofCheckpoint {
    /// Block height at which this checkpoint was taken.
    pub block_height: u64,
    /// Epoch at checkpoint.
    pub epoch: u64,
    /// State root at checkpoint.
    pub state_root: [u8; 32],
    /// Genesis state root (needed to resume).
    pub genesis_state_root: [u8; 32],
    /// Serialized IVC accumulator (backend-specific bytes).
    pub accumulator_bytes: Vec<u8>,
    /// Number of steps folded so far.
    pub num_steps: usize,
    /// Checkpoint creation timestamp.
    pub created_at: u64,
}

impl ProofCheckpoint {
    pub fn size(&self) -> usize {
        self.accumulator_bytes.len()
    }
}

// ─── Proof Chain ───────────────────────────────────────────────────────────

/// A chain of range proofs that together cover a contiguous block range.
/// Each segment proves a sub-range; the chain verifies they link correctly.
///
/// Example: [0→1000] + [1000→2000] + [2000→3000] = [0→3000]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofChain {
    /// Ordered segments, each covering a contiguous range.
    pub segments: Vec<ProofSegment>,
}

/// A single segment in a proof chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofSegment {
    /// The compressed proof for this range.
    pub proof: CompressedProof,
    /// Starting block height (inclusive).
    pub start_height: u64,
    /// Ending block height (inclusive).
    pub end_height: u64,
    /// State root at start of this segment.
    pub start_state_root: [u8; 32],
    /// State root at end of this segment.
    pub end_state_root: [u8; 32],
    /// Number of IVC steps in this segment.
    pub num_steps: usize,
}

impl ProofChain {
    /// Create a new empty proof chain.
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    /// Add a segment to the chain. Validates continuity.
    pub fn add_segment(&mut self, segment: ProofSegment) -> Result<(), ProvingError> {
        if let Some(last) = self.segments.last() {
            // Segments must be contiguous
            if segment.start_height != last.end_height + 1 {
                return Err(ProvingError::FoldingFailed(format!(
                    "non-contiguous segment: last ends at {}, new starts at {}",
                    last.end_height, segment.start_height
                )));
            }
            // State roots must chain
            if segment.start_state_root != last.end_state_root {
                return Err(ProvingError::VerificationFailed(
                    "state root mismatch between segments".into(),
                ));
            }
        }
        self.segments.push(segment);
        Ok(())
    }

    /// Total block range covered.
    pub fn block_range(&self) -> Option<(u64, u64)> {
        if self.segments.is_empty() {
            return None;
        }
        Some((
            self.segments.first().unwrap().start_height,
            self.segments.last().unwrap().end_height,
        ))
    }

    /// Total number of blocks covered.
    pub fn total_blocks(&self) -> u64 {
        match self.block_range() {
            Some((start, end)) => end - start + 1,
            None => 0,
        }
    }

    /// Total proof size across all segments.
    pub fn total_size(&self) -> usize {
        self.segments
            .iter()
            .map(|s| s.proof.size())
            .sum()
    }

    /// Number of segments.
    pub fn num_segments(&self) -> usize {
        self.segments.len()
    }

    /// Genesis state root (from the first segment).
    pub fn genesis_state_root(&self) -> Option<[u8; 32]> {
        self.segments.first().map(|s| s.start_state_root)
    }

    /// Final state root (from the last segment).
    pub fn final_state_root(&self) -> Option<[u8; 32]> {
        self.segments.last().map(|s| s.end_state_root)
    }
}

impl Default for ProofChain {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Chain Prover ──────────────────────────────────────────────────────────

/// Orchestrates recursive proof generation for the entire chain.
/// Wraps any `ProvingEngine` and adds checkpointing, chain proof
/// generation, and metrics.
pub struct ChainProver {
    /// The underlying IVC proving engine.
    engine: Box<dyn ProvingEngine>,
    /// Genesis state root.
    genesis_state_root: [u8; 32],
    /// Current block height.
    current_height: u64,
    /// Current epoch.
    current_epoch: u64,
    /// Latest state root.
    latest_state_root: [u8; 32],
    /// Timing metrics: total proving time in microseconds.
    total_prove_time_us: u64,
    /// Checkpoint interval: auto-checkpoint every N blocks.
    checkpoint_interval: u64,
    /// Stored checkpoints.
    checkpoints: Vec<ProofCheckpoint>,
    /// Blocks since last checkpoint.
    blocks_since_checkpoint: u64,
}

impl ChainProver {
    /// Create a new chain prover wrapping any proving engine.
    pub fn new(
        engine: Box<dyn ProvingEngine>,
        genesis_state_root: [u8; 32],
        checkpoint_interval: u64,
    ) -> Self {
        Self {
            engine,
            genesis_state_root,
            current_height: 0,
            current_epoch: 0,
            latest_state_root: genesis_state_root,
            total_prove_time_us: 0,
            checkpoint_interval,
            checkpoints: Vec::new(),
            blocks_since_checkpoint: 0,
        }
    }

    /// Fold a new block into the running proof.
    /// Automatically creates checkpoints at the configured interval.
    pub fn fold_block(
        &mut self,
        block: &Block,
        new_state_root: [u8; 32],
    ) -> Result<FoldResult, ProvingError> {
        let old_state_root = self.latest_state_root;
        let start = Instant::now();

        self.engine
            .fold_block(block, old_state_root, new_state_root)?;

        let fold_time_us = start.elapsed().as_micros() as u64;
        self.total_prove_time_us += fold_time_us;
        self.current_height = block.number;
        self.current_epoch = block.epoch;
        self.latest_state_root = new_state_root;
        self.blocks_since_checkpoint += 1;

        let mut checkpointed = false;

        // Auto-checkpoint
        if self.checkpoint_interval > 0
            && self.blocks_since_checkpoint >= self.checkpoint_interval
        {
            self.create_checkpoint()?;
            checkpointed = true;
        }

        debug!(
            block = block.number,
            fold_time_us,
            total_folded = self.engine.num_blocks_folded(),
            "Block folded into chain proof"
        );

        Ok(FoldResult {
            block_height: block.number,
            fold_time_us,
            accumulator_size: self.engine.accumulator_size(),
            checkpointed,
        })
    }

    /// Create a checkpoint at the current height.
    pub fn create_checkpoint(&mut self) -> Result<&ProofCheckpoint, ProvingError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let checkpoint = ProofCheckpoint {
            block_height: self.current_height,
            epoch: self.current_epoch,
            state_root: self.latest_state_root,
            genesis_state_root: self.genesis_state_root,
            accumulator_bytes: Vec::new(), // In production: serialize the IVC accumulator
            num_steps: self.engine.num_blocks_folded(),
            created_at: now,
        };

        info!(
            height = self.current_height,
            num_steps = checkpoint.num_steps,
            "Chain proof checkpoint created"
        );

        self.checkpoints.push(checkpoint);
        self.blocks_since_checkpoint = 0;
        Ok(self.checkpoints.last().unwrap())
    }

    /// Generate a compressed chain proof covering genesis → current height.
    pub fn generate_chain_proof(&self) -> Result<ChainProof, ProvingError> {
        let start = Instant::now();

        let proof = self.engine.get_proof()?;
        let proof_size = proof.size();

        let compress_time = start.elapsed().as_micros() as u64;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        info!(
            height = self.current_height,
            proof_size_bytes = proof_size,
            compress_time_us = compress_time,
            num_steps = proof.num_steps,
            "Chain proof generated"
        );

        Ok(ChainProof {
            num_steps: proof.num_steps,
            proof,
            genesis_state_root: self.genesis_state_root,
            final_state_root: self.latest_state_root,
            block_height: self.current_height,
            final_epoch: self.current_epoch,
            created_at: now,
            proof_size_bytes: proof_size,
        })
    }

    /// Verify a chain proof against this prover's genesis state.
    pub fn verify_chain_proof(&self, chain_proof: &ChainProof) -> Result<bool, ProvingError> {
        // Genesis must match
        if chain_proof.genesis_state_root != self.genesis_state_root {
            return Ok(false);
        }

        self.engine.verify_proof(
            &chain_proof.proof,
            chain_proof.num_steps,
            self.genesis_state_root,
        )
    }

    /// Get all checkpoints.
    pub fn checkpoints(&self) -> &[ProofCheckpoint] {
        &self.checkpoints
    }

    /// Get the latest checkpoint.
    pub fn latest_checkpoint(&self) -> Option<&ProofCheckpoint> {
        self.checkpoints.last()
    }

    /// Current block height.
    pub fn height(&self) -> u64 {
        self.current_height
    }

    /// Current epoch.
    pub fn epoch(&self) -> u64 {
        self.current_epoch
    }

    /// Total number of blocks folded.
    pub fn blocks_folded(&self) -> usize {
        self.engine.num_blocks_folded()
    }

    /// Current accumulator size in bytes.
    pub fn accumulator_size(&self) -> usize {
        self.engine.accumulator_size()
    }

    /// Total proving time in microseconds.
    pub fn total_prove_time_us(&self) -> u64 {
        self.total_prove_time_us
    }

    /// Average fold time per block in microseconds.
    pub fn avg_fold_time_us(&self) -> u64 {
        let n = self.engine.num_blocks_folded() as u64;
        if n == 0 {
            return 0;
        }
        self.total_prove_time_us / n
    }

    /// Metrics snapshot.
    pub fn metrics(&self) -> ChainProverMetrics {
        ChainProverMetrics {
            block_height: self.current_height,
            epoch: self.current_epoch,
            blocks_folded: self.engine.num_blocks_folded(),
            accumulator_size_bytes: self.engine.accumulator_size(),
            total_prove_time_us: self.total_prove_time_us,
            avg_fold_time_us: self.avg_fold_time_us(),
            last_fold_time_us: self.engine.last_fold_time_us(),
            num_checkpoints: self.checkpoints.len(),
        }
    }
}

/// Result of folding a single block.
#[derive(Debug)]
pub struct FoldResult {
    pub block_height: u64,
    pub fold_time_us: u64,
    pub accumulator_size: usize,
    pub checkpointed: bool,
}

/// Metrics for the chain prover.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainProverMetrics {
    pub block_height: u64,
    pub epoch: u64,
    pub blocks_folded: usize,
    pub accumulator_size_bytes: usize,
    pub total_prove_time_us: u64,
    pub avg_fold_time_us: u64,
    pub last_fold_time_us: u64,
    pub num_checkpoints: usize,
}

// ─── Light Client Verifier ─────────────────────────────────────────────────

/// Standalone verifier for light clients performing instant chain sync.
///
/// A light client:
///   1. Obtains a `ChainProof` from a full node
///   2. Verifies it against the known genesis state root
///   3. Trusts the `final_state_root` without replaying any blocks
///   4. Can optionally verify a `ProofChain` (multiple segments)
pub struct LightClientVerifier {
    /// The proving engine (only used for verification — no folding needed).
    engine: Box<dyn ProvingEngine>,
    /// Known genesis state root.
    genesis_state_root: [u8; 32],
}

/// Result of light client verification.
#[derive(Debug, Clone)]
pub struct SyncResult {
    /// Whether the proof verified successfully.
    pub valid: bool,
    /// The trusted state root (if valid).
    pub trusted_state_root: Option<[u8; 32]>,
    /// Block height the proof covers.
    pub block_height: u64,
    /// Final epoch.
    pub epoch: u64,
    /// Verification time in microseconds.
    pub verify_time_us: u64,
    /// Proof size in bytes.
    pub proof_size_bytes: usize,
}

impl LightClientVerifier {
    /// Create a new verifier with a proving engine and known genesis.
    pub fn new(engine: Box<dyn ProvingEngine>, genesis_state_root: [u8; 32]) -> Self {
        Self {
            engine,
            genesis_state_root,
        }
    }

    /// Verify a chain proof and return the trusted state.
    /// This is the core instant sync operation.
    pub fn verify_and_sync(&self, chain_proof: &ChainProof) -> Result<SyncResult, ProvingError> {
        let start = Instant::now();

        // 1. Genesis must match
        if chain_proof.genesis_state_root != self.genesis_state_root {
            return Ok(SyncResult {
                valid: false,
                trusted_state_root: None,
                block_height: chain_proof.block_height,
                epoch: chain_proof.final_epoch,
                verify_time_us: start.elapsed().as_micros() as u64,
                proof_size_bytes: chain_proof.proof_size_bytes,
            });
        }

        // 2. Verify the SNARK proof
        let valid = self.engine.verify_proof(
            &chain_proof.proof,
            chain_proof.num_steps,
            self.genesis_state_root,
        )?;

        let verify_time_us = start.elapsed().as_micros() as u64;

        info!(
            valid,
            height = chain_proof.block_height,
            verify_time_us,
            proof_size = chain_proof.proof_size_bytes,
            "Light client sync verification"
        );

        Ok(SyncResult {
            valid,
            trusted_state_root: if valid {
                Some(chain_proof.final_state_root)
            } else {
                None
            },
            block_height: chain_proof.block_height,
            epoch: chain_proof.final_epoch,
            verify_time_us,
            proof_size_bytes: chain_proof.proof_size_bytes,
        })
    }

    /// Verify a proof chain (multiple linked segments).
    /// Checks each segment individually and verifies continuity.
    pub fn verify_proof_chain(
        &self,
        chain: &ProofChain,
    ) -> Result<ProofChainVerification, ProvingError> {
        if chain.segments.is_empty() {
            return Err(ProvingError::NoBlocksFolded);
        }

        let start = Instant::now();
        let mut segment_results = Vec::with_capacity(chain.segments.len());

        // Verify first segment starts at genesis
        let first = &chain.segments[0];
        if first.start_state_root != self.genesis_state_root {
            return Ok(ProofChainVerification {
                valid: false,
                trusted_state_root: None,
                total_blocks: chain.total_blocks(),
                segments_verified: 0,
                total_verify_time_us: start.elapsed().as_micros() as u64,
                total_proof_size: chain.total_size(),
            });
        }

        // Verify each segment
        for (i, segment) in chain.segments.iter().enumerate() {
            // Check continuity with previous segment
            if i > 0 {
                let prev = &chain.segments[i - 1];
                if segment.start_state_root != prev.end_state_root {
                    return Ok(ProofChainVerification {
                        valid: false,
                        trusted_state_root: None,
                        total_blocks: chain.total_blocks(),
                        segments_verified: i,
                        total_verify_time_us: start.elapsed().as_micros() as u64,
                        total_proof_size: chain.total_size(),
                    });
                }
                if segment.start_height != prev.end_height + 1 {
                    return Ok(ProofChainVerification {
                        valid: false,
                        trusted_state_root: None,
                        total_blocks: chain.total_blocks(),
                        segments_verified: i,
                        total_verify_time_us: start.elapsed().as_micros() as u64,
                        total_proof_size: chain.total_size(),
                    });
                }
            }

            // Verify the segment's proof
            let genesis_for_segment = if i == 0 {
                self.genesis_state_root
            } else {
                segment.start_state_root
            };

            let valid = self.engine.verify_proof(
                &segment.proof,
                segment.num_steps,
                genesis_for_segment,
            )?;

            segment_results.push(valid);

            if !valid {
                return Ok(ProofChainVerification {
                    valid: false,
                    trusted_state_root: None,
                    total_blocks: chain.total_blocks(),
                    segments_verified: i,
                    total_verify_time_us: start.elapsed().as_micros() as u64,
                    total_proof_size: chain.total_size(),
                });
            }
        }

        Ok(ProofChainVerification {
            valid: true,
            trusted_state_root: chain.final_state_root(),
            total_blocks: chain.total_blocks(),
            segments_verified: chain.segments.len(),
            total_verify_time_us: start.elapsed().as_micros() as u64,
            total_proof_size: chain.total_size(),
        })
    }
}

/// Result of verifying a proof chain.
#[derive(Debug, Clone)]
pub struct ProofChainVerification {
    pub valid: bool,
    pub trusted_state_root: Option<[u8; 32]>,
    pub total_blocks: u64,
    pub segments_verified: usize,
    pub total_verify_time_us: u64,
    pub total_proof_size: usize,
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockProver;

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
        }
    }

    fn state_root(val: u8) -> [u8; 32] {
        let mut r = [0u8; 32];
        r[0] = val;
        r
    }

    // ── ChainProver Tests ──

    #[test]
    fn test_chain_prover_fold_blocks() {
        let engine = Box::new(MockProver::new());
        let genesis = state_root(0);
        let mut prover = ChainProver::new(engine, genesis, 0);

        for i in 1..=10 {
            let block = dummy_block(i, i);
            let result = prover.fold_block(&block, state_root(i as u8)).unwrap();
            assert_eq!(result.block_height, i);
        }

        assert_eq!(prover.height(), 10);
        assert_eq!(prover.blocks_folded(), 10);
    }

    #[test]
    fn test_chain_prover_auto_checkpoint() {
        let engine = Box::new(MockProver::new());
        let genesis = state_root(0);
        let mut prover = ChainProver::new(engine, genesis, 5); // Checkpoint every 5 blocks

        for i in 1..=12 {
            let block = dummy_block(i, i);
            prover.fold_block(&block, state_root(i as u8)).unwrap();
        }

        // Should have 2 checkpoints: at block 5 and block 10
        assert_eq!(prover.checkpoints().len(), 2);
        assert_eq!(prover.checkpoints()[0].block_height, 5);
        assert_eq!(prover.checkpoints()[1].block_height, 10);
    }

    #[test]
    fn test_chain_prover_manual_checkpoint() {
        let engine = Box::new(MockProver::new());
        let genesis = state_root(0);
        let mut prover = ChainProver::new(engine, genesis, 0); // No auto-checkpoint

        for i in 1..=3 {
            let block = dummy_block(i, i);
            prover.fold_block(&block, state_root(i as u8)).unwrap();
        }

        let cp = prover.create_checkpoint().unwrap();
        assert_eq!(cp.block_height, 3);
        assert_eq!(cp.num_steps, 3);
    }

    #[test]
    fn test_chain_proof_generation() {
        let engine = Box::new(MockProver::new());
        let genesis = state_root(0);
        let mut prover = ChainProver::new(engine, genesis, 0);

        for i in 1..=5 {
            let block = dummy_block(i, i);
            prover.fold_block(&block, state_root(i as u8)).unwrap();
        }

        let chain_proof = prover.generate_chain_proof().unwrap();
        assert_eq!(chain_proof.block_height, 5);
        assert_eq!(chain_proof.genesis_state_root, genesis);
        assert_eq!(chain_proof.final_state_root, state_root(5));
        assert_eq!(chain_proof.num_steps, 5);
        assert!(chain_proof.proof_size_bytes > 0);
    }

    #[test]
    fn test_chain_proof_verify() {
        let genesis = state_root(0);

        // Prover
        let engine1 = Box::new(MockProver::new());
        let mut prover = ChainProver::new(engine1, genesis, 0);
        for i in 1..=5 {
            prover
                .fold_block(&dummy_block(i, i), state_root(i as u8))
                .unwrap();
        }
        let chain_proof = prover.generate_chain_proof().unwrap();

        // Verify with the prover
        assert!(prover.verify_chain_proof(&chain_proof).unwrap());
    }

    #[test]
    fn test_chain_proof_wrong_genesis_rejected() {
        let genesis = state_root(0);

        let engine = Box::new(MockProver::new());
        let mut prover = ChainProver::new(engine, genesis, 0);
        prover
            .fold_block(&dummy_block(1, 1), state_root(1))
            .unwrap();
        let mut chain_proof = prover.generate_chain_proof().unwrap();

        // Tamper with genesis
        chain_proof.genesis_state_root = state_root(99);
        assert!(!prover.verify_chain_proof(&chain_proof).unwrap());
    }

    #[test]
    fn test_chain_proof_compression_ratio() {
        let engine = Box::new(MockProver::new());
        let genesis = state_root(0);
        let mut prover = ChainProver::new(engine, genesis, 0);

        for i in 1..=100 {
            prover
                .fold_block(&dummy_block(i, i), state_root((i % 256) as u8))
                .unwrap();
        }

        let proof = prover.generate_chain_proof().unwrap();
        let ratio = proof.compression_ratio();
        assert!(ratio > 0.0, "compression ratio should be positive");
    }

    // ── Light Client Verifier Tests ──

    #[test]
    fn test_light_client_instant_sync() {
        let genesis = state_root(0);

        // Full node generates proof
        let engine = Box::new(MockProver::new());
        let mut prover = ChainProver::new(engine, genesis, 0);
        for i in 1..=20 {
            prover
                .fold_block(&dummy_block(i, i), state_root(i as u8))
                .unwrap();
        }
        let chain_proof = prover.generate_chain_proof().unwrap();

        // Light client verifies
        let verifier = LightClientVerifier::new(Box::new(MockProver::new()), genesis);
        let sync = verifier.verify_and_sync(&chain_proof).unwrap();

        assert!(sync.valid);
        assert_eq!(sync.trusted_state_root, Some(state_root(20)));
        assert_eq!(sync.block_height, 20);
    }

    #[test]
    fn test_light_client_rejects_wrong_genesis() {
        let genesis = state_root(0);

        let engine = Box::new(MockProver::new());
        let mut prover = ChainProver::new(engine, genesis, 0);
        prover
            .fold_block(&dummy_block(1, 1), state_root(1))
            .unwrap();
        let chain_proof = prover.generate_chain_proof().unwrap();

        // Verifier with different genesis
        let verifier = LightClientVerifier::new(Box::new(MockProver::new()), state_root(99));
        let sync = verifier.verify_and_sync(&chain_proof).unwrap();

        assert!(!sync.valid);
        assert_eq!(sync.trusted_state_root, None);
    }

    // ── Proof Chain Tests ──

    #[test]
    fn test_proof_chain_build() {
        let mut chain = ProofChain::new();

        let seg1 = ProofSegment {
            proof: CompressedProof {
                proof_bytes: vec![1, 2, 3],
                num_steps: 100,
                z0_bytes: vec![],
            },
            start_height: 0,
            end_height: 99,
            start_state_root: state_root(0),
            end_state_root: state_root(1),
            num_steps: 100,
        };

        let seg2 = ProofSegment {
            proof: CompressedProof {
                proof_bytes: vec![4, 5, 6],
                num_steps: 100,
                z0_bytes: vec![],
            },
            start_height: 100,
            end_height: 199,
            start_state_root: state_root(1), // Must match seg1.end
            end_state_root: state_root(2),
            num_steps: 100,
        };

        chain.add_segment(seg1).unwrap();
        chain.add_segment(seg2).unwrap();

        assert_eq!(chain.num_segments(), 2);
        assert_eq!(chain.total_blocks(), 200);
        assert_eq!(chain.block_range(), Some((0, 199)));
        assert_eq!(chain.genesis_state_root(), Some(state_root(0)));
        assert_eq!(chain.final_state_root(), Some(state_root(2)));
    }

    #[test]
    fn test_proof_chain_rejects_gap() {
        let mut chain = ProofChain::new();

        chain
            .add_segment(ProofSegment {
                proof: CompressedProof {
                    proof_bytes: vec![],
                    num_steps: 10,
                    z0_bytes: vec![],
                },
                start_height: 0,
                end_height: 9,
                start_state_root: state_root(0),
                end_state_root: state_root(1),
                num_steps: 10,
            })
            .unwrap();

        // Gap: starts at 20 instead of 10
        let result = chain.add_segment(ProofSegment {
            proof: CompressedProof {
                proof_bytes: vec![],
                num_steps: 10,
                z0_bytes: vec![],
            },
            start_height: 20,
            end_height: 29,
            start_state_root: state_root(1),
            end_state_root: state_root(2),
            num_steps: 10,
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_proof_chain_rejects_state_mismatch() {
        let mut chain = ProofChain::new();

        chain
            .add_segment(ProofSegment {
                proof: CompressedProof {
                    proof_bytes: vec![],
                    num_steps: 10,
                    z0_bytes: vec![],
                },
                start_height: 0,
                end_height: 9,
                start_state_root: state_root(0),
                end_state_root: state_root(1),
                num_steps: 10,
            })
            .unwrap();

        // State mismatch: start_state_root should be state_root(1)
        let result = chain.add_segment(ProofSegment {
            proof: CompressedProof {
                proof_bytes: vec![],
                num_steps: 10,
                z0_bytes: vec![],
            },
            start_height: 10,
            end_height: 19,
            start_state_root: state_root(99), // Wrong!
            end_state_root: state_root(2),
            num_steps: 10,
        });
        assert!(result.is_err());
    }

    #[test]
    fn test_light_client_verify_proof_chain() {
        let genesis = state_root(0);

        // Build a proof chain with MockProver segments
        let mut chain = ProofChain::new();

        // Segment 1: blocks 0-9
        let engine1 = Box::new(MockProver::new());
        let mut prover1 = ChainProver::new(engine1, genesis, 0);
        for i in 1..=10 {
            prover1
                .fold_block(&dummy_block(i, i), state_root(1))
                .unwrap();
        }
        let proof1 = prover1.generate_chain_proof().unwrap();

        chain
            .add_segment(ProofSegment {
                proof: proof1.proof,
                start_height: 0,
                end_height: 9,
                start_state_root: genesis,
                end_state_root: state_root(1),
                num_steps: 10,
            })
            .unwrap();

        // Segment 2: blocks 10-19
        let engine2 = Box::new(MockProver::new());
        let mut prover2 = ChainProver::new(engine2, state_root(1), 0);
        for i in 11..=20 {
            prover2
                .fold_block(&dummy_block(i, i), state_root(2))
                .unwrap();
        }
        let proof2 = prover2.generate_chain_proof().unwrap();

        chain
            .add_segment(ProofSegment {
                proof: proof2.proof,
                start_height: 10,
                end_height: 19,
                start_state_root: state_root(1),
                end_state_root: state_root(2),
                num_steps: 10,
            })
            .unwrap();

        // Light client verifies the chain
        let verifier = LightClientVerifier::new(Box::new(MockProver::new()), genesis);
        let result = verifier.verify_proof_chain(&chain).unwrap();

        assert!(result.valid);
        assert_eq!(result.trusted_state_root, Some(state_root(2)));
        assert_eq!(result.total_blocks, 20);
        assert_eq!(result.segments_verified, 2);
    }

    // ── Metrics Tests ──

    #[test]
    fn test_chain_prover_metrics() {
        let engine = Box::new(MockProver::new());
        let genesis = state_root(0);
        let mut prover = ChainProver::new(engine, genesis, 5);

        for i in 1..=7 {
            prover
                .fold_block(&dummy_block(i, i), state_root(i as u8))
                .unwrap();
        }

        let metrics = prover.metrics();
        assert_eq!(metrics.block_height, 7);
        assert_eq!(metrics.blocks_folded, 7);
        assert_eq!(metrics.num_checkpoints, 1); // Auto at block 5
    }

    #[test]
    fn test_empty_prover_no_proof() {
        let engine = Box::new(MockProver::new());
        let genesis = state_root(0);
        let prover = ChainProver::new(engine, genesis, 0);

        let result = prover.generate_chain_proof();
        assert!(result.is_err()); // No blocks folded
    }

    #[test]
    fn test_proof_chain_empty() {
        let chain = ProofChain::new();
        assert_eq!(chain.num_segments(), 0);
        assert_eq!(chain.total_blocks(), 0);
        assert_eq!(chain.block_range(), None);
    }

    #[test]
    fn test_single_block_proof() {
        let genesis = state_root(0);
        let engine = Box::new(MockProver::new());
        let mut prover = ChainProver::new(engine, genesis, 0);

        prover
            .fold_block(&dummy_block(1, 1), state_root(1))
            .unwrap();

        let proof = prover.generate_chain_proof().unwrap();
        assert_eq!(proof.block_height, 1);
        assert_eq!(proof.num_steps, 1);

        let verifier = LightClientVerifier::new(Box::new(MockProver::new()), genesis);
        let sync = verifier.verify_and_sync(&proof).unwrap();
        assert!(sync.valid);
    }
}
