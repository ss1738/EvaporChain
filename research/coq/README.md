# EvaporChain — Coq mechanization

Companion to `research/tla/` and `research/frontier/`. TLA+ specs the
state-machine logic; the Coq proofs here mechanize the integer-arithmetic
properties the Rust implementation actually computes.

**Why this exists.** Several theorems in the EvaporChain papers are stated
over the reals — `energy_at_epoch(t) = E₀ · 2^(-t/h)` is a continuous
exponential. The production code uses `u64` integers with saturating
arithmetic and bit-shift halvings. The continuous proofs do not directly
imply the integer proofs: integer truncation can introduce non-monotonic
artefacts at half-life boundaries unless the implementation is careful.
These mechanizations close the gap.

## Files

| File | Punch-list item | Status |
|---|---|---|
| `EnergyDecayMonotonicity.v` | #7 — integer-decay monotonicity over `nat` | spec + monotonicity theorem stated; within-halving case proved at `Qed.`; cross-halving case `Admitted` pending one arithmetic-bound lemma |
| `EnergyVerkleCompression.v` | #8 — Energy-Verkle compression invariants | leaf-count preservation `Qed.`; energy-sum monotonicity `Qed.`; commitment preservation as named axiom (BLS12-381 dependency) |
| `PoHAFreeloading.v` | #9 — DA freeloading-resistance theorem | threat model + theorem statement + reduction to blake3 + BFT axioms; final transitivity `Admitted` pending `Q`-modeled `negligible` |
| `LazyEagerEquivalence.v` | #10 — Rule-Based Consensus `lazy ≡ eager` | `eager_eq_lazy` proven at `Qed.` relative to `decay_step_compose` axiom; integer-drift bound for the real impl tracked as follow-up |
| `../proofs/LLSAInvariantPreservation.v` | LLSA invariant preservation (§A1.2 T4 descope path) | All 4 lemmas at `Qed.` — `redirect_preserves_inv`, `decay_preserves_inv`, `block_produce_preserves_inv`, `llsa_conservation_invariant_preservation`. Build-verified clean under Rocq 9.1.1 (M2 closure 2026-05-05). |

## Building

```sh
cd research/coq
make           # check all *.v files
make clean     # remove generated artefacts
```

Verified clean under **Rocq 9.1.1** (the renamed Coq, available via
`brew install coq`). Older Coq 8.18+ should also work for these files
modulo the Coq 9.0 transition fixes documented in the M2 punch-list
entry (dropped `Coq.Arith.Div2` import, brace-focus instead of bullets
between splits, direct `Nat.le_0_l`/`Nat.le_refl` in place of `lia` on
trivial goals, `eapply`/`eassumption` for evar inference).

## What "machine-checked" means here

A `Qed.` line at the end of a `Theorem` is the gold standard — Coq has
verified the proof against its kernel. An `Admitted.` line accepts the
theorem as an axiom; this is used for proofs whose remaining obligations
have been reduced to standard arithmetic facts but which haven't been
fully discharged yet.

The aim of this directory is to drive every theorem to `Qed.` before
mainnet. As of 2026-05-05 the LLSA invariant-preservation file
(`../proofs/LLSAInvariantPreservation.v`) is fully `Qed.` end-to-end
and the descope-path Layer 7 LLSA claim is therefore build-verifiable.
`EnergyDecayMonotonicity` cross-halving case + `PoHAFreeloading`
transitivity remain the open `Admitted.` items.

## Cross-references

- `crates/evaporchain-types/src/lib.rs:1331` — `energy_at_epoch` Rust
  implementation. Any change to the Rust definition must be reflected
  in `EnergyDecayMonotonicity.v` and re-checked.
- `research/papers/paper_2_state_economics.md` — continuous-domain
  derivation of the decay law that motivates the integer impl.
- `research/tla/EnergyVerkleTrie.tla` — state-machine spec that uses the
  decay function as an opaque oracle; this Coq file closes that gap.
