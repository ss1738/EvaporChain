# ADR-002: Energy-Decay State Evaporation Model

**Status:** Accepted  
**Date:** 2026-01-20  
**Deciders:** Satyawan Singh (founder)

---

## Context

Every public blockchain that stores unbounded state eventually faces a state growth crisis: archival nodes must store all historical state, new full nodes take weeks to sync, and the marginal cost of adding state approaches zero for the writer while the cost is distributed across all validators.

Existing mitigations (EIP-4444 history expiry, Ethereum state expiry EIPs) are retroactive patches. The goal for EvaporChain is a protocol where **state naturally expires unless actively maintained** — analogous to thermodynamic entropy.

## Decision

Model each on-chain object as a thermodynamic system with an energy level that decays over time. When energy reaches zero, the object evaporates (is removed from the active state trie). Object creators pay a storage deposit; operators may replenish energy to extend lifetime.

Decay curves are pluggable: `Linear`, `Exponential`, `Stepped`, `Custom(Vec<(u64, f64)>)`.

The energy function is:

```
energy(t) = initial_energy * decay_curve(elapsed_epochs)
```

Evaporation happens at epoch boundaries via `collect_storage_rent()`.

## Alternatives considered

| Alternative | Why rejected |
|-------------|-------------|
| Ethereum-style state expiry with witness proofs | Complex, requires witnesses for every access to expired state; breaks existing tooling |
| Rent-only model (charge per byte per block, no expiry) | Rent debt can accumulate; doesn't actually remove state from the trie |
| No state management | Accepted for Ethereum V1; EvaporChain's thesis is that this is architecturally unsustainable at scale |
| Hard cap on state size | Arbitrary, creates rent-seeking around the cap; doesn't reflect actual cost |

## Consequences

- Objects that are not refreshed disappear. Applications must design for this — session keys, temporary NFTs, and ephemeral data are good fits; long-lived registries must budget for refresh costs.
- The `resurrect_object` transaction allows previously-evaporated objects to be re-created with the same ID (ghost bridges depend on this).
- Light clients can verify evaporation proofs against the Merkle root without downloading all objects.
- The Verkle trie (see ADR-005) is necessary to make evaporation proofs efficient at scale.
