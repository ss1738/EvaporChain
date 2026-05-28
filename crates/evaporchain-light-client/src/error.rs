//! Unified error type for the Light Client SDK.

use thiserror::Error;

/// Top-level error for any operation the SDK exposes. Wraps the
/// underlying-layer errors (BFT commit-cert verification, Nova-IVC
/// verification, Verkle state-query verification) into a single
/// type so consumers don't need to handle each layer's error
/// independently.
#[derive(Debug, Error)]
pub enum LightClientError {
    /// BFT commit-certificate verification failed (signature
    /// invalid, insufficient stake, validator-set mismatch, or
    /// trust period expired). Wraps the consensus layer's error.
    #[error("BFT light-client error: {0}")]
    Bft(String),

    /// Nova-IVC sublinear verification failed. Reasons include
    /// proof bytes corrupted, `vk_bytes` mismatched against the
    /// chain's compiled circuit, num_steps disagreement, or the
    /// chain's energy-floor policy violated.
    #[error("Nova verification failed: {0}")]
    Nova(String),

    /// Verkle state-query Merkle proof did not verify against the
    /// trusted state root from the most-recently-ingested block.
    #[error("Verkle state proof did not verify against state_root {state_root_hex}")]
    VerkleProof {
        /// Hex-encoded state root the proof was checked against.
        state_root_hex: String,
    },

    /// State query value mismatch — proof verifies but the
    /// returned value differs from the expected one. This
    /// indicates the prover gave a non-membership proof when
    /// membership was expected, or vice versa.
    #[error(
        "state value mismatch at key {key_hex}: expected {expected_hex:?}, got {actual_hex:?}"
    )]
    StateValueMismatch {
        key_hex: String,
        expected_hex: Option<String>,
        actual_hex: Option<String>,
    },

    /// The light client has no trusted state to verify against.
    /// Caller must ingest at least one block (typically genesis)
    /// before doing state queries or block-chain verification.
    #[error("no trusted state — caller must ingest a genesis block first")]
    NoTrustedState,

    /// The provided block has a height ≤ the most-recently-
    /// trusted block. Light clients track monotone height; older
    /// blocks should be ignored or treated as a separate audit
    /// query.
    #[error("block height {provided} is not greater than current trusted height {trusted}")]
    NonMonotoneHeight { provided: u64, trusted: u64 },

    /// The block's parent hash doesn't match the trusted block's
    /// hash, breaking the chain. (Only enforced for height = trusted
    /// + 1.)
    #[error(
        "block at height {height} has parent hash {parent_hash_hex} but trusted tip's hash is {trusted_hash_hex}"
    )]
    ParentHashMismatch {
        height: u64,
        parent_hash_hex: String,
        trusted_hash_hex: String,
    },

    /// Generic protocol error — unexpected state machine path.
    #[error("light-client protocol error: {0}")]
    Protocol(String),
}
