# Ghost Records and Resurrection

When a state object's energy decays to zero on EvaporChain, it doesn't simply disappear. The full state data is removed, but a compact cryptographic ghost record is left behind — a proof that the object once existed.

## What is a Ghost?

A ghost record contains:

```json
{
  "object_id": "0a00000000000000000000000000000000000000000000000000000000000000",
  "owner": "0100000000000000000000000000000000000000000000000000000000000000",
  "evaporated_at": 342,
  "data_hash": "b7c3f2a1e4d5..."
}
```

| Field | Description |
|-------|-------------|
| `object_id` | The original 32-byte object ID |
| `owner` | The address that owned the object at evaporation |
| `evaporated_at` | The epoch when evaporation occurred |
| `data_hash` | Cryptographic hash of the object's full state data |

The ghost is much smaller than the original state — it only stores proof of existence, not the data itself.

## The Evaporation Process

```
Active Object          Grace Period           Ghost
┌──────────────┐      ┌──────────────┐      ┌──────────────┐
│ full state   │      │ full state   │      │ object_id    │
│ energy: 1250 │ ──>  │ energy: 0    │ ──>  │ owner        │
│ data: [...]  │      │ grace_epoch  │      │ evaporated_at│
│ owner        │      │ last chance  │      │ data_hash    │
└──────────────┘      └──────────────┘      └──────────────┘
    ~500 bytes           ~500 bytes             ~100 bytes
```

1. **Energy reaches zero** — the object enters the grace period
2. **Grace period expires** (typically 3-5 epochs) — if no one refreshes the object
3. **Evaporation** — full state data is removed, ghost record is created
4. **State root updates** — the ghost is included in the Merkle root

Smart contracts with `on_evaporate()` hooks are called before the state is removed, giving them a chance to emit final events or archive data.

## Querying Ghosts

### API

```bash
# List all ghost records
curl https://testnet.evaporchain.com/api/ghosts
```

Response:

```json
[
  {
    "id": "0a00000000000000000000000000000000000000000000000000000000000000",
    "original_owner": "0100000000000000000000000000000000000000000000000000000000000000",
    "evaporated_epoch": 342,
    "data_hash": "b7c3f2a1e4d5..."
  }
]
```

### SDK

```typescript
const ghosts = await chain.getGhosts();

for (const ghost of ghosts) {
  console.log(`Ghost ${ghost.id} — evaporated at epoch ${ghost.evaporated_epoch}`);
  console.log(`  Owner: ${ghost.original_owner}`);
  console.log(`  Data hash: ${ghost.data_hash}`);
}
```

## Resurrection

EvaporChain is the only blockchain where you can bring state back from the dead. Any ghost can be resurrected by depositing new energy.

### Via API

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/resurrect \
  -H "Content-Type: application/json" \
  -d '{
    "object_id": 10,
    "energy_deposit": 10000
  }'
```

### Via SDK

```typescript
await chain.resurrectObject(10, 10000);
```

### What Happens During Resurrection

1. The ghost record is found by object ID
2. A new active state object is created with the deposited energy
3. The object's state is restored (with fresh energy and a new `last_refreshed` epoch)
4. The ghost record is removed
5. The object enters "Risen" state (functionally identical to Active)
6. Smart contracts with `on_refresh()` hooks are notified

### Limitations

- The resurrector must provide energy — there's no free resurrection
- The object's original data may not be fully recoverable (only the hash is preserved in the ghost). On the current testnet, objects retain enough metadata for restoration
- Resurrection doesn't change the object's half-life — it keeps the same decay rate

## Ghost Gallery

The NFT marketplace at `/nft` features a "Ghost Gallery" — a visual display of evaporated NFTs. Each ghost NFT shows its original generative art in a faded, spectral style, along with the evaporation epoch and data hash.

Ghost NFTs can be resurrected through the UI, bringing them back to the active collection with fresh energy.

## Use Cases for Ghosts

### Proof of Historical Existence

The `data_hash` in a ghost record is a cryptographic proof that specific data once existed on-chain. This is useful for:

- **Audit trails** — prove a contract existed and what it contained
- **Insurance claims** — prove a policy was active at a specific epoch
- **Legal records** — prove a document was notarized on-chain

### Conditional Resurrection

Applications can monitor ghosts and selectively resurrect valuable state:

```typescript
const ghosts = await chain.getGhosts();
const recentGhosts = ghosts.filter(
  g => g.evaporated_epoch > currentEpoch - 100
);

for (const ghost of recentGhosts) {
  // Resurrect ghosts that evaporated recently
  await chain.resurrectObject(parseInt(ghost.id.slice(0, 2), 16), 10000);
}
```

### State Archaeology

Ghosts form a historical record of the chain's evolution. By examining ghost records, you can understand what state the chain has shed over time — a form of blockchain archaeology unique to EvaporChain.

## Ghost Proofs in NFTs

Mortal NFTs on EvaporChain generate a ghost proof when they evaporate. This proof includes:

- Blake3 hash of the NFT metadata (name, collection, owner, minted epoch)
- Evaporation epoch
- Original energy and half-life parameters

The ghost proof serves as a certificate of existence — proof that an NFT was minted, lived for a specific duration, and evaporated naturally through thermodynamic decay.
