//! Inclusion proofs for `PAdicMerkleTree`.
//!
//! A proof for `key` is a vector of `depth` "siblings" — at each level,
//! the `(P − 1)` sibling subtree hashes that, together with the leaf-
//! containing subtree, hash up to the parent. The verifier walks
//! low-order base-`P` digits of `key` and re-derives the root.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::key::PAdicKey;
use crate::tree::{leaf_hash, node_hash, subtree_root, Hash, PAdicMerkleTree};

/// One level of the proof: which `digit` the path took at this level
/// (i.e. `key`'s digit at `level`), plus the `P − 1` sibling subtree
/// hashes in canonical (digit-ascending, omitting the path digit) order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofLevel {
    pub digit: u8,
    pub siblings: Vec<Hash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InclusionProof<const P: usize> {
    pub key: u64,
    pub leaf_hash: Hash,
    /// Levels from leaf-up. `levels[0]` is the level just above the leaf.
    pub levels: Vec<ProofLevel>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProofError {
    #[error("proof level count {got} does not match expected depth {expected}")]
    WrongDepth { got: u32, expected: u32 },
    #[error("proof level {level} sibling count {got} != P-1 ({expected})")]
    WrongSiblingCount { level: u32, got: usize, expected: usize },
    #[error("digit at level {level} ({got}) does not match key's digit ({expected})")]
    DigitMismatch { level: u32, got: u8, expected: u8 },
    #[error("recomputed root {got:?} != expected root {expected:?}")]
    RootMismatch { got: Hash, expected: Hash },
}

impl<const P: usize> PAdicMerkleTree<P> {
    /// Build an inclusion proof for `key`. Returns `None` if the leaf
    /// is absent.
    pub fn prove(&self, key: PAdicKey<P>) -> Option<InclusionProof<P>> {
        let leaf = self.get(key)?;
        let leaves: Vec<(u64, Hash)> = self.leaves.iter().map(|(k, v)| (*k, *v)).collect();
        let depth = self.depth();
        let mut levels = Vec::with_capacity(depth as usize);
        // Walk from the leaf upward: at each level, partition by next
        // (low-order) digit, record the sibling subtree hashes.
        for current_level in 0..depth {
            let depth_remaining = depth - current_level;
            let path_digit = key_digit::<P>(key.raw(), current_level);
            // Partition the *current bucket* (= leaves whose lower
            // `current_level` digits agree with `key`'s) by next digit.
            let bucket: Vec<(u64, Hash)> = leaves
                .iter()
                .filter(|(k, _)| key_share_low_digits::<P>(*k, key.raw(), current_level))
                .copied()
                .collect();
            let mut by_digit: Vec<Vec<(u64, Hash)>> = (0..P).map(|_| Vec::new()).collect();
            for (k, h) in &bucket {
                let d = key_digit::<P>(*k, current_level) as usize;
                by_digit[d].push((*k, *h));
            }
            let mut siblings: Vec<Hash> = Vec::with_capacity(P - 1);
            for (digit, sub) in by_digit.iter().enumerate() {
                if digit == path_digit as usize {
                    continue;
                }
                siblings.push(subtree_root::<P>(sub, depth_remaining - 1, current_level + 1));
            }
            levels.push(ProofLevel {
                digit: path_digit,
                siblings,
            });
        }
        Some(InclusionProof {
            key: key.raw(),
            leaf_hash: leaf,
            levels,
        })
    }
}

/// Stand-alone verifier — does not need the tree itself, just the
/// expected root and the proof + leaf-content the prover claims.
pub fn verify_inclusion<const P: usize>(
    expected_root: Hash,
    key: PAdicKey<P>,
    value: &[u8],
    proof: &InclusionProof<P>,
) -> Result<(), ProofError> {
    // 1. Leaf hash matches the bytes the prover claims.
    let computed_leaf = leaf_hash(key.raw(), value);
    if computed_leaf != proof.leaf_hash {
        return Err(ProofError::RootMismatch {
            got: computed_leaf,
            expected: proof.leaf_hash,
        });
    }
    // 2. Walk up: rebuild each level's parent hash from the path digit's
    //    subtree hash + the sibling hashes.
    let mut acc = computed_leaf;
    for (level, lvl) in proof.levels.iter().enumerate() {
        let level = level as u32;
        let key_dig = key_digit::<P>(key.raw(), level);
        if lvl.digit != key_dig {
            return Err(ProofError::DigitMismatch {
                level,
                got: lvl.digit,
                expected: key_dig,
            });
        }
        if lvl.siblings.len() != P - 1 {
            return Err(ProofError::WrongSiblingCount {
                level,
                got: lvl.siblings.len(),
                expected: P - 1,
            });
        }
        // Reassemble the children array, inserting `acc` at the path
        // digit and the siblings (in digit order) elsewhere.
        let mut children = [[0u8; 32]; 64];
        let mut sib_iter = lvl.siblings.iter();
        for d in 0..P {
            if d == lvl.digit as usize {
                children[d] = acc;
            } else {
                children[d] = *sib_iter.next().expect("sibling count checked above");
            }
        }
        acc = node_hash::<P>(level, &children);
    }
    // 3. Compare to the expected root.
    if acc != expected_root {
        return Err(ProofError::RootMismatch {
            got: acc,
            expected: expected_root,
        });
    }
    Ok(())
}

/// Helper: `key`'s digit at `level` (low-order first).
fn key_digit<const P: usize>(key: u64, level: u32) -> u8 {
    let p = P as u64;
    let mut x = key;
    for _ in 0..level {
        x /= p;
    }
    (x % p) as u8
}

/// Helper: do `a` and `b` agree on their first `level` low-order
/// base-`P` digits?
fn key_share_low_digits<const P: usize>(a: u64, b: u64, level: u32) -> bool {
    let p = P as u64;
    let mut a = a;
    let mut b = b;
    for _ in 0..level {
        if a % p != b % p {
            return false;
        }
        a /= p;
        b /= p;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::PAdicMerkleTree;

    #[test]
    fn prove_and_verify_round_trip_p2() {
        let mut t = PAdicMerkleTree::<2>::new(8).unwrap();
        for k in [3u64, 7, 12, 25, 41] {
            t.insert(PAdicKey::<2>::new(k), &k.to_le_bytes());
        }
        let root = t.root();
        for k in [3u64, 7, 12, 25, 41] {
            let proof = t.prove(PAdicKey::<2>::new(k)).unwrap();
            verify_inclusion::<2>(root, PAdicKey::<2>::new(k), &k.to_le_bytes(), &proof)
                .expect("proof should verify");
        }
    }

    #[test]
    fn prove_and_verify_round_trip_p3() {
        let mut t = PAdicMerkleTree::<3>::new(6).unwrap();
        for k in [1u64, 4, 9, 14, 27, 81] {
            t.insert(PAdicKey::<3>::new(k), &k.to_le_bytes());
        }
        let root = t.root();
        for k in [1u64, 4, 9, 14, 27, 81] {
            let proof = t.prove(PAdicKey::<3>::new(k)).unwrap();
            verify_inclusion::<3>(root, PAdicKey::<3>::new(k), &k.to_le_bytes(), &proof)
                .expect("proof should verify");
        }
    }

    #[test]
    fn proof_with_wrong_value_rejected() {
        let mut t = PAdicMerkleTree::<2>::new(4).unwrap();
        t.insert(PAdicKey::<2>::new(5), b"good");
        let root = t.root();
        let proof = t.prove(PAdicKey::<2>::new(5)).unwrap();
        // Verify with the wrong claimed value.
        let err = verify_inclusion::<2>(root, PAdicKey::<2>::new(5), b"bad", &proof).unwrap_err();
        assert!(matches!(err, ProofError::RootMismatch { .. }));
    }

    #[test]
    fn proof_for_absent_key_is_none() {
        let mut t = PAdicMerkleTree::<2>::new(4).unwrap();
        t.insert(PAdicKey::<2>::new(1), b"x");
        assert!(t.prove(PAdicKey::<2>::new(99)).is_none());
    }

    #[test]
    fn tampered_root_rejected() {
        let mut t = PAdicMerkleTree::<2>::new(4).unwrap();
        t.insert(PAdicKey::<2>::new(7), b"v");
        let mut bad_root = t.root();
        bad_root[0] ^= 0xFF;
        let proof = t.prove(PAdicKey::<2>::new(7)).unwrap();
        let err = verify_inclusion::<2>(bad_root, PAdicKey::<2>::new(7), b"v", &proof).unwrap_err();
        assert!(matches!(err, ProofError::RootMismatch { .. }));
    }
}
