# Rule-Based Consensus — Formal Proof Companion

**Companion to** `research/frontier/03-rule-based-consensus.md` (the design rationale) and `research/tla/RuleBasedConsensus.tla` (the TLA+ specification).

**Author:** Satyawan Singh
**Date:** 2026-04-27
**Status:** v0.1 — TLA+ spec drafted, TLC runs pending. Mechanized proof (Coq / Lean) is open work.

---

## 1. The theorem we are formalising

The frontier document proposes that EvaporChain's consensus commits to a state *function* (anchor + decay rules), not to a per-block state root. The headline correctness claim is paraphrased there as:

> Two validators applying the same decay formula to the same anchor state at the same query epoch MUST get the same answer.

This document treats that claim with care. It is **not exactly true** as stated for the integer-arithmetic implementation that EvaporChain uses, and the correct statement is more subtle. The TLA+ spec makes the subtlety explicit.

The corrected theorem, as encoded in `RuleBasedConsensus.tla`:

> **Query Determinism:** for any two validators $v_1, v_2$ and any query $(\text{obj}, q_{\text{epoch}})$ with $q_{\text{epoch}} \geq \text{anchor\_epoch}$, the result of `LazyEnergy(anchor\_energy[obj], HalfLife[obj], q_{\text{epoch}} − \text{anchor\_epoch})` is identical on every validator that performs the query.

This is a property of the *function*, not of the system. It is true because `LazyEnergy` is a pure function of three deterministic integer inputs, and the TLA+ spec verifies it via the `QueryDeterminism` invariant.

What the frontier document needs additionally — and what the TLA+ spec assumes rather than proves — is that all validators *agree on `anchor_energy` and `anchor_epoch`*. That agreement is the responsibility of the BFT consensus layer modelled in `EvaporChainBFT.tla`. Together, the two specs prove:

> **Composite property (consensus + functional determinism):** if Tendermint BFT achieves anchor agreement, and validators apply the canonical `LazyEnergy` to that anchor, every state query is deterministic across the validator set.

The composite property is what the protocol actually relies on.

## 2. What the spec models

`RuleBasedConsensus.tla` (525 lines, this commit) models:

- A finite set of objects, each with an initial energy and a half-life.
- A finite set of validators, each able to perform queries.
- A monotonically advancing global epoch.
- A re-anchoring action that snapshots `anchor_energy` from the previous anchor at a specified `AnchorInterval`.
- A query action: any validator may query any object at any epoch in `[anchor_epoch, current_epoch]`.

It does *not* model:

- The Tendermint consensus that produces anchor agreement (separate spec).
- Object creation, refresh, or resurrection (Frontier #2 territory; out of scope here).
- The DA layer or any networking concerns.
- Adversarial behaviour. This spec is for the honest case. Adversarial behaviour is handled at the consensus layer, not at the lazy-evaluation layer.

The spec is deliberately small and focused. Each property it verifies should be provable in seconds by TLC on a modest bound.

## 3. Properties verified by TLC

Five properties are stated in the spec; the first four are strict invariants and the fifth is approximate:

| Property | Statement | Expected |
|---|---|---|
| `TypeOK` | All variables stay within their declared domains | PASS |
| `QueryDeterminism` | Same `(obj, qepoch)` ⇒ same result for all validators | **PASS — this is the central theorem** |
| `MonotoneDecay` | For fixed `obj`, energy is non-increasing in `qepoch` | PASS |
| `BoundedByInitial` | No query returns energy > `InitialEnergy[obj]` | PASS |
| `AnchorSanity` | `anchor_epoch ≤ epoch`, anchor map covers all objects | PASS |
| `ReAnchorEquivalenceApprox` | Re-anchoring is approximately equivalent to lazy from the previous anchor | **APPROXIMATE — see §4** |

If any of `QueryDeterminism`, `MonotoneDecay`, `BoundedByInitial`, or `AnchorSanity` fails under TLC, the integer `LazyEnergy` formula in `crates/evaporchain-types/src/lib.rs` has a bug. That would be the kind of bug an audit firm should find.

## 4. The integer-rounding gap (the subtlety the frontier doc hides)

The frontier document's proof sketch claims:

> The product of per-epoch decay factors equals the direct computation. QED for exponential decay.

This is true in continuous mathematics. In the integer-arithmetic implementation, **it is not exactly true**, because each step rounds down. Specifically:

For the EvaporChain decay formula:

$$
\text{LazyEnergy}(E_0, \tau, t) = \left\lfloor \frac{E_0}{2^{\lfloor t/\tau \rfloor}} \right\rfloor - \left\lfloor \frac{\lfloor E_0 / 2^{\lfloor t/\tau \rfloor} \rfloor \cdot (t \bmod \tau)}{2\tau} \right\rfloor
$$

Composition of two lazy applications:

$$
\text{LazyEnergy}(\text{LazyEnergy}(E_0, \tau, t_1), \tau, t_2) \stackrel{?}{=} \text{LazyEnergy}(E_0, \tau, t_1 + t_2)
$$

The continuous-version equality holds. The integer version does not in general — each step rounds down a fractional remainder, and rounding errors compose.

### 4.1 Magnitude of the error

The single-step rounding error is at most 1 (the `\lfloor \cdot \rfloor` introduces error < 1). After $k$ compositions, the accumulated error is at most $k$.

For the protocol's intended use:

- `AnchorInterval` is small (e.g., 100 epochs).
- An object whose energy crosses zero between anchors will have its energy quantum-collapsed to 0 at the next anchor.
- The cumulative rounding error after $\frac{T}{\text{AnchorInterval}}$ re-anchors over a long horizon $T$ is at most $\frac{T}{\text{AnchorInterval}}$ energy units.

For real parameters (`InitialEnergy` ≈ $10^6$, `HalfLife` ≈ $10^4$, $T$ ≈ $10^7$), this is $10^3$ units of error against $10^6$ initial — a relative error below 0.1% **for these specific parameters**.

**Caveat (Frontier #3, audit 2026-05-17):** this 0.1% figure is an arithmetic example, not a mechanized bound. `research/coq/LazyEagerEquivalence.v` proves only the one-sided bound `lazy ≤ eager`; it does not mechanize the magnitude. The above calculation is correct for the given concrete parameters but should not be cited as a general mechanized result. Mechanizing the magnitude bound is open work (see §5.1).

### 4.2 Why this matters for consensus

Because all validators apply the *same* formula, they accumulate the *same* rounding error. **Determinism is preserved.** The error is a property of the formula, not of the validators. Every validator that re-anchors will produce the same (slightly-rounded) anchor as every other validator. `QueryDeterminism` still holds.

What changes is: re-anchoring **does** drift away from "what a single lazy evaluation from genesis would have produced". This drift is bounded, deterministic, and reproducible.

### 4.3 What this means for the frontier doc's proof

The frontier doc's proof sketch (*"the product of per-epoch decay factors equals the direct computation"*) is **wrong as stated** for the integer formula but is **true in the continuous limit**. The right statement is:

> For any two consensus rounds in which all validators apply the same canonical `LazyEnergy` to the same anchor, the resulting `anchor_energy` is identical across validators, even though it may differ by a bounded amount from a hypothetical full-history-from-genesis lazy evaluation.

This statement is what `QueryDeterminism` formalises in the TLA+ spec and what TLC will confirm.

The frontier doc should be revised to reflect this more careful version. Otherwise an external auditor reading the doc will (correctly) reject the proof sketch.

### 4.4 Defensible design choice

There are three defensible responses to the rounding gap:

1. **Re-derive from genesis on every query.** Eliminates rounding-of-rounding. Cost: every query walks the entire object history. Unworkable for state at scale.
2. **Re-anchor at fixed intervals; accept bounded drift.** What EvaporChain does today. The drift is bounded and deterministic; the protocol absorbs it as a known property.
3. **Use a different decay formula that *is* exactly multiplicative under integer arithmetic.** Such formulae exist (e.g., direct shift-only, no fractional remainder) but they have coarser granularity. Trade-off.

Choice 2 is correct for EvaporChain's design goals. The TLA+ spec captures the trade-off explicitly and bounds the error.

## 5. What's still open

This spec proves **QueryDeterminism** (the central theorem) and gives a precise statement of the rounding gap. Remaining formal-verification work:

### 5.1 Full Coq / Lean mechanization

The TLA+ spec is checked by TLC for bounded models (small Objects, Validators, MaxEpoch). For an unbounded statement of the theorem ("for ALL parameter values, not just the bounded ones TLC explores"), a mechanized proof in Coq or Lean is needed.

The structure of the Coq proof would be:

```
Theorem lazy_energy_deterministic :
  forall (E0 hl elapsed : nat) (v1 v2 : Validator),
    lazy_energy v1 E0 hl elapsed = lazy_energy v2 E0 hl elapsed.
Proof.
  intros.
  unfold lazy_energy.
  reflexivity.   (* it's literally the same function *)
Qed.
```

The hard part of the Coq mechanization is not determinism; it's the rounding-error bound (§4.1). That is:

```
Theorem lazy_energy_composition_bound :
  forall (E0 hl t1 t2 : nat),
    | lazy_energy (lazy_energy E0 hl t1) hl t2
    - lazy_energy E0 hl (t1 + t2) | <= 1.
Proof.
  (* this is the real proof. It requires reasoning about
     integer division, floor semantics, and the saturation
     boundary where energy hits zero. *)
Admitted.
```

This is genuine open research work. Estimated effort: 2-4 weeks for a Coq-experienced researcher; longer for someone learning the system.

### 5.2 Combination with EvaporChainBFT.tla

The full protocol-level claim — *"if BFT achieves anchor agreement, then all queries are deterministic"* — requires composing this spec with `EvaporChainBFT.tla`. The composition is straightforward but produces a much larger model that TLC can only check on small bounds.

The composition is:

```
Spec_EvaporChain == Spec_BFT /\ Spec_RuleBasedConsensus
                           /\ (anchor_energy = the agreed anchor in BFT)
```

### 5.3 Resurrection across anchors

The current spec assumes objects only decay. In production, objects can be refreshed (energy restored) before they evaporate, and ghost records can be resurrected. Both require careful modelling because they re-introduce the question: which anchor is the "right" one to evaluate from for an object whose history crosses multiple anchors?

Frontier #2 (Energy-Annotated Verkle Trie) addresses this for the storage layer. The consensus-layer formal treatment is open.

### 5.4 Adversarial anchor proposal

This spec models an honest cluster. An adversarial validator may propose a bad anchor that produces wrong-but-deterministic results. That's a Tendermint BFT problem (covered by `EvaporChainBFT.tla`'s `Validity` invariant), not a Rule-Based Consensus problem. But the composition needs to be made formal.

## 6. How an auditor should read the spec

For an external audit firm engaging with EvaporChain:

1. Read the Rust implementation `crates/evaporchain-types/src/lib.rs` and confirm `energy_at_epoch` matches the formula in `LazyEnergy` (§3 of the .tla file).
2. Run TLC on `RuleBasedConsensus.cfg` and confirm all PASS-marked invariants pass.
3. Read §4 of this document and confirm the bounded-error argument is acceptable for the protocol's parameters.
4. Note that §5 (Coq mechanization, BFT composition, resurrection, adversarial cases) is open work — the audit should flag these as in-scope or out-of-scope per the engagement terms.

A mature audit would cover all four points and produce written findings on §4 specifically — auditors weight integer-rounding issues heavily because they are a common source of consensus-divergence bugs in production blockchains.

## 7. References

- `research/frontier/03-rule-based-consensus.md` — design rationale
- `research/tla/RuleBasedConsensus.tla` — formal spec
- `research/tla/RuleBasedConsensus.cfg` — TLC configuration
- `research/tla/EvaporChainBFT.tla` — Tendermint BFT spec (anchor-agreement layer)
- `crates/evaporchain-types/src/lib.rs` — Rust implementation of `LazyEnergy`
- Lamport, L. *Specifying Systems*. Addison-Wesley, 2002. (TLA+ canonical reference)
- Yu, Y. et al. *Model Checking TLA+ Specifications*. CHARME 1999.
- Pnueli, A. *The Temporal Logic of Programs*. FOCS 1977. (Liveness foundations)

---

**End of v0.1.**

Note for revision: §5.1 Coq theorem statement is illustrative — the actual mechanization should use a fixed-precision integer model that matches Rust's u64. Leave that to a Coq-fluent collaborator.
