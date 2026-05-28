//! §Proving — IVC accumulator e2e
//!
//! Scenario: "EvaporChain light-client proof arc" — a light client
//! LEILA receives a CompressedProof and a ChainProof from a block
//! producer. She checks the proof type, measures its size, verifies
//! the compression ratio, and chains two range proofs together.
//! The H-19 fingerprint guard is confirmed: any all-zeros proof is
//! detectable before the cryptographic path runs, preventing
//! MockProver leakage in production.
//!
//! The suite proves: `is_mock_prover_proof` correctly detects the
//! all-zeros fingerprint and passes non-zero proofs; the bytes-level
//! guard mirrors the struct-level guard; CompressedProof and ChainProof
//! are serde-round-trippable; ProofChain segment management works;
//! ProvingError variants are all representable.

use evaporchain_proving::chain_proof::{ChainProof, ProofChain, ProofSegment};
use evaporchain_proving::{
    is_mock_prover_proof, is_mock_prover_proof_bytes, CompressedProof, ProvingError,
};

fn mock_compressed(num_steps: usize) -> CompressedProof {
    CompressedProof {
        proof_bytes: vec![0u8; 32],
        num_steps,
        z0_bytes: vec![0u8; 16],
    }
}

fn real_like_compressed(num_steps: usize) -> CompressedProof {
    CompressedProof {
        proof_bytes: (0u8..32).collect(),
        num_steps,
        z0_bytes: (0u8..16).collect(),
    }
}

fn segment(start: u64, end: u64, start_root: [u8; 32], end_root: [u8; 32]) -> ProofSegment {
    let steps = (end - start + 1) as usize;
    ProofSegment {
        proof: real_like_compressed(steps),
        start_height: start,
        end_height: end,
        start_state_root: start_root,
        end_state_root: end_root,
        num_steps: steps,
    }
}

fn chain_proof_struct(blocks: u64, state_root: [u8; 32]) -> ChainProof {
    let proof = real_like_compressed(blocks as usize);
    let size = proof.proof_bytes.len();
    ChainProof {
        proof,
        genesis_state_root: [0u8; 32],
        final_state_root: state_root,
        block_height: blocks,
        final_epoch: blocks * 10,
        created_at: 1_716_000_000,
        proof_size_bytes: size,
        num_steps: blocks as usize,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn mock_proof_fingerprint_detected() {
    // H-19: all-zeros proof_bytes of length 32 = MockProver fingerprint.
    let mock = mock_compressed(3);
    assert!(
        is_mock_prover_proof(&mock),
        "all-zeros 32-byte proof must be detected as MockProver"
    );
}

#[test]
fn real_like_proof_not_flagged() {
    let real_like = real_like_compressed(5);
    assert!(
        !is_mock_prover_proof(&real_like),
        "non-zero proof must not be flagged as mock"
    );
}

#[test]
fn mock_proof_bytes_wire_layer_detected() {
    // Wire-layer check: raw proof_bytes from the chain block field.
    assert!(is_mock_prover_proof_bytes(&vec![0u8; 32]));
    // Wrong length: not the fingerprint.
    assert!(!is_mock_prover_proof_bytes(&vec![0u8; 64]));
    assert!(!is_mock_prover_proof_bytes(&vec![0u8; 1]));
    // Non-zero bytes: not the fingerprint.
    let mut almost = vec![0u8; 32];
    almost[31] = 1;
    assert!(!is_mock_prover_proof_bytes(&almost));
}

#[test]
fn empty_proof_bytes_not_flagged_as_mock() {
    // An empty proof is not the MockProver fingerprint (length mismatch).
    assert!(!is_mock_prover_proof_bytes(&[]));
    let empty = CompressedProof {
        proof_bytes: vec![],
        num_steps: 0,
        z0_bytes: vec![],
    };
    assert!(!is_mock_prover_proof(&empty));
}

#[test]
fn compressed_proof_size_equals_bytes_len() {
    let p = real_like_compressed(7);
    assert_eq!(p.size(), p.proof_bytes.len());
}

#[test]
fn compressed_proof_serde_round_trip() {
    let proof = CompressedProof {
        proof_bytes: vec![0x42; 32],
        num_steps: 11,
        z0_bytes: vec![0x11; 16],
    };
    let json = serde_json::to_string(&proof).unwrap();
    let back: CompressedProof = serde_json::from_str(&json).unwrap();
    assert_eq!(back.num_steps, 11);
    assert_eq!(back.proof_bytes, proof.proof_bytes);
    assert_eq!(back.z0_bytes, proof.z0_bytes);
}

#[test]
fn chain_proof_size_and_ratio() {
    let cp = chain_proof_struct(1_000, [0xAA; 32]);
    assert_eq!(cp.size(), cp.proof_size_bytes);
    let ratio = cp.compression_ratio();
    assert!(
        ratio > 1.0,
        "compression_ratio must be > 1.0 for 1000 blocks in 32 bytes"
    );
}

#[test]
fn chain_proof_zero_size_ratio_is_zero() {
    let mut cp = chain_proof_struct(100, [0xBB; 32]);
    cp.proof_size_bytes = 0;
    assert_eq!(cp.compression_ratio(), 0.0);
}

#[test]
fn chain_proof_serde_round_trip() {
    let cp = chain_proof_struct(500, [0xCC; 32]);
    let json = serde_json::to_string(&cp).unwrap();
    let back: ChainProof = serde_json::from_str(&json).unwrap();
    assert_eq!(back.block_height, 500);
    assert_eq!(back.final_state_root, [0xCC; 32]);
    assert_eq!(back.num_steps, 500);
}

#[test]
fn proof_chain_empty_has_no_range() {
    let pc = ProofChain::new();
    assert!(
        pc.block_range().is_none(),
        "empty ProofChain must have no range"
    );
    assert_eq!(pc.total_blocks(), 0);
    assert_eq!(pc.num_segments(), 0);
}

#[test]
fn proof_chain_add_segment_extends_range() {
    let mut pc = ProofChain::new();
    pc.add_segment(segment(0, 999, [0x00; 32], [0xAA; 32]))
        .unwrap();
    let (lo, hi) = pc.block_range().unwrap();
    assert_eq!(lo, 0);
    assert_eq!(hi, 999);
    assert_eq!(pc.total_blocks(), 1_000);
    assert_eq!(pc.num_segments(), 1);
}

#[test]
fn proof_chain_two_segments_link() {
    let mut pc = ProofChain::new();
    pc.add_segment(segment(0, 999, [0x00; 32], [0xAA; 32]))
        .unwrap();
    pc.add_segment(segment(1_000, 1_999, [0xAA; 32], [0xBB; 32]))
        .unwrap();
    let (lo, hi) = pc.block_range().unwrap();
    assert_eq!(lo, 0);
    assert_eq!(hi, 1_999);
    assert_eq!(pc.total_blocks(), 2_000);
    assert_eq!(pc.num_segments(), 2);
}

#[test]
fn proving_error_no_blocks_folded_is_representable() {
    let err = ProvingError::NoBlocksFolded;
    assert!(!err.to_string().is_empty());
}

#[test]
fn proving_error_nova_not_available_is_representable() {
    let err = ProvingError::NovaNotAvailable;
    assert!(
        err.to_string().contains("nova")
            || err.to_string().contains("Nova")
            || err.to_string().contains("feature")
    );
}

#[test]
fn leila_light_client_full_arc() {
    // Full arc: LEILA receives a ChainProof for 5_000 blocks. She checks
    // it's not a MockProver proof (H-19), verifies size and genesis/final
    // roots, chains it with a range proof. IGOR's combined proof covers
    // the full chain.

    let genesis_root = [0x00; 32];
    let mid_root = [0x50; 32];
    let final_root = [0xFF; 32];

    // First half: blocks 0–4_999.
    let first_half = ChainProof {
        proof: real_like_compressed(5_000),
        genesis_state_root: genesis_root,
        final_state_root: mid_root,
        block_height: 4_999,
        final_epoch: 49_990,
        created_at: 1_716_000_000,
        proof_size_bytes: 32,
        num_steps: 5_000,
    };

    // Second half: blocks 5_000–9_999.
    let second_half = ChainProof {
        proof: real_like_compressed(5_000),
        genesis_state_root: mid_root,
        final_state_root: final_root,
        block_height: 9_999,
        final_epoch: 99_990,
        created_at: 1_716_000_100,
        proof_size_bytes: 32,
        num_steps: 5_000,
    };

    // H-19: neither proof is a MockProver proof.
    assert!(!is_mock_prover_proof(&first_half.proof));
    assert!(!is_mock_prover_proof(&second_half.proof));

    // Chain the two proofs (using the CompressedProof inside each ChainProof).
    let mut pc = ProofChain::new();
    pc.add_segment(ProofSegment {
        proof: first_half.proof,
        start_height: 0,
        end_height: 4_999,
        start_state_root: genesis_root,
        end_state_root: mid_root,
        num_steps: 5_000,
    })
    .unwrap();
    pc.add_segment(ProofSegment {
        proof: second_half.proof,
        start_height: 5_000,
        end_height: 9_999,
        start_state_root: mid_root,
        end_state_root: final_root,
        num_steps: 5_000,
    })
    .unwrap();

    assert_eq!(pc.total_blocks(), 10_000);
    let (lo, hi) = pc.block_range().unwrap();
    assert_eq!(lo, 0);
    assert_eq!(hi, 9_999);

    // Final state root is accessible.
    let final_sr = pc.final_state_root().unwrap();
    assert_eq!(final_sr, final_root);
}
