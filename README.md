# EvaporChain

**A blockchain where state expires by default and the entire chain history compresses into a single recursive proof.**

EvaporChain introduces thermodynamic state decay — every piece of on-chain state has an energy budget that depletes exponentially over time. Unused state evaporates automatically, leaving only a cryptographic nullifier proof. Combined with Nova recursive proof folding, the chain gets *lighter* over time, not heavier.

## Status: Testnet

- [x] Research corpus (1.2 MB across 5 phases)
- [x] Whitepaper (188 KB, 70 citations)
- [x] Core types and cryptographic layer (BLAKE3, ML-DSA, Verkle, MMR)
- [x] State layer (evaporation engine, refresh engine, dual commitment)
- [x] Execution engine (gas, fees, PID controller, signature verification)
- [x] Smart contracts (8 templates + rule engine)
- [x] EvaporScript (parser, compiler, VM with gas metering)
- [x] Consensus (Tendermint BFT — Propose/Prevote/Precommit/Commit, BLS aggregation, encrypted mempool, validator sets)
- [x] ZK proving (Nova recursive proof folding)
- [x] P2P networking (block propagation, tx gossip)
- [x] Full node with API, dashboard, faucet, and CLI
- [x] **5,500+ tests passing** (286 cross-crate integration tests across 48 modules)
- [ ] Public testnet deployment

## Run Locally

```bash
git clone https://github.com/ss1738/EvaporChain.git
cd EvaporChain

# Run tests
cargo test

# Start a node with API + dashboard
cargo run -p evaporchain-node -- --api --api-port 3000

# Open dashboard
open http://localhost:3000

# Get testnet tokens
open http://localhost:3000/faucet
```

## Connect to Public Testnet

Coming soon. Infrastructure is Terraform-ready (`deploy/terraform/modules/hetzner/`) — scalable to 50 validators on Hetzner EU. See [`scripts/deploy-testnet.sh`](scripts/deploy-testnet.sh) for manual deployment.

## Documentation

| Document | Description |
|----------|-------------|
| [Getting Started](docs/README.md) | API reference, curl examples, how to connect |
| [EvaporScript Guide](docs/EVAPORSCRIPT.md) | Scripting language syntax, types, examples |
| [Architecture](docs/ARCHITECTURE.md) | System diagram, crate descriptions, how decay/proofs/consensus work |
| [Whitepaper](research/) | Full technical specification |

## Technical Stack

| Layer | Implementation |
|-------|----------------|
| Language | Rust |
| Smart Contracts | 7 template contracts + EvaporScript (custom 44-opcode VM) |
| Consensus | Tendermint BFT with BLS aggregation + VRF leader election |
| Execution | SimpleExecutor with gas metering + PID fee controller |
| ZK Proofs | Nova IVC recursive proof folding |
| State | Energy-Verkle trie (active) + MMR nullifier accumulator (expired) + WAL crash recovery |
| Signatures | ML-DSA Dilithium3 (post-quantum) + BLS12-381 aggregation |
| Hashing | BLAKE3 |
| Networking | libp2p with gossipsub, mDNS, TLS 1.3, block sync, DA shard sampling |
| API | Axum HTTP with live dashboard |

## Crate Map

```
evaporchain-types       Core types (19 tx variants, objects, accounts, energy decay)
evaporchain-crypto      BLAKE3, BLS, ML-DSA, VRF, Verkle trie, Energy-Verkle, MMR
evaporchain-state       Evaporation engine, refresh engine, state DB, WAL, RocksDB
evaporchain-contracts   7 contract templates + rule engine + upgrades
evaporchain-script      EvaporScript parser → compiler (constant fold + DCE) → VM (44 ops)
evaporchain-execution   Block-STM parallel executor, gas, PID fees, privacy execution
evaporchain-consensus   Tendermint BFT, finality tracker, light client, state sync
evaporchain-proving     Nova IVC recursive proofs, privacy proofs, evaporation proofs
evaporchain-network     libp2p gossipsub, block sync, DA shard sampling
evaporchain-da          2D erasure coding, PoHA, namespace proofs, DA certificates
evaporchain-oracle      Decentralized oracle with BFT consensus + inclusion proofs
evaporchain-sharding    Dynamic shard assignment, cross-shard messaging, compaction
evaporchain-node        Full node binary + JSON-RPC API + dashboard + persistence
evaporchain-cli         CLI with genesis ceremony + keygen + monitoring
evaporchain-mcp         MCP server for AI agent interaction (26 tools, 13 resources, 6 prompts)
```

## Test Coverage

**5,500+ tests** across 71 crates — all passing. Includes 286 cross-crate integration tests covering the full pipeline (consensus → execution → DA → proving → contracts → frontier primitives → 48 substrate modules).

```bash
cargo test --workspace
# 5,500+ passed; 0 failed
```

## License

MIT
