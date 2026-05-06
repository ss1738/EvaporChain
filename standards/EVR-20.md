# EVR-20: Decaying Fungible Token Standard

| Field | Value |
|-------|-------|
| **Standard** | EVR-20 |
| **Title** | Decaying Fungible Token Standard |
| **Author** | Satyawan Singh |
| **Status** | Living |
| **Created** | 2026-03-26 |
| **Network** | EvaporChain Testnet |

## Implementation Status

> ⚠️ **The specification below is the target shape of EVR-20. Not every part is wired into the chain's HTTP API today.** External developers writing against this standard should consult the table below before integrating.
>
> | Surface | Status | Where |
> |---|---|---|
> | Read queries (token metadata, holder balance, supply with current decay) | ✅ Live | `GET /api/tokens/:id`, `GET /api/tokens/:id/holders/:addr` |
> | Token deployment (`deploy`) | ✅ Live | `POST /api/tokens` (admin-gated) |
> | `transfer(from, to, amount)` mutation endpoint | ⏳ Planned — **Phase 4.4** of `DOCTRINE_PUNCH_LIST.md` ecosystem layer | not yet wired into `evaporchain-node::api` |
> | `burn(amount)` mutation endpoint | ⏳ Planned — **Phase 4.4** | not yet wired |
> | `refresh(amount)` (counteract decay) | ⏳ Planned — **Phase 4.4** | substrate exists in `evaporchain-execution`; HTTP route not exposed |
> | Decay arithmetic (Coq-verified `energy_at_epoch`) | ✅ Live and machine-checked | `research/coq/EnergyDecayMonotonicity.v` |
>
> The mutation endpoints are deliberately not yet exposed: every code path that mutates token state must first land behind the antichain-mempool / MCC fork-choice promotion to `default` (Layer 4 hot-path wiring per the doctrine roadmap). When that ships, this banner will be replaced with a "✅ Fully implemented as of vX.Y" notice + commit reference.

## Abstract

EVR-20 defines a standard interface for fungible tokens with thermodynamic supply decay on the EvaporChain network. Unlike ERC-20 tokens which maintain a fixed total supply, EVR-20 tokens have a supply that decays exponentially over time according to a configurable half-life. All holder balances decay proportionally, creating a naturally deflationary token model that mirrors the thermodynamic principle of entropy increase.

## Motivation

Fixed-supply fungible tokens create several economic distortions:

1. **Permanent state cost**: Dead tokens with zero economic activity occupy blockchain state forever. There is no mechanism to reclaim the storage.

2. **No velocity incentive**: Holders can hoard tokens indefinitely with no cost, reducing circulation and economic utility.

3. **Misaligned with real-world value**: Most assets depreciate. Currency inflates. Stored energy dissipates. Yet fungible token standards model value as eternal.

EVR-20 introduces supply decay as a first-class protocol feature. Token supply and all balances decay at the same rate, creating a built-in cost of holding that incentivizes economic activity. Tokens can be refreshed to counteract decay, but abandoned token balances will eventually reach zero and be reclaimed by the network.

## Specification

### Token Schema

```
DeployedToken {
    id:               u64                    // Unique token identifier
    name:             String                 // Human-readable name
    symbol:           String                 // Trading symbol (e.g., "EVAP")
    total_supply:     u64                    // Supply at deployment
    decay_half_life:  u64                    // Epochs for supply to halve
    deployed_epoch:   u64                    // Epoch of deployment
    deployer:         Address                // Creator address
    balances:         Map<Address, u64>      // Holder balances
    last_decay_epoch: u64                    // Last epoch decay was applied
}
```

### Supply Decay Formula

Total supply and all balances decay according to:

```
S(t) = S_initial * 2^(-t / half_life)
```

Where:
- `S(t)` is the supply at epoch `t` (epochs elapsed since deployment)
- `S_initial` is the supply at deployment
- `half_life` is the number of epochs for supply to halve

**Implementation** uses the same integer-only decay function as EVR-721:

```
full_halvings = epochs_elapsed / half_life
remainder = epochs_elapsed % half_life
after_halvings = initial >> full_halvings
fractional_decay = after_halvings * remainder / (2 * half_life)
result = after_halvings - fractional_decay
```

### Balance Decay

All holder balances decay at the same rate as the total supply. This is applied lazily — when `tick_decay(epoch)` is called:

```
tick_decay(current_epoch):
  if current_epoch <= last_decay_epoch: return
  elapsed = current_epoch - last_decay_epoch
  for each (address, balance) in balances:
    balance = energy_at_epoch(balance, decay_half_life, elapsed)
  last_decay_epoch = current_epoch
```

This ensures proportional decay: if Alice holds 10% of total supply at epoch N, she still holds ~10% at epoch N+K, even though both her balance and total supply have decreased.

### Required Interface

#### Queries (Read)

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `list_tokens` | — | `Vec<TokenResponse>` | All deployed tokens |
| `get_token` | `token_id: u64` | `TokenResponse` | Token details with computed supply |
| `balance_of` | `token_id: u64, address: Address` | `u64` | Current balance after decay |
| `total_supply` | `token_id: u64` | `u64` | Current supply after decay |

The `TokenResponse` includes computed fields:

```
TokenResponse {
    id:               u64
    name:             String
    symbol:           String
    total_supply:     u64       // Original supply at deployment
    current_supply:   u64       // Supply after decay at current epoch
    decay_half_life:  u64
    deployed_epoch:   u64
    deployer:         Address
    decay_percentage: f64       // % of original supply lost to decay
    holder_count:     usize     // Number of non-zero balance holders
    holders:          Vec<TokenHolder>
}

TokenHolder {
    address: Address
    balance: u64                // Current balance after decay
}
```

#### Mutations (Write)

| Method | Parameters | Auth Required | Description |
|--------|-----------|--------------|-------------|
| `deploy` | `name, symbol, total_supply, decay_half_life` | Sender signature | Create new token |
| `transfer` | `token_id, from, to, amount` | Holder signature (caller == from) | Transfer tokens |
| `mint` | `token_id, to, amount` | Deployer only | Mint additional supply |
| `burn` | `token_id, from, amount` | Holder signature (caller == from) | Burn tokens from balance |
| `refresh_balance` | `token_id, address, energy` | Any signature | Add energy to counter decay |

> **Auth note (2026-05-03 reconciliation):** "Owner signature" in the
> previous version of this spec was ambiguous — it could mean the
> contract deployer/owner or the balance holder. The reference
> implementation (`crates/evaporchain-contracts/src/lib.rs`) had
> initially required `caller == creator` for all three privileged
> ops, which contradicted ERC-20 parity. The canonical EVR-20
> behaviour is now: **caller MUST equal `from` for `transfer` and
> `burn`; the contract deployer has no override**. `refresh_balance`
> remains permissionless (any caller). Only `mint` is deployer-only,
> matching ERC-20's typical issuer pattern.

**Deploy** creates a new token:
```
deploy(name, symbol, total_supply, decay_half_life) -> token_id
  Creates: DeployedToken with full supply allocated to deployer
  Gas cost: 65,000 + creation deposit
```

**Transfer** moves tokens between addresses:
```
transfer(token_id, from, to, amount) -> ()
  Requires: caller == from, balances[from] >= amount
  Effect:   balances[from] -= amount, balances[to] += amount
  Gas cost: 21,000
```

**Mint** increases supply (deployer-only):
```
mint(token_id, to, amount) -> ()
  Requires: caller == deployer
  Effect:   balances[to] += amount, total_minted += amount
  Gas cost: 21,000
```

**Burn** permanently destroys tokens:
```
burn(token_id, from, amount) -> ()
  Requires: caller == from, balances[from] >= amount
  Effect:   balances[from] -= amount
  Gas cost: 21,000
```

**Refresh Balance** counteracts decay for a specific holder:
```
refresh_balance(token_id, address, energy) -> ()
  Effect: balances[address] += energy
  Gas cost: 21,000 + refresh fee
```

### Contract Template

EVR-20 tokens can also be deployed as smart contracts using the `DecayingToken` template:

```
DecayingToken {
    state: TokenState {
        name:             String
        symbol:           String
        balances:         Map<String, u64>
        decay_half_life:  u64
        owner:            String
        total_minted:     u64
        total_decayed:    u64
        last_tick_epoch:  u64
    }

    methods:
        mint(to, amount)           -> {minted: amount}
        transfer(from, to, amount) -> {transferred: amount}
        balance_of(addr)           -> {balance: u64}
        total_supply()             -> {total_supply: u64}
        burn(from, amount)         -> {burned: amount}
        refresh_balance(addr, energy) -> {refreshed: energy}
}
```

### Fee Structure

| Operation | Gas | Additional Fee |
|-----------|-----|---------------|
| Deploy | 65,000 | Creation deposit |
| Transfer | 21,000 | — |
| Mint | 21,000 | — |
| Burn | 21,000 | — |
| Refresh | 21,000 | Refresh fee (proportional to energy) |

All fees are burned (deflationary).

## Differences From ERC-20

| Feature | ERC-20 (Ethereum) | EVR-20 (EvaporChain) |
|---------|-------------------|---------------------|
| Supply model | Fixed or inflationary | Naturally deflationary (exponential decay) |
| Balance persistence | Permanent | Decays over time |
| Holding cost | None (gas-free to hold) | Implicit (balance shrinks if not refreshed) |
| Velocity incentive | None | Natural (use it or lose it) |
| State cleanup | Never | Balances reaching 0 can be pruned |
| Refresh mechanism | Not applicable | Native `refresh_balance` method |
| Proportionality | N/A | All balances decay at same rate |

## Economic Properties

### Demurrage Currency

EVR-20 implements a form of **demurrage** — a cost levied on holding currency. This was proposed by Silvio Gesell in the early 20th century and has been validated in local currency experiments (e.g., the Wörgl experiment of 1932). Key properties:

1. **Increased velocity**: Holders are incentivized to spend or invest rather than hoard
2. **Natural deflation**: Total supply decreases without requiring burns or fee mechanisms
3. **Self-cleaning state**: Abandoned balances eventually decay to zero

### Half-Life Selection Guide

| Token Type | Suggested Half-Life | Rationale |
|------------|-------------------|-----------|
| Governance tokens | 500-2000 epochs | Long-lived, slow decay encourages participation |
| Utility tokens | 50-200 epochs | Medium decay matches usage cycles |
| Reward tokens | 10-50 epochs | Fast decay prevents hoarding of rewards |
| Ephemeral credits | 1-10 epochs | Very fast decay for session-based use |
| Stablecoins | 5000+ epochs | Very slow decay, nearly permanent |

### Genesis Tokens on EvaporChain Testnet

| Token | Symbol | Supply | Half-Life | Behaviour |
|-------|--------|--------|-----------|-----------|
| EvaporChain | EVAP | 962,716 | 1000 epochs | Core governance, very slow decay |
| Flux Token | FLUX | 183,272 | 20 epochs | Fast-decaying utility token |
| Thermal Credits | HEAT | 14,258 | 5 epochs | Rapid-decay ephemeral credits |

## Reference Implementation

The reference implementation is located in the EvaporChain repository:

- **Token struct and API**: `crates/evaporchain-node/src/api.rs` (DeployedToken, TokenStore)
- **Contract template**: `crates/evaporchain-contracts/src/lib.rs` (DecayingToken)
- **Energy decay formula**: `crates/evaporchain-types/src/lib.rs` (energy_at_epoch)
- **API endpoints**: `crates/evaporchain-node/src/api.rs` (/api/token/*)

Live testnet: https://testnet.evaporchain.com/tokens

## Security Considerations

1. **Rounding dust**: Repeated decay operations may leave rounding dust (balances of 1-2 units). Implementations should consider a minimum balance threshold below which balances are zeroed.

2. **Lazy vs eager decay**: The reference implementation uses lazy decay (applied on access). Eager decay (applied every epoch) would be more expensive but ensures consistent state. Cross-contract calls should trigger `tick_decay` before reading balances.

3. **Proportional fairness**: All balances must decay at the same rate. Implementations must not allow any address to exempt itself from decay.

4. **Overflow on refresh**: The `refresh_balance` method adds energy to a balance. Implementations must check for u64 overflow.

5. **Deterministic computation**: The integer-only decay formula ensures deterministic results across all nodes, preventing consensus divergence.

## Copyright

This standard is released under the MIT License as part of the EvaporChain project.
