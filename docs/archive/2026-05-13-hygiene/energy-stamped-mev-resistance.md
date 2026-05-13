# EvaporChain Proposal: Energy-Stamped MEV Resistance

**Status:** draft
**Author:** Satyawan Singh + Claude (collaborative)
**Date:** 2026-04-28

## Summary

Use EvaporChain's thermodynamic state-decay primitive as an **MEV defense
mechanism**. Each transaction carries an "energy stamp" that decays
exponentially with the number of blocks the tx waits in the mempool.
Validator rewards are weighted by remaining stamp at inclusion time.
This makes ordering games (sandwich, frontrun, backrun) economically
self-defeating: holding a tx in the mempool to insert your own first
DECAYS the tx you're waiting on, so reordering bleeds value from the
validator who reorders.

EvaporChain is the only L1 with a native energy-decay primitive in the
state machine, so this defense is uniquely available here.

## Background

Existing MEV defenses fall into three buckets:

1. **Commit-reveal (encrypted mempool)** — encrypts tx until ordering
   is fixed. Effective but adds latency. EvaporChain ships this
   already (`crates/evaporchain-consensus/src/encrypted_mempool.rs`).
2. **Threshold-encryption + DKG** — same idea, distributed key.
   Operationally heavier.
3. **PBS (proposer-builder separation)** — outsources ordering to a
   marketplace. Doesn't eliminate MEV; just makes it explicit.

This proposal adds a **fourth** mechanism that complements (1):
**economic-decay-based ordering**.

## Mechanism

### 1. Tx-level energy stamp

At submission, every transaction is assigned an `inclusion_energy` and
an `inclusion_half_life`:

```rust
pub struct InclusionStamp {
    pub initial_energy: u64,    // e.g. 1_000_000
    pub half_life_blocks: u64,  // e.g. 4
    pub submitted_block: u64,
}

impl InclusionStamp {
    pub fn priority_at(&self, current_block: u64) -> u64 {
        let elapsed = current_block.saturating_sub(self.submitted_block);
        energy_at_epoch(self.initial_energy, self.half_life_blocks, elapsed)
    }
}
```

`energy_at_epoch` is the existing chain primitive
(`evaporchain-types::lib::energy_at_epoch`).

### 2. Mempool ordering

`Mempool::take_with_priority(n)` returns the `n` highest-priority txs
where priority is `inclusion_stamp.priority_at(current_block)`. Ties
broken by sender + nonce (existing `sort_nonce_aware`).

This means:
- Old txs naturally drop off the front of the queue.
- A tx held back two `half_life` periods has 1/4 of its initial
  inclusion priority.
- A tx held back six `half_life` periods is at 1/64 — effectively
  unincludeable.

### 3. Validator reward weighting

Block reward = `BASE_BLOCK_REWARD + Σ tx.priority_at(inclusion_block)`.
The proposer's economic incentive is to **include high-priority txs
fast**. Holding a tx for one block to insert your own first costs
`(1 - 0.5^(1/half_life))` of that tx's reward weight.

For `half_life = 4`, the per-block decay is ~16%. To insert one of
your own txs in front of a victim, you sacrifice 16% of the victim's
weighted reward. To run a 3-tx sandwich (yours, victim, yours), you
sacrifice ~30%.

If your sandwich gross profit is below the decay cost, the attack
is unprofitable.

### 4. MEV-tx detection (optional refinement)

A tx can be flagged as MEV-suspect if it pays an unusually large
priority fee for its position; those txs get a **steeper half_life**
(e.g. 1 block) so they self-decay faster. This is the
"penalize-the-attempt" knob.

## Properties

### Strength

- **No new cryptography.** Uses the existing `energy_at_epoch`
  primitive — no new audit surface.
- **Composable with encrypted mempool.** Encrypted txs can carry
  inclusion stamps too; the stamp decays during the commit phase but
  is bounded by the reveal deadline.
- **Validator-aligned.** Honest proposers MAXIMIZE their reward by
  including all eligible txs ASAP — exactly the behavior we want.
- **Sybil-resistant.** Submitter can't game by spamming multiple tx
  versions: each one decays independently, mempool has the global
  byte cap (closed by K-05).

### Weaknesses

1. **Initial-energy gaming.** Wallets / SDKs would set
   `initial_energy = MAX_U64` to maximize priority. Need a
   normalization pass — divide by the chain's `BASE_INCLUSION_ENERGY`
   constant before ranking. Or charge a fee proportional to declared
   initial energy.
2. **Stale tx burden.** Mempool keeps decayed txs around until TTL
   evicts them. Mitigation: drop priority-zero txs at next take().
3. **Block-time-dependent.** Half-life in BLOCKS means the decay
   rate is sensitive to block interval. Recommend half_life of 4
   blocks at 2s = 8s halving — feels right for ETH-scale ordering
   attacks (12s slot).

## Integration

### Phase 1 (this proposal)

- Add `submit_block: u64` field to mempool entries (already partially
  there via `tx_submit_epoch` map).
- `Mempool::take_with_priority(n, current_block)` method using
  `energy_at_epoch` for ordering.
- Wire into `TendermintConsensus::create_proposal` as the default
  drain method, replacing the current nonce-aware `take(n)`.
- Reward function update in `evaporchain-execution::fees` — include
  Σ priority_at_inclusion in proposer reward computation.

### Phase 2

- Tx-level `inclusion_stamp` field on `Transaction` variants.
  Currently the mempool would synthesize this from `submit_epoch +
  default initial_energy`. Phase 2 lets users set their own.
- Fee-model integration: charging `f(declared_initial_energy)` so
  submitters can't trivially game.

### Phase 3

- Per-tx-type half_life tuning (cross-shard txs decay faster, e.g.).
- MEV-suspect detection using the existing PoHA infrastructure to
  flag sandwich-shaped tx clusters.

## References

- `crates/evaporchain-types/src/lib.rs::energy_at_epoch` — the
  decay primitive this builds on.
- `crates/evaporchain-consensus/src/encrypted_mempool.rs` — the
  commit-reveal MEV defense already shipped.
- `crates/evaporchain-consensus/src/mempool.rs::sort_nonce_aware` —
  the current tie-break semantics that this would replace.

## Open questions

1. Is `energy_at_epoch` overflow-safe at `initial_energy = MAX_U64`
   and `elapsed = 0`? (Should be — returns initial unchanged.)
2. How should the proposer signal "I'm choosing tx X over tx Y to
   maximize reward, not to MEV"? Maybe a per-block "ordering proof"
   showing the priority sum is monotonic.
3. Does this interact badly with the validator delegation reward
   split (K-11)? The total reward grows; the split should still be
   the same percentages.

## Why only EvaporChain can build this

Every other L1 would need to add an entirely new state-decay
primitive to its consensus layer to implement this defense. EvaporChain
already has it, audited via Coq (`research/coq/EnergyDecayMonotonicity.v`)
and TLA+ (`research/tla/EnergyVerkleTrie.tla`). The same primitive
that powers state evaporation, ghost records, and the
energy-Verkle trie now serves as the unit of MEV defense.

This is a moat: nobody else can copy this without a full L1 rewrite.
