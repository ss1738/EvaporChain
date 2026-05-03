# Run an EvaporChain Node

## Quick Start (Local Devnet)

Run a 4-validator testnet on your local machine:

```bash
# Build
cargo build --release -p evaporchain-node

# Launch 4 validators with auto-generated transactions
./scripts/launch-testnet.sh --demo
```

Dashboards will be available at:
- Validator 1: http://localhost:8080
- Validator 2: http://localhost:8081
- Validator 3: http://localhost:8082
- Validator 4: http://localhost:8083

## Single Node (Development)

```bash
cargo run --release -p evaporchain-node -- --api --demo
```

This starts a single-node chain with MockConsensus. Good for development and testing.

## Tendermint BFT Node

Run a validator participating in multi-node BFT consensus:

```bash
cargo run --release -p evaporchain-node -- \
    --tendermint \
    --network \
    --api \
    --demo \
    --validator-id 1 \
    --validators 4 \
    --node-id node-1 \
    --port 9000 \
    --api-port 8080 \
    --data-dir ./data/validator-1 \
    --startup-delay 5000 \
    --bootstrap /ip4/37.27.1.1/tcp/9001
```

### Command-line Flags

| Flag | Description | Default |
|------|-------------|---------|
| `--tendermint` | Enable Tendermint BFT consensus | off (MockConsensus) |
| `--network` | Enable P2P networking (libp2p) | off |
| `--api` | Enable HTTP API + dashboard | off |
| `--demo` | Auto-generate transactions | off |
| `--prove` | Enable real Nova IVC proving (release-only; debug builds use MockProver) | off |
| `--mainnet` | Bootstrap from `genesis-mainnet.json` with mainnet defaults (block interval, gas limits, coordinator pk) | off |
| `--genesis-config PATH` | Bootstrap from a JSON genesis config (supersedes built-in defaults) | none |
| `--fast-sync` | On startup, fetch the latest finalized snapshot from peers and apply it before joining consensus; the node then catches up over normal sync. Verified end-to-end on the 3-Mini cluster 2026-05-02 | off |
| `--block-gas-limit N` | Per-block gas ceiling | 500_000 |
| `--high-throughput` | Preset: 10M gas, 200ms blocks | off |
| `--validator-id N` | This node's validator ID | 1 |
| `--validators N` | Total validators in the set | 4 |
| `--stake N` | Stake per validator | 1000 |
| `--port N` | P2P listen port (0 = random) | 0 |
| `--api-port N` | HTTP API port | 8080 |
| `--node-id NAME` | Display name for this node | "node" |
| `--data-dir PATH` | Persistent data directory | ./evaporchain-data |
| `--interval MS` | Block interval in milliseconds (devnet default) — `--mainnet` overrides to 2000 ms per `genesis-mainnet.json` | 1000 |
| `--startup-delay MS` | Wait for peer discovery | 0 (5000 if --network) |
| `--bootstrap ADDR` | Bootstrap peer multiaddr (repeatable) | none |

**Validator key passphrase:** the encrypted at-rest BLS key (`bls_key.bin`, EVPL format) is unlocked via `EVAPORCHAIN_VALIDATOR_KEY_PASS_FILE` (path to a file containing the passphrase). Avoid `EVAPORCHAIN_VALIDATOR_KEY_PASS` in environment variables — it is exposed via `/proc/<pid>/environ`. See `runbooks/validator-onboarding.md` for the full key-rotation runbook.

### Data Persistence

All state is persisted in the `--data-dir`:
- `state/` — RocksDB database (accounts, objects, ghosts)
- `chain/` — Block history, consensus metadata, DeFi stores

The node resumes from persistent state on restart.

## CLI

```bash
# Install
cargo install --path crates/evaporchain-cli

# Usage
evaporchain status
evaporchain blocks --limit 5
evaporchain accounts
evaporchain objects
evaporchain transfer --from 1 --to 2 --amount 500
evaporchain faucet --address 0xYOUR_ADDRESS
evaporchain consensus
evaporchain devnet --validators 4 --demo
```

## TypeScript SDK

```bash
cd sdk/typescript
npm install
npm run build
```

```typescript
import { EvaporClient } from '@evaporchain/sdk';

const client = new EvaporClient('https://testnet.evaporchain.com');
const status = await client.getStatus();
console.log(`Block height: ${status.block_height}`);

// Get testnet tokens
await client.faucet('0xYOUR_ADDRESS');

// Transfer
await client.transfer({ from: '0x...', to: '0x...', amount: 100 });

// Mint an NFT (EVR-721)
await client.mintNft({
  name: 'My Mortal NFT',
  collection: 'Test',
  energy: 10000,
  half_life: 50,
  owner: '0x...',
});
```

## Architecture

```
┌─────────────────────────────────────────────────┐
│                   API Server                     │
│              (Axum HTTP + Dashboard)             │
├─────────────────────────────────────────────────┤
│            Tendermint BFT Consensus              │
│  Propose → Prevote → Precommit → Commit          │
├──────────────────┬──────────────────────────────┤
│   Execution      │       State Layer             │
│ (Gas, Fees,      │  (RocksDB, Decay Engine,      │
│  Signatures)     │   Verkle Trie, MMR)           │
├──────────────────┴──────────────────────────────┤
│              P2P Network (libp2p)                 │
│  GossipSub + mDNS + Block Sync + Consensus Msgs  │
├─────────────────────────────────────────────────┤
│           Cryptography Layer                      │
│  ML-DSA (FIPS 204) · BLAKE3 · Poseidon · Verkle  │
└─────────────────────────────────────────────────┘
```

## Requirements

- Rust 1.75+
- RocksDB (bundled via `rocksdb` crate)
- 2GB RAM minimum per validator
- 10GB disk space
