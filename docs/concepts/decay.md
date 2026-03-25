# Thermodynamic State Decay

EvaporChain is the first blockchain where state has a natural lifespan. Every state object carries an energy budget that decays exponentially over time, modeled after radioactive half-life decay. When energy reaches zero, the state evaporates — replaced by a compact cryptographic ghost record.

## The Decay Formula

Energy decays according to exponential half-life:

```
energy(t) = initial_energy * 2^(-t / half_life)
```

Where:
- `initial_energy` — energy when last refreshed
- `t` — epochs elapsed since last refresh
- `half_life` — number of epochs for energy to halve

### Worked Example

An object created with `energy = 10,000` and `half_life = 50`:

| Epoch | Energy | Decay |
|-------|--------|-------|
| 0 | 10,000 | 0% |
| 50 | 5,000 | 50% |
| 100 | 2,500 | 75% |
| 150 | 1,250 | 87.5% |
| 200 | 625 | 93.75% |
| 332 | ~1 | ~99.99% |
| 333+ | 0 | Evaporated |

The total lifespan before evaporation is approximately `half_life * log2(initial_energy)` epochs.

## State Lifecycle

Every state object on EvaporChain goes through these phases:

```
Created ──> Active ──> Grace ──> Ghost (Evaporated)
                │         │
                │         └── Refresh ──> Active
                │
                └── Refresh ──> Active (energy reset)
```

### Active

The object is alive and fully functional. Energy is decaying but still above zero. Can be read, written, and interacted with normally.

### Grace Period

Energy has dropped to zero but the object hasn't been removed yet. During grace (typically ~3-5 epochs), the object can still be saved by depositing new energy via a refresh transaction. Think of it as a "last chance" window.

Smart contracts with `on_grace()` hooks are notified when entering this state.

### Ghost (Evaporated)

The object's full state data is removed from the chain. What remains is a ghost record:

```json
{
  "object_id": "0a00...0000",
  "owner": "0100...0000",
  "evaporated_at": 342,
  "data_hash": "b7c3f2..."
}
```

The ghost preserves a cryptographic hash of the original data, proving the object once existed. See [Ghost Records](ghosts.md) for details.

### Risen (Resurrected)

A ghost can be brought back to life by depositing new energy. The object returns to Active state. This is the only blockchain where you can bring state back from the dead.

## Why Decay Matters

### 1. State Bloat Elimination

Traditional blockchains accumulate state forever. Ethereum's state trie has grown to hundreds of gigabytes. EvaporChain's state naturally shrinks as unused objects evaporate. Only data that someone actively maintains persists.

### 2. Storage Cost Alignment

On EvaporChain, the cost of keeping data alive is proportional to how long you keep it. This creates honest economics: temporary data (event tickets, session tokens, flash loan state) costs less than permanent data because it evaporates naturally.

### 3. Natural Garbage Collection

No need for complex state rent mechanisms or EIP-4444-style history expiry proposals. The protocol handles cleanup through thermodynamics. Dead contracts don't accumulate — they evaporate.

### 4. Programmable Lifespans

By choosing `half_life`, developers control how long their state persists:

| Half-life | Use Case |
|-----------|----------|
| 5 epochs | Flash data, temporary credentials |
| 50 epochs | Session state, short-lived tokens |
| 500 epochs | Long-lived contracts, persistent services |
| 5000 epochs | Near-permanent data (decades to evaporate) |

## Energy and Refresh

Energy is the "fuel" that keeps state alive. It has two properties:

- **Amount** — determines how much time before evaporation
- **Half-life** — determines the decay rate (set at creation, immutable)

### Refreshing

Anyone can deposit energy into a state object:

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/refresh \
  -H "Content-Type: application/json" \
  -d '{"object_id": 10, "energy_deposit": 5000}'
```

When refreshed:
1. Current decayed energy is computed
2. The deposit is added to the current energy
3. The `last_refreshed` epoch is updated
4. Decay restarts from the new energy level

This means refreshing is always beneficial — it extends the object's lifetime from the current point forward.

## Decay Across the Stack

Thermodynamic decay isn't limited to state objects. EvaporChain applies it uniformly:

| Layer | What Decays | Half-life |
|-------|-------------|-----------|
| State Objects | Object energy | Per-object |
| Smart Contracts | Contract energy | Per-contract |
| NFTs | NFT energy | Per-NFT |
| Tokens | Token supply | Per-token |
| Staking Rewards | Unclaimed rewards | Per-pool |
| DAO Proposals | Voting window | Per-proposal |

This creates a consistent mental model: everything on EvaporChain has a natural lifespan. Nothing lives forever unless someone actively maintains it.

## Implementation

The core decay function in Rust (`evaporchain-types`):

```rust
pub fn energy_at_epoch(initial: u64, half_life: u64, elapsed: u64) -> u64 {
    if half_life == 0 || initial == 0 {
        return 0;
    }
    // energy = initial * 2^(-elapsed / half_life)
    // Using integer arithmetic to avoid floating point
    let full_halvings = elapsed / half_life;
    if full_halvings >= 64 {
        return 0;
    }
    let remaining = initial >> full_halvings;
    let fractional_epochs = elapsed % half_life;
    if fractional_epochs == 0 || remaining == 0 {
        return remaining;
    }
    // Linear interpolation for sub-halving precision
    let next_halving = remaining >> 1;
    let diff = remaining - next_halving;
    remaining - (diff * fractional_epochs / half_life)
}
```

This is computed on-the-fly — no state mutation needed to track decay. The chain only stores `initial_energy`, `half_life`, and `last_refreshed`. Current energy is always derived.
