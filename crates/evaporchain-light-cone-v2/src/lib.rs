//! Light-Cone V2 — causal-cone Merkle proofs.
//!
//! V1 (`evaporchain-light-cone`) ships `causal_past(lc, block_id)`
//! as a reachability set computed by BFS over parent edges. Correct
//! math, but every verifier needs the full DAG to evaluate it.
//!
//! V2 closes that gap with a Merkle commitment over the sorted
//! ancestor list:
//!
//! 1. `causal_root(lc, block_id)` — BLAKE3 Merkle root over the
//!    canonical (BTreeSet-sorted) `causal_past(lc, block_id)`.
//!    Empty causal past → sentinel "empty cone" root. Domain-separated
//!    leaf and inner tags.
//!
//! 2. `prove_ancestry(lc, descendant, ancestor)` — produces a
//!    `MerklePath` from `ancestor`'s leaf to `causal_root(descendant)`,
//!    or `None` if `ancestor` is not in the causal past.
//!
//! 3. `verify_ancestry(causal_root, ancestor_id, proof)` — pure
//!    function of (32-byte root, 32-byte candidate id, proof bytes).
//!    Verifier never touches the DAG.
//!
//! ## Holographic reduction
//!
//! V2 is the substrate for **light-client ancestry queries**: a
//! light client holding only the latest block's `causal_root` can
//! verify any ancestry claim a full node sends in O(log N) hashes.
//! Same shape as the Merkle-Patricia state proofs Ethereum uses for
//! account inclusion, but over the partial-order DAG instead of a
//! contiguous trie.

pub mod merkle;
pub mod proof;

pub use merkle::{causal_root, EMPTY_CAUSAL_ROOT};
pub use proof::{prove_ancestry, verify_ancestry, AncestryError, MerklePath};
