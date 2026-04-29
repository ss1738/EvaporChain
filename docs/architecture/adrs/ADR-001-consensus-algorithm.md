# ADR-001: Tendermint BFT as Consensus Algorithm

**Status:** Accepted  
**Date:** 2026-01-15  
**Deciders:** Satyawan Singh (founder)

---

## Context

EvaporChain requires Byzantine fault-tolerant consensus with deterministic finality. Deterministic finality (no forks after commit) is a hard requirement because:

1. The energy-decay state model means object state is time-indexed — a fork would produce divergent decay histories that cannot be merged.
2. Cross-chain resurrection (ghost bridges) requires a provably final commit before a foreign chain can act on a state claim.
3. Storage rent enforcement must be monotone — re-orgs that un-create objects would corrupt deposit accounting.

## Decision

Use Tendermint BFT (rounds of propose → prevote → precommit with 2/3 stake-weighted quorum) as the consensus algorithm.

## Alternatives considered

| Algorithm | Why rejected |
|-----------|-------------|
| Nakamoto PoW | Probabilistic finality incompatible with decay-indexed state |
| PBFT (classic) | O(n²) message complexity; doesn't scale past ~100 validators |
| HotStuff / Tendermint variants | Tendermint is the most audited BFT implementation; HotStuff adds linear communication at the cost of an extra round in the common case |
| Avalanche | Probabilistic finality (same problem as Nakamoto); leaderless design conflicts with ordered execution needed for temporal contracts |

## Consequences

- Maximum validator set is practically bounded by network bandwidth (~200 for 2s block times), not a hard constant.
- Safety holds under asynchrony (no liveness, but no forks); liveness requires partial synchrony (eventual message delivery).
- The 2/3 threshold means the network tolerates up to 1/3 malicious stake before safety breaks.
- View-change (round) logic must be carefully tested — this is where most Tendermint CVEs appear.
