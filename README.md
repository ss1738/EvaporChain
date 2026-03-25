# EvaporChain

**A blockchain where state expires by default and the entire chain history compresses into a single recursive proof.**

EvaporChain introduces thermodynamic state decay — every piece of on-chain state has an energy budget that depletes exponentially over time. Unused state evaporates automatically, leaving only a cryptographic nullifier proof. Combined with Nova recursive proof folding, the chain gets *lighter* over time, not heavier.

## Status: Testnet

- [x] Research corpus (1.2 MB across 5 phases)
- [x] Whitepaper (188 KB, 70 citations)
- [x] Core types and cryptographic layer (BLAKE3, ML-DSA, Verkle, MMR)
- [x] State layer (evaporation engine, refresh engine, dual commitment)
- [x] Execution engine (gas, fees, PID controller, signature verification)
- [x] Smart contracts (6 templates + rule engine)
- [x] EvaporScript (parser, compiler, VM with gas metering)
- [x] Consensus (rotating leader, encrypted mempool, validator sets)
- [x] ZK proving (Nova recursive proof folding)
- [x] P2P networking (block propagation, tx gossip)
- [x] Full node with API, dashboard, faucet, and CLI
- [x] **298 tests passing**
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

Coming soon. The testnet will be deployed on 4 Hetzner nodes. See [`scripts/deploy-testnet.sh`](scripts/deploy-testnet.sh) for deployment instructions.

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
| Smart Contracts | 6 template contracts + EvaporScript (custom scripting language) |
| Consensus | Rotating leader with stake-weighted selection |
| Execution | SimpleExecutor with gas metering + PID fee controller |
| ZK Proofs | Nova IVC recursive proof folding |
| State | Verkle trie (active) + MMR nullifier accumulator (expired) |
| Signatures | ML-DSA (post-quantum) |
| Hashing | BLAKE3 |
| Networking | Custom P2P with block propagation and tx gossip |
| API | Axum HTTP with live dashboard |

## Crate Map

```
evaporchain-types       Core types (transactions, objects, accounts, energy decay)
evaporchain-crypto      BLAKE3, ML-DSA signatures, Verkle trie, MMR
evaporchain-state       Evaporation engine, refresh engine, state DB
evaporchain-contracts   6 contract templates + rule engine
evaporchain-script      EvaporScript parser → compiler → VM
evaporchain-execution   Transaction executor, gas, fees
evaporchain-consensus   Rotating leader, encrypted mempool, validator sets
evaporchain-proving     Nova recursive ZK proof folding
evaporchain-network     P2P block/tx propagation
evaporchain-node        Full node binary + API + dashboard + faucet
evaporchain-cli         Command-line interface
```

## Test Coverage

**298 tests** across 12 crates — all passing.

```bash
cargo test
# test result: ok. 298 passed; 0 failed
```

## License

MIT
