# Mortal NFT Guide

EvaporChain introduces Mortal NFTs — non-fungible tokens with thermodynamic energy that decays over time. Unlike traditional NFTs that exist forever, Mortal NFTs live, age, and eventually evaporate into ghost records. This creates scarcity through time, not just supply.

## NFT Lifecycle

```
Minted ──> Active ──> Grace ──> Ghost
               │         │
               │         └── Refresh ──> Active (energy restored)
               │
               └── Refresh ──> Active (energy reset)
```

### Active

The NFT is alive. Energy decays exponentially based on its half-life. The NFT can be transferred, displayed, and traded normally.

### Grace Period

Energy has reached zero. The NFT enters a grace window (~5 epochs) where it can still be saved by depositing new energy. If no one refreshes it, it evaporates.

### Ghost

The NFT's metadata is removed. A ghost record remains with the metadata hash, proving the NFT once existed. Ghost NFTs appear in the Ghost Gallery on the marketplace page.

Ghost NFTs can be resurrected by depositing fresh energy.

## Mint an NFT

### Via API

```bash
curl -X POST https://testnet.evaporchain.com/api/nft/mint \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Sunset Over Mountains",
    "collection": "Landscapes",
    "owner": "0x910000…0000",
    "energy": 5000,
    "half_life": 100
  }'
```

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | NFT name |
| `collection` | string | Collection name |
| `owner` | string | Owner address |
| `energy` | integer | Initial energy (determines lifespan) |
| `half_life` | integer | Epochs for energy to halve |

Response:

```json
{
  "success": true,
  "message": "Minted NFT #7 'Sunset Over Mountains' with energy 5000 (HL 100)"
}
```

### Via Dashboard

Navigate to [testnet.evaporchain.com/nft](https://testnet.evaporchain.com/nft) and use the mint form at the bottom of the page. The lifespan preview shows estimated epochs until evaporation.

## Choosing Energy and Half-life

The lifespan of an NFT is approximately `half_life * log2(energy)` epochs.

| Energy | Half-life | Lifespan | Character |
|--------|-----------|----------|-----------|
| 1,000 | 5 | ~50 epochs | Firefly — blink and it's gone |
| 5,000 | 50 | ~615 epochs | Seasonal — lives for a while |
| 10,000 | 100 | ~1,330 epochs | Durable — long-lived art |
| 10,000 | 500 | ~6,644 epochs | Heirloom — nearly permanent |

The half-life is immutable after minting. Energy can always be added via refresh.

## Transfer an NFT

```bash
curl -X POST https://testnet.evaporchain.com/api/nft/transfer \
  -H "Content-Type: application/json" \
  -d '{
    "id": 7,
    "to": "0x2b0000…0000"
  }'
```

Transfers change ownership but don't affect energy or decay rate.

## Refresh (Extend Lifespan)

Deposit energy to prevent evaporation:

```bash
curl -X POST https://testnet.evaporchain.com/api/nft/refresh \
  -H "Content-Type: application/json" \
  -d '{
    "id": 7,
    "energy": 5000
  }'
```

This adds 5,000 energy to the NFT's current energy level and resets the decay clock. Refreshing a ghost NFT resurrects it.

## Query NFTs

### List All NFTs

```bash
curl https://testnet.evaporchain.com/api/nfts
```

Response includes computed fields:

```json
{
  "id": 1,
  "name": "Eternal Flame",
  "collection": "Genesis",
  "owner": "0x7f0000…0000",
  "energy": 10000,
  "max_energy": 10000,
  "half_life": 500,
  "minted_epoch": 0,
  "state": "Active",
  "current_energy": 9800,
  "decay_pct": 2.0,
  "epochs_remaining": 4482
}
```

### Get Single NFT

```bash
curl https://testnet.evaporchain.com/api/nft/1
```

## Genesis NFTs

The testnet launches with 6 genesis NFTs, each with different decay characteristics:

| Name | Half-life | Character |
|------|-----------|-----------|
| Eternal Flame | 500 | Near-permanent, slow decay |
| Shooting Star | 10 | Burns bright, dies fast |
| Sunset Canvas | 50 | Medium lifespan |
| Quantum Bloom | 25 | Short-lived but vivid |
| First Light | 100 | Long-lived, steady decay |
| Binary Requiem | 5 | Ephemeral, almost instant |

## Generative Art

Each NFT on the marketplace page has unique generative canvas art derived from its ID, name, and collection. The art is rendered client-side using HTML5 canvas with deterministic seed-based generation — same NFT always produces the same art.

Ghost NFTs display their art in a faded, spectral style in the Ghost Gallery.

## Ghost Gallery

The marketplace page (`/nft`) has two tabs:

- **Live Collection** — active and grace-period NFTs with energy bars
- **Ghost Gallery** — evaporated NFTs displayed in spectral style

Ghost NFTs show:
- Original generative art (faded)
- Evaporation epoch
- Ghost proof hash
- Resurrect button

## Building on Mortal NFTs

### Monitor Energy Levels

Track NFTs approaching evaporation:

```typescript
import { EvaporChain } from "@evaporchain/sdk";

const chain = new EvaporChain("https://testnet.evaporchain.com");

// Poll NFTs and alert on low energy
setInterval(async () => {
  const response = await fetch("https://testnet.evaporchain.com/api/nfts");
  const nfts = await response.json();

  for (const nft of nfts) {
    if (nft.state === "Active" && nft.decay_pct > 80) {
      console.log(`WARNING: ${nft.name} is ${nft.decay_pct}% decayed`);
      console.log(`  ~${nft.epochs_remaining} epochs remaining`);
    }
    if (nft.state === "Grace") {
      console.log(`CRITICAL: ${nft.name} is in grace period — refresh now!`);
    }
  }
}, 10000);
```

### Auto-Refresh Valuable NFTs

```typescript
// Refresh NFTs below 20% energy
const nfts = await (await fetch("https://testnet.evaporchain.com/api/nfts")).json();

for (const nft of nfts) {
  if (nft.decay_pct > 80 && nft.state !== "Ghost") {
    await fetch("https://testnet.evaporchain.com/api/nft/refresh", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ id: nft.id, energy: 5000 }),
    });
    console.log(`Refreshed ${nft.name} with 5000 energy`);
  }
}
```

## Design Philosophy

Traditional NFTs exist forever, creating a paradox: if everything is permanent, nothing is scarce in time. Mortal NFTs solve this by making time the ultimate scarce resource.

- **Scarcity through time** — an NFT that existed for 10,000 epochs and naturally evaporated is more interesting than one that sits in a wallet forever
- **Active ownership** — owners must decide: refresh to keep alive, or let it die and become a ghost?
- **Ghost provenance** — the ghost record proves the NFT lived, creating a new form of digital archaeology
- **Natural curation** — the collection self-curates over time as neglected NFTs evaporate
