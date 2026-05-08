//! Cross-side constants. Must match Solidity `BridgeConstants.sol`
//! byte-for-byte. The Solidity values are `keccak256(tag)` where `tag`
//! is the literal ASCII shown below.
//!
//! Round-trip is checked by `tests/cross_side_agreement.rs` — if Solidity
//! changes a tag, the Rust test fails.

use sha3::{Digest, Keccak256};

/// Domain-separation tag for BLS commit-certificate hashing.
pub const DOMAIN_TAG_COMMIT_TEXT: &str = "EvaporChain/v1/commit-cert";

/// Domain-separation tag for Verkle root commitments.
pub const DOMAIN_TAG_VERKLE_ROOT_TEXT: &str = "EvaporChain/v1/verkle-root";

/// Domain-separation tag for ghost-record commitments.
pub const DOMAIN_TAG_GHOST_RECORD_TEXT: &str = "EvaporChain/v1/ghost-record";

/// Domain-separation tag for finalised block-header commitments.
pub const DOMAIN_TAG_HEADER_TEXT: &str = "EvaporChain/v1/header";

/// Domain-separation tag for state-membership attestations (Phase 4 MVP).
pub const DOMAIN_TAG_STATE_MEMBERSHIP_TEXT: &str = "EvaporChain/v1/state-membership";

/// Maximum validator-set size accepted on-chain. Must match
/// `BridgeConstants.MAX_VALIDATORS` on the Solidity side.
pub const MAX_VALIDATORS: usize = 1024;

/// BFT 2/3+ stake-threshold numerator. Quorum needs `stake * STAKE_DEN > total * STAKE_NUM`.
pub const STAKE_NUM: u64 = 2;
/// BFT 2/3+ stake-threshold denominator. See [`STAKE_NUM`].
pub const STAKE_DEN: u64 = 3;

/// `keccak256(tag)` — matches `bytes32` constants on the Solidity side.
pub fn domain_tag(tag: &str) -> [u8; 32] {
    let mut h = Keccak256::new();
    h.update(tag.as_bytes());
    let out = h.finalize();
    let mut a = [0u8; 32];
    a.copy_from_slice(&out);
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_distinct() {
        let a = domain_tag(DOMAIN_TAG_COMMIT_TEXT);
        let b = domain_tag(DOMAIN_TAG_VERKLE_ROOT_TEXT);
        let c = domain_tag(DOMAIN_TAG_GHOST_RECORD_TEXT);
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
    }

    #[test]
    fn tags_deterministic() {
        let a1 = domain_tag(DOMAIN_TAG_COMMIT_TEXT);
        let a2 = domain_tag(DOMAIN_TAG_COMMIT_TEXT);
        assert_eq!(a1, a2);
    }
}
