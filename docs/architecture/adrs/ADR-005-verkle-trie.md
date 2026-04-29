# ADR-005: Verkle Trie as State Commitment Structure

**Status:** Accepted  
**Date:** 2026-02-18  
**Deciders:** Satyawan Singh (founder)

---

## Context

The state commitment structure must support:

1. **Compact proofs** — light clients need to verify individual object state without downloading the full trie.
2. **Evaporation proofs** — when an object evaporates, the chain must produce a proof that the object is *absent* (non-membership proof).
3. **Stateless validation** — validators in the long run should be able to validate blocks without storing the full state, given witnesses supplied by the proposer.

The Merkle Patricia Trie (MPT, used by Ethereum) produces proofs of size O(log n * branch_factor) ≈ several KB per proof. With 10M+ objects on a mature chain, MPT proofs become expensive to produce and transmit.

## Decision

Use a Verkle Trie as the primary state commitment structure. Verkle tries replace hash-based internal nodes with polynomial commitments (KZG or Pedersen), producing constant-size membership and non-membership proofs regardless of trie depth.

The implementation is in `crates/evaporchain-state` and uses a Pedersen-based construction over the BLS12-381 scalar field for alignment with the existing BLS crypto stack.

## Alternatives considered

| Structure | Why not chosen |
|-----------|---------------|
| Merkle Patricia Trie | O(log n) proof size; proof sizes grow with state; no native non-membership proofs |
| Plain Merkle Tree | Simpler but same O(log n) proof growth; MPT is strictly better for key-value storage |
| Sparse Merkle Tree (SMT) | Supports non-membership proofs; but proof size still O(log n) at 256-bit key space |
| IAVL (Cosmos) | Same O(log n) weakness; doesn't support witness-based stateless validation |

## Consequences

- Verkle proof generation and verification are more compute-intensive than Merkle hashing per node, but require far fewer nodes to traverse.
- The `energy-verkle` hybrid construction (Energy-Verkle Trie, one of the 5 novel primitives) extends Verkle tries to include energy metadata in the commitment, enabling evaporation proofs without a separate data structure.
- Light clients can sample object liveness with O(1) proof size, independent of total state size.
- The state sync protocol (`evaporchain-state/src/sync.rs`) transmits blocks with embedded Verkle witnesses, enabling stateless validation by the receiving node.
