# Getting Started with EvaporChain Testnet

EvaporChain is a blockchain where state decays over time. Every object, token, contract, and NFT has thermodynamic energy that depletes according to an exponential half-life. When energy reaches zero, state evaporates into a ghost record — a cryptographic proof that something once existed.

This guide walks you through connecting to the testnet, getting tokens, and sending your first transaction.

## Testnet Information

| Property | Value |
|----------|-------|
| RPC Endpoint | `https://testnet.evaporchain.com` |
| Chain Name | EvaporChain |
| Native Token | EVAP |
| Block Time | ~3 seconds |
| Explorer | [testnet.evaporchain.com/explorer](https://testnet.evaporchain.com/explorer) |
| Wallet | [testnet.evaporchain.com/wallet](https://testnet.evaporchain.com/wallet) |
| Faucet | [testnet.evaporchain.com/faucet](https://testnet.evaporchain.com/faucet) |

## 1. Check Chain Status

Verify the testnet is running:

```bash
curl https://testnet.evaporchain.com/api/status
```

Response:

```json
{
  "chain_name": "EvaporChain",
  "version": "0.1.0",
  "block_height": 1234,
  "epoch": 1234,
  "active_objects": 15,
  "ghost_count": 3,
  "total_evaporated": 3,
  "peer_count": 4,
  "state_root": "a1b2c3...",
  "proving_enabled": false,
  "uptime_seconds": 86400
}
```

## 2. Get Testnet Tokens from the Faucet

Each address can claim 10,000 EVAP once per hour.

```bash
curl -X POST https://testnet.evaporchain.com/api/faucet \
  -H "Content-Type: application/json" \
  -d '{"address": 42}'
```

Response:

```json
{
  "success": true,
  "balance": 10000
}
```

Addresses on the testnet are identified by a single byte (0-255). The faucet UI at `/faucet` provides a visual interface.

## 3. Send Your First Transfer

Transfer 1,000 EVAP from address 42 to address 100:

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/transfer \
  -H "Content-Type: application/json" \
  -d '{
    "from": 42,
    "to": 100,
    "amount": 1000,
    "nonce": 0
  }'
```

Response:

```json
{
  "success": true,
  "message": "Transfer queued: 0x2a0000...0000 -> 0x640000...0000 amount=1000 (mempool=1)"
}
```

Transactions are queued in the mempool and included in the next block (~3 seconds).

## 4. Create a Decaying State Object

This is what makes EvaporChain unique. Create an object with energy that decays over time:

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/create-object \
  -H "Content-Type: application/json" \
  -d '{
    "creator": 42,
    "object_id": 150,
    "energy": 10000,
    "half_life": 50
  }'
```

This creates an object with 10,000 energy and a half-life of 50 epochs. After 50 epochs, energy drops to ~5,000. After 100 epochs, ~2,500. When it reaches zero, the object evaporates.

## 5. Watch It Decay

Check your object's current energy:

```bash
curl https://testnet.evaporchain.com/api/object/9600000000000000000000000000000000000000000000000000000000000000
```

The `current_energy` field shows real-time decayed energy. The `decay_percentage` shows how much has been lost.

## 6. Refresh to Prevent Evaporation

Deposit more energy to extend an object's lifetime:

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/refresh \
  -H "Content-Type: application/json" \
  -d '{
    "object_id": 150,
    "energy_deposit": 5000
  }'
```

## 7. Resurrect a Ghost

If an object has already evaporated, you can bring it back:

```bash
curl -X POST https://testnet.evaporchain.com/api/tx/resurrect \
  -H "Content-Type: application/json" \
  -d '{
    "object_id": 150,
    "energy_deposit": 10000
  }'
```

The ghost record proves the object once existed. Resurrection restores it with fresh energy.

## Using the TypeScript SDK

Install the SDK for a cleaner developer experience:

```bash
npm install @evaporchain/sdk
```

```typescript
import { EvaporChain } from "@evaporchain/sdk";

const chain = new EvaporChain("https://testnet.evaporchain.com");

// Check status
const status = await chain.getStatus();
console.log(`Block: ${status.block_height}, Epoch: ${status.epoch}`);

// Get faucet tokens
await chain.claimFaucet(42);

// Transfer
await chain.transfer(42, 100, 1000);

// Create a decaying object
await chain.createObject(42, 150, 10000, 50);

// Watch it decay in real-time
const stop = chain.watchObject(
  "9600000000000000000000000000000000000000000000000000000000000000",
  (obj) => {
    console.log(`Energy: ${obj.current_energy}/${obj.max_energy} (${obj.decay_percentage}% decayed)`);
    if (obj.state === "Ghost") {
      console.log("Object evaporated.");
      stop();
    }
  }
);
```

See the [SDK Guide](sdk.md) for the full API reference.

## Dashboard Pages

| Page | URL | Description |
|------|-----|-------------|
| Explorer | `/explorer` | Block explorer with live state objects, blocks, events, ghosts |
| Wallet | `/wallet` | Account management, transfers, activity history |
| NFTs | `/nft` | Mortal NFT marketplace with energy decay lifecycle |
| Tokens | `/tokens` | Decaying token deployment and management |
| Staking | `/staking` | Stake EVAP, earn decaying rewards |
| DAO | `/dao` | Governance proposals that evaporate |
| Faucet | `/faucet` | Claim free testnet EVAP |

## Next Steps

- [Concepts: Thermodynamic Decay](concepts/decay.md) — understand the core decay mechanism
- [Concepts: Ghost Records](concepts/ghosts.md) — what happens after evaporation
- [API Reference](api-reference.md) — every endpoint with curl examples
- [SDK Guide](sdk.md) — TypeScript SDK with code examples
- [EvaporScript](evaporscript.md) — write smart contracts with built-in decay
- [Smart Contract Templates](contracts.md) — pre-built contract patterns
- [NFT Guide](nft-guide.md) — mint and manage mortal NFTs
- [MCP Server](mcp.md) — connect AI agents to EvaporChain
