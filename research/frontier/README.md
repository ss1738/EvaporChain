# EvaporChain Frontier Research

Three novel primitives that don't exist in any blockchain or academic literature.
Target: unified paper at ACM CCS 2026 or USENIX Security 2027.

## Primitives

| # | Primitive | Status | Tests | Doc |
|---|-----------|--------|-------|-----|
| 1 | [Proof-of-Historical-Availability (PoHA)](01-poha-decaying-da.md) | **Done** | 19 | Decaying DA certificates |
| 2 | [Energy-Annotated Verkle Trie](02-energy-verkle-trie.md) | **Done** | 26 | Self-pruning state tree |
| 3 | [Rule-Based Consensus](03-rule-based-consensus.md) | **Done** | 28 | Consensus over decay rules, not state snapshots |

## Build Order

1. **Energy-Annotated Verkle Trie** — lowest risk, self-contained, new data structure
2. **PoHA** — extends existing `evaporation_da.rs`, strongest publication potential
3. **Rule-Based Consensus** — fixes state root divergence, hardest, most fundamental

## Paper Strategy

One unified paper: "Thermodynamic Blockchain Primitives: State Decay as a First-Class Distributed Systems Abstraction"

Contributions: (a) energy-annotated authenticated data structures, (b) decaying data availability certificates, (c) rule-based consensus for time-dependent state.
