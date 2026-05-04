//! `MnemoTriePtr` — opaque pointer to off-chain card content.
//!
//! Card content (front/back text, audio, image) is heavy and would
//! bloat the chain. We commit only a 32-byte BLAKE3 hash of the
//! canonical encoding plus the byte length. The off-chain blob layer
//! (likely IPFS or a CDN) serves the actual bytes; clients verify
//! against the hash.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TrieError {
    #[error("content_hash must not be all-zero")]
    EmptyHash,
    #[error("content_len must be > 0")]
    ZeroLen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MnemoTriePtr {
    pub content_hash: [u8; 32],
    pub content_len: u64,
}

impl MnemoTriePtr {
    pub fn new(content_hash: [u8; 32], content_len: u64) -> Result<Self, TrieError> {
        if content_hash == [0u8; 32] {
            return Err(TrieError::EmptyHash);
        }
        if content_len == 0 {
            return Err(TrieError::ZeroLen);
        }
        Ok(Self {
            content_hash,
            content_len,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_hash() {
        assert_eq!(
            MnemoTriePtr::new([0; 32], 64).unwrap_err(),
            TrieError::EmptyHash
        );
    }

    #[test]
    fn rejects_zero_len() {
        assert_eq!(
            MnemoTriePtr::new([0xAB; 32], 0).unwrap_err(),
            TrieError::ZeroLen
        );
    }

    #[test]
    fn valid_constructor_works() {
        let p = MnemoTriePtr::new([0xCD; 32], 1024).unwrap();
        assert_eq!(p.content_len, 1024);
    }

    #[test]
    fn round_trip_serde() {
        let p = MnemoTriePtr::new([0x77; 32], 256).unwrap();
        let s = serde_json::to_string(&p).unwrap();
        let back: MnemoTriePtr = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
