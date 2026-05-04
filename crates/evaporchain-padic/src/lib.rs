//! p-adic ultrametric Merkle commitment for EvaporChain.
//!
//! Per `research/INVENTION_STACK.md` §A1.4 (Amendment 1, far-frontier math
//! that survived the L1 shipping filter):
//!
//! > p-adic valuation `v_p(x)` = energy level. Ultrametric balls form a
//! > *strict* tree — perfect Merkle-native geometry. Distinctive,
//! > low-risk, ship-now. No other chain has p-adic state metrics.
//!
//! ## Why this is novel at L1
//!
//! Standard Merkle Patricia tries are *radix* tries: keys split by their
//! high-order bits/nibbles. The p-adic ultrametric tree splits keys by
//! their *low-order* base-`p` digits, which lines up with the algebraic
//! p-adic valuation `v_p(x − y)`. Two keys lie in the same depth-`d`
//! ultrametric ball iff they agree on their first `d` low-order base-`p`
//! digits, iff `v_p(x − y) ≥ d`.
//!
//! This is not a cosmetic re-labelling: the *strong triangle inequality*
//! `d(x, z) ≤ max(d(x, y), d(y, z))` makes ultrametric balls **either
//! disjoint or strictly nested** (Hughes 2004 — every ultrametric space
//! embeds in a tree). That property is what makes the Merkle tree
//! *automatic* — there is no choice of topology, the metric defines it.
//!
//! ## Energy semantics
//!
//! `valuation::<P>(key)` is the energy-level index for that key — a key
//! divisible by `P^k` lives in a depth-`k` ball, has fewer neighbours
//! sharing its low digits, and is treated as colder/more isolated by
//! later EvaporChain primitives that consume the kernel's λ.
//!
//! ## Module map
//!
//! - [`valuation`] — pure p-adic valuation `v_p(n)`.
//! - [`metric`] — the ultrametric distance `v_p(|x − y|)`, plus the
//!   strong triangle inequality (proven by `proptest`).
//! - [`key`] — `PAdicKey<P>` newtype + base-`p` digit decomposition.
//! - [`tree`] — fixed-depth `PAdicMerkleTree<P>` (sparse, blake3-hashed).
//! - [`proof`] — inclusion proof + verifier.

pub mod key;
pub mod metric;
pub mod proof;
pub mod tree;
pub mod valuation;

pub use key::PAdicKey;
pub use metric::ultrametric_distance;
pub use proof::{verify_inclusion, InclusionProof, ProofError};
pub use tree::{Hash, PAdicMerkleTree, TreeError};
pub use valuation::valuation;

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "p-adic ultrametric Merkle commitment satisfies
    /// the STRONG triangle inequality `d(x,z) ≤ max(d(x,y), d(y,z))`,
    /// which makes ultrametric balls strictly nested or disjoint —
    /// the Merkle tree is automatic. Inclusion proofs verify against
    /// the canonical root; tampering with the value or path rejects."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Strong triangle inequality (ultrametric).
        let x = 12u64;
        let y = 28u64;
        let z = 36u64;
        let d_xy = ultrametric_distance::<2>(x, y);
        let d_yz = ultrametric_distance::<2>(y, z);
        let d_xz = ultrametric_distance::<2>(x, z);
        // d_xz ≤ max(d_xy, d_yz)  — but ultrametric distance here is
        // larger-when-closer (it's the valuation), so check the
        // "smaller-of-three" form: the smallest two distances are equal.
        // (Equivalently: max(d_xy, d_yz) ≥ d_xz once we flip sign
        // conventions; here we use the canonical strong-form check.)
        let mut ds = [d_xy, d_yz, d_xz];
        ds.sort();
        assert_eq!(ds[0], ds[1], "ultrametric: smallest two distances must be equal");

        // Build a tree, insert keys, prove inclusion.
        let mut tree = PAdicMerkleTree::<2>::new(6).unwrap();
        let k1 = PAdicKey::<2>::new(0xAB);
        let k2 = PAdicKey::<2>::new(0xCD);
        tree.insert(k1, b"hello");
        tree.insert(k2, b"world");
        let root = tree.root();
        assert_ne!(root, [0u8; 32], "non-empty tree must have non-zero root");

        // Honest proof verifies.
        let p1 = tree.prove(k1).unwrap();
        verify_inclusion::<2>(root, k1, b"hello", &p1).unwrap();
        // Tampered value fails verification.
        assert!(verify_inclusion::<2>(root, k1, b"WRONG", &p1).is_err());

        // Querying a missing key returns None (not Some(zero)).
        let missing = PAdicKey::<2>::new(0xFF);
        assert!(tree.get(missing).is_none());

        // Two distinct trees with different keys produce different roots.
        let mut tree2 = PAdicMerkleTree::<2>::new(6).unwrap();
        tree2.insert(k1, b"DIFFERENT");
        assert_ne!(tree2.root(), root);

        // valuation: v_p(0) is the depth (or u32::MAX). v_p(P) = 1
        // for P=2, since 2 = 2^1 with quotient 1.
        assert_eq!(valuation::<2>(2), 1);
        assert_eq!(valuation::<2>(8), 3); // 2^3
    }
}
