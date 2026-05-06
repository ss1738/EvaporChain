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

// ─────────────────────────── MockProver Fingerprint Guard ───────────────
//
// AUDIT_2026_05_06.md H-19 — defence in depth on top of the
// `#[cfg(any(test, feature = "test-utils"))]` gate.
//
// `MockProver::get_proof` produces a recognisable byte-pattern
// (`proof_bytes = [0u8; PROOF_LEN]` + `z0_bytes = [0u8; Z0_LEN]`)
// because it skips actual cryptographic work. If MockProver ever
// leaked into a release build (e.g. via accidentally enabling the
// `test-utils` feature in a release dependency tree), the proofs
// it emits would still be detectable on the wire by the
// `is_mock_prover_proof` predicate.
//
// Production verifiers SHOULD call `is_mock_prover_proof` on every
// inbound `CompressedProof` and reject matches with a hard error
// before forwarding to the cryptographic verify path. This is
// belt-and-braces — the cfg gate is the primary protection; the
// fingerprint is the catch-all.

/// Length of `MockProver`-emitted `proof_bytes`. Matches the
/// vector size in `MockProver::get_proof` impl below.
const MOCK_PROOF_BYTES_LEN: usize = 32;

/// Length of `MockProver`-emitted `z0_bytes`. Matches the vector
/// size in `MockProver::get_proof` impl below.
const MOCK_Z0_BYTES_LEN: usize = 16;

/// Detect whether a `CompressedProof` carries the all-zeros
/// fingerprint that `MockProver` emits. Returns `true` iff the
/// proof matches MockProver's output shape exactly.
///
/// Production verifiers should reject any proof for which this
/// returns `true`. False positives are extremely unlikely against
/// a real Nova-folded proof (proof_bytes from `nova-snark` are
/// hundreds-to-thousands of pseudo-random hash output bytes; the
/// chance of all-zeros is `2^(-8 * proof_bytes.len())`).
pub fn is_mock_prover_proof(proof: &CompressedProof) -> bool {
    proof.proof_bytes.len() == MOCK_PROOF_BYTES_LEN
        && proof.z0_bytes.len() == MOCK_Z0_BYTES_LEN
        && proof.proof_bytes.iter().all(|&b| b == 0)
        && proof.z0_bytes.iter().all(|&b| b == 0)
}

/// Wire-layer fingerprint check for the raw `proof_bytes` field
/// of a chain block (`Block::nova_proof: Option<Vec<u8>>`). This
/// is the version production verifiers actually call: the consensus
/// layer holds only `Vec<u8>` (per the trait-only abstraction in
/// `evaporchain-consensus::ProofVerifier`), not a deserialised
/// `CompressedProof`.
///
/// Returns `true` iff the bytes are exactly 32 zero bytes — the
/// shape `MockProver::get_proof` emits as `proof_bytes`. Real Nova
/// proofs are 100s-1000s of pseudo-random hash output bytes, so
/// length alone discriminates with overwhelming probability.
///
/// `ChainProofVerifier::verify_block_proof` in
/// `evaporchain-node` calls this first; matches are rejected
/// with a hard error before the cryptographic verify path runs.
pub fn is_mock_prover_proof_bytes(proof_bytes: &[u8]) -> bool {
    proof_bytes.len() == MOCK_PROOF_BYTES_LEN
        && proof_bytes.iter().all(|&b| b == 0)
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

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_types::Block;

    fn block(num: u64, epoch: u64) -> Block {
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

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "ProvingEngine is the chain's IVC contract:
    /// fold_block accumulates per-block transitions, get_proof
    /// produces a CompressedProof carrying num_steps == num_folded,
    /// and verify_proof checks that the claimed num_blocks matches.
    /// `get_proof` before any fold fails closed with NoBlocksFolded;
    /// verify_proof with the wrong num_blocks rejects."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let mut prover = MockProver::new();

        // Empty prover: no proof obtainable.
        assert!(matches!(
            prover.get_proof(),
            Err(ProvingError::NoBlocksFolded)
        ));

        // Fold three blocks → proof carries num_steps=3.
        for i in 1u64..=3 {
            prover.fold_block(&block(i, i), [0; 32], [1; 32]).unwrap();
        }
        let proof = prover.get_proof().unwrap();
        assert_eq!(proof.num_steps, 3);
        assert_eq!(prover.num_blocks_folded(), 3);

        // Honest verification: claimed num_blocks matches.
        assert!(prover.verify_proof(&proof, 3, [0; 32]).unwrap());

        // Wrong claimed num_blocks rejects (no silent acceptance).
        assert!(!prover.verify_proof(&proof, 5, [0; 32]).unwrap());

        // CompressedProof.size == proof_bytes.len.
        assert_eq!(proof.size(), proof.proof_bytes.len());
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

    /// H-19 fingerprint guard — `is_mock_prover_proof` correctly
    /// identifies the all-zeros pattern that `MockProver::get_proof`
    /// emits. This is the production-side defence in depth on top
    /// of the `#[cfg]` gate: if MockProver ever leaked into a
    /// release build, its output would still be detectable on the
    /// wire and rejectable by any verifier that calls this
    /// predicate.
    #[test]
    fn is_mock_prover_proof_identifies_mock_output() {
        let mut prover = MockProver::new();
        prover
            .fold_block(&dummy_block(1, 1), [0; 32], [1; 32])
            .unwrap();
        let proof = prover.get_proof().unwrap();
        assert!(
            is_mock_prover_proof(&proof),
            "MockProver output must trip the fingerprint guard"
        );
    }

    /// H-19 fingerprint guard — well-formed non-mock proofs (here
    /// simulated by hand-crafted random-looking bytes at the right
    /// lengths) do NOT trigger the fingerprint. This is the false-
    /// positive regression guard: tightening the predicate to
    /// "all zeros + exact length" must not catch real proofs.
    #[test]
    fn is_mock_prover_proof_does_not_flag_random_bytes() {
        let proof = CompressedProof {
            proof_bytes: (0u8..32).map(|i| i.wrapping_mul(0x9E)).collect(),
            num_steps: 1,
            z0_bytes: (0u8..16).map(|i| i.wrapping_mul(0x39)).collect(),
        };
        assert!(
            !is_mock_prover_proof(&proof),
            "non-mock proof of correct shape must not match fingerprint"
        );
    }

    /// H-19 fingerprint guard — different-length proofs (what a
    /// real Nova-folded proof would produce — 100s-1000s of bytes)
    /// do not match the mock fingerprint regardless of content.
    #[test]
    fn is_mock_prover_proof_does_not_flag_long_proofs() {
        // All-zero but at lengths that don't match MockProver's
        // exact emission (32 + 16). A real Nova proof's lengths
        // wouldn't match either, even in the unlikely event of
        // all-zero output.
        let proof_long = CompressedProof {
            proof_bytes: vec![0u8; 1024],
            num_steps: 1,
            z0_bytes: vec![0u8; 16],
        };
        assert!(!is_mock_prover_proof(&proof_long));

        let proof_short = CompressedProof {
            proof_bytes: vec![0u8; 32],
            num_steps: 1,
            z0_bytes: vec![0u8; 8],
        };
        assert!(!is_mock_prover_proof(&proof_short));
    }

    /// H-19 fingerprint guard — empty proof bytes (degenerate case
    /// that wouldn't pass any real verifier anyway) is not
    /// flagged as MockProver output. The predicate is precise:
    /// only the exact MockProver shape trips it.
    #[test]
    fn is_mock_prover_proof_does_not_flag_empty_bytes() {
        let proof = CompressedProof {
            proof_bytes: vec![],
            num_steps: 0,
            z0_bytes: vec![],
        };
        assert!(!is_mock_prover_proof(&proof));
    }

    // ── Wire-layer fingerprint guard (is_mock_prover_proof_bytes) ──
    //
    // The version production verifiers actually call. Operates on
    // the raw `Vec<u8>` from `block.nova_proof` because the
    // consensus layer holds only opaque bytes (per the trait-only
    // ProofVerifier abstraction).

    /// H-19 wire guard — exactly 32 zero bytes is the MockProver
    /// signature on the wire. Locks the contract.
    #[test]
    fn is_mock_prover_proof_bytes_identifies_mock_signature() {
        let mock_bytes = vec![0u8; 32];
        assert!(is_mock_prover_proof_bytes(&mock_bytes));
    }

    /// H-19 wire guard — actual MockProver output's `proof_bytes`
    /// field round-trips through the wire predicate.
    #[test]
    fn is_mock_prover_proof_bytes_round_trips_with_mock_get_proof() {
        let mut prover = MockProver::new();
        prover
            .fold_block(&dummy_block(1, 1), [0; 32], [1; 32])
            .unwrap();
        let proof = prover.get_proof().unwrap();
        assert!(
            is_mock_prover_proof_bytes(&proof.proof_bytes),
            "MockProver proof_bytes field must trip the wire predicate"
        );
    }

    /// H-19 wire guard — non-zero bytes of correct length do NOT
    /// match. Regression guard against length-only matching that
    /// would catch real proofs of the same size.
    #[test]
    fn is_mock_prover_proof_bytes_does_not_flag_random_32_bytes() {
        let bytes: Vec<u8> = (0u8..32).map(|i| i.wrapping_mul(0x9E)).collect();
        assert!(!is_mock_prover_proof_bytes(&bytes));
    }

    /// H-19 wire guard — wrong-length all-zeros (real Nova proofs
    /// are 100s-1000s of bytes; length alone usually discriminates).
    #[test]
    fn is_mock_prover_proof_bytes_does_not_flag_wrong_lengths() {
        // Real Nova-sized all-zero is a degenerate case but still
        // not the MockProver fingerprint. Length must be exactly 32.
        assert!(!is_mock_prover_proof_bytes(&vec![0u8; 1024]));
        assert!(!is_mock_prover_proof_bytes(&vec![0u8; 31]));
        assert!(!is_mock_prover_proof_bytes(&vec![0u8; 33]));
        assert!(!is_mock_prover_proof_bytes(&[]));
    }
}
