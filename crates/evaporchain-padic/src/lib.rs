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
