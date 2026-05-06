# Energy-Verkle Trie — Formal Proof Companion

**Companion to** `research/frontier/02-energy-verkle-trie.md` (the design rationale) and `research/tla/EnergyVerkleTrie.tla` (the TLA+ specification).

**Author:** Satyawan Singh
**Date:** 2026-04-27
**Status:** v0.1 — TLA+ spec drafted, TLC runs pending. Algebraic-commitment proof (compress + decompress preserves Pedersen commitment) is open work and out of scope for TLC.

---

## 1. The theorem we are formalising

The frontier doc proposes an Energy-Annotated Verkle Trie where cold subtrees (max_energy=0, leaf_count>0) are *compressed* into a single `Compressed` node, and where *decompression* happens implicitly when something is inserted into a path that traverses a Compressed node. The doc names DECOMPRESSION as the hardest part: when a ghost under a compressed subtree gets resurrected, the trie must:

1. Allow the new leaf to be inserted "above" the compressed boundary
2. Preserve the count of dead leaves the compressed node carried
3. Maintain a commitment that's consistent with the original (uncompressed) subtree shape

The TLA+ spec (`EnergyVerkleTrie.tla`) verifies the **state-machine** invariants of this lifecycle. The **algebraic-commitment** invariant (Pedersen-commitment equivalence) is sketched in §5 and is open work.

---

## 2. Properties verified by TLC (state-machine)

The spec verifies four invariants that hold across all reachable states of the bounded model:

| Property | Statement | TLA+ name |
|---|---|---|
| Type safety | All variables stay in declared domains | `TypeOK` |
| Cold-subtree gate | Only cold subtrees (max_energy=0, leaf_count>0) can transition to Compressed | `NoHotCompressed` |
| Leaf-count preservation | Compressed subtrees record the original leaf count exactly | `CompressionPreservesLeafCount` |
| Energy monotonicity | Per-leaf energy never exceeds its initial value (decay-only without refresh) | `EnergyMonotonicityRespected` |

These map to specific lines in `crates/evaporchain-crypto/src/energy_verkle.rs`:

- The cold-subtree gate corresponds to the `is_cold()` check at line 109 (`max_energy == 0 && leaf_count > 0`) and the compression guard.
- Leaf-count preservation corresponds to `CompressedNode.leaf_count` carrying the original count from `EnergyMeta.leaf_count`.
- Energy monotonicity corresponds to the bit-shift integer decay formula in `evaporation.rs:energy_at_epoch` (which is monotonically non-increasing for fixed initial energy and half-life).

If any of these invariants fails under TLC's bounded check, the implementation has a bug.

---

## 3. What the spec abstracts away

For TLC tractability, the model deliberately abstracts:

- **Tree shape.** The 256-ary BTreeMap of internal nodes is collapsed into a flat set of "subtrees", each with `(leaf_count, state, max_energy)`. The actual production trie has a hierarchical Pedersen structure; this spec ignores that.
- **Decay formula.** Production uses an integer bit-shift exponential (`energy_at_epoch` in `evaporation.rs`). The spec uses linear `DecayPerEpoch`. Both are monotonically non-increasing in elapsed time for fixed `InitialEnergy`/`half_life`, which is what the invariants depend on.
- **Pedersen commitments.** The spec doesn't model the algebraic commitment scheme at all. It models leaf existence and leaf count, which is what the lifecycle invariants need; the commitment-preservation argument is separate (§5).
- **Multi-leaf paths.** Leaves are flat-mapped to subtrees in this spec. Production has multi-byte path indices into a 256-ary tree.

These abstractions are appropriate for the property class verified. They would be **wrong** for verifying e.g. proof-soundness (which depends on the algebraic structure).

---

## 4. The decompression hardness

The frontier doc identifies decompression as the hard problem. The spec models it via the action `DecompressOnInsert(s, newLeaf)`:

```tla
DecompressOnInsert(s, newLeaf) ==
    /\ subtree_state[s] = "Compressed"
    /\ newLeaf \in Leaves /\ LeafSubtree[newLeaf] = s
    /\ subtree_state' = [subtree_state EXCEPT ![s] = "Active"]
    /\ subtree_leaf_count' = [subtree_leaf_count EXCEPT ![s] = subtree_leaf_count[s] + 1]
    /\ leaf_energy' = [leaf_energy EXCEPT ![newLeaf] = InitialEnergy[newLeaf]]
    /\ UNCHANGED <<epoch>>
```

This corresponds to `energy_verkle.rs:352-355` — the implementation's branch where inserting into a Compressed node creates a new internal node with the compressed node as one child:

```rust
EnergyNode::Compressed(_) => {
    // Inserting into a compressed region = decompression.
    // We can't expand the original subtree (it's gone), so we create
    // a new internal node with the compressed node as one child and ...
}
```

The spec's claim: as long as `subtree_leaf_count` is preserved and the new leaf is a fresh active leaf, the post-decompression state is consistent. TLC verifies this invariant holds across all reachable schedules.

The key insight: **the original subtree's contents are not recoverable from the Compressed node.** The compressed node carries only a commitment hash and a leaf count. So decompression doesn't restore old leaves; it adds a new active leaf "alongside" the dead history. This is the correct behaviour — the frontier doc's design explicitly accepts that Compressed nodes are forward-only (compress-then-resurrect adds a new leaf, not the original).

---

## 5. The open algebraic theorem

The state-machine spec verifies that the lifecycle is correct. It does **not** verify that the Pedersen commitment is consistent across compress / decompress. That's a separate algebraic theorem:

> **Theorem (commitment equivalence, open):** Let `T` be an active subtree with leaves `L = {l_1, ..., l_n}` and Pedersen commitment `C(T)`. Let `T_compressed` be the result of compressing all leaves in `L` (all energy = 0). Then:
>
>     C(T_compressed) = H(C(T) || leaf_count(T) || last_activity)
>
> where `H` is the canonical hash used for compressed-node commitment. Furthermore, after `DecompressOnInsert(T_compressed, l_new)` produces an internal node with the compressed node as one child and `l_new` as another:
>
>     C(internal_node) = pedersen_combine(C(T_compressed), C(leaf(l_new)))

This is what makes light-client proofs work across compression: a proof against `C(internal_node)` can verify either (a) inclusion of `l_new` (via the new-leaf branch), (b) non-existence of any specific leaf in `L` via the compressed branch (the leaf was compressed-out; non-membership proof against `C(T_compressed)`).

**This theorem is NOT verified by TLA+.** It requires algebraic reasoning about Pedersen commitments and is more naturally proved in Coq, Lean, or by hand. Estimated effort: 2-3 weeks of focused work for a researcher with prior commitment-scheme proof experience.

---

## 6. What's still open

Beyond the algebraic theorem (§5), the following items round out the formal-verification track for the Energy-Verkle Trie:

### 6.1 TLC at larger scale

The default `.cfg` uses 2 subtrees, 3 leaves, 12 epochs. TLC can verify these invariants in seconds. Extending to ~5 subtrees and ~10 leaves is feasible but requires a few minutes; bigger configurations require state-space pruning. The bounded coverage is sufficient to catch logic bugs in the lifecycle but doesn't prove the invariants for arbitrarily-sized tries.

### 6.2 Coq / Lean mechanization

Mechanized proof of:

- The state-machine invariants (Type, NoHotCompressed, etc.) for unbounded models
- The commitment-equivalence theorem (§5)
- The integer-arithmetic decay function's monotonicity (parallels the work for Rule-Based Consensus in `RuleBasedConsensus.tla`'s proof companion §4)

### 6.3 Composition with Rule-Based Consensus

The Energy-Verkle Trie's commitments feed into the Rule-Based Consensus anchor scheme. A combined spec proving "consensus-on-anchor + Energy-Verkle commitment correctness ⇒ deterministic state queries even after compression" is the natural follow-up.

---

## 7. How an auditor should read this

For an external audit firm engaging with EvaporChain on the Energy-Verkle Trie:

1. Read `crates/evaporchain-crypto/src/energy_verkle.rs` (the implementation).
2. Read `research/frontier/02-energy-verkle-trie.md` (the design rationale).
3. Read this proof companion + `EnergyVerkleTrie.tla` (the formal model).
4. Run TLC on `EnergyVerkleTrie.cfg` and confirm all four PASS-marked invariants pass.
5. Note that the algebraic commitment theorem (§5) is **not** TLC-checked — it's an open Coq/Lean target. The audit should explicitly scope whether commitment correctness is in or out of scope.

---

## 8. References

- `research/frontier/02-energy-verkle-trie.md` — design rationale
- `research/tla/EnergyVerkleTrie.tla` — formal spec
- `research/tla/EnergyVerkleTrie.cfg` — TLC configuration
- `research/tla/RuleBasedConsensus.tla` — sister formal spec (similar shape)
- `research/frontier/03-rule-based-consensus-proof.md` — sister proof companion
- `crates/evaporchain-crypto/src/energy_verkle.rs` — Rust implementation
- Lamport, L. *Specifying Systems*. Addison-Wesley, 2002.
- Boneh, D., Bunz, B., Fisch, B. *Batching Techniques for Accumulators with Applications to IOPs and Stateless Blockchains*. CRYPTO 2019. (Pedersen accumulator background.)

---

**End of v0.1.**

Note for revision: §5's Pedersen-commitment equivalence theorem statement is illustrative — the real proof would need a formal commitment-scheme model. Treat as a target-shape, not a final formulation. Engaged auditor should refine.
