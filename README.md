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
- [x] **Doctrine arc shipped 2026-05** — Lambda-Fold Nova IVC (sublinear light-client verification), Crooks-MEV refund pipeline (sandwich detection → settlement → stake-slash), Light-Cone Full DAG mode (multi-parent blocks, antichain finality, cross-validator commit-cert digest), Causal-CHSH cartel detection (real-Eth gate PASS), MultiAuditorVerifier k-of-n governance attestation, M2 Coq build-verified under Rocq 9.1.1
- [x] **Browser-side light client** — `evaporchain-light-client-wasm` (310 KB `.wasm` post-`wasm-pack`) verifies BFT BLS aggregate signatures + Verkle Pasta-curve Pedersen state proofs entirely in-browser; pure-Rust `bls12_381` backend (10 cross-backend interop tests vs. native `blst`)
- [x] **Death-is-final doctrine enforcement** (2026-05-08) — tombstoned validators jailed automatically (`enforce_validator_tombstones`), would-be block rewards redirected to refresh pool, eulogy trie + evaporation MMR + ghost-object counters surfaced at `/api/four_act`. Empirically validated live: val-3 + val-1 organically tombstoned; refresh pool absorbed redirected energy under §1.2 conservation
- [x] **Singh Pool AMM API** (2026-05-08) — decay-aware xy=k AMM with energy-tagged LP shares (mercenary-resistant: holders below `energy_floor` cannot withdraw). Full HTTP surface: `POST /api/pool/{create,mint,withdraw,swap_x_for_y,swap_y_for_x,reanchor}` + `GET /api/pool/{list,:id}`. `/api/swap` automatically routes through pools when one exists for the canonical pair-id ("EVAP-FLUX"); oracle-priced fallback otherwise. Pool state persists across restarts via bincode-encoded ledger in the data dir
- [x] **25,435+ test functions** across **147 workspace crates** (substrate primitives, consensus, execution, proving, DA, networking, frontier primitives, plus doctrine "press-claim" tests asserting headline properties as structural invariants)
- [ ] Public testnet deployment

## Run Locally

```bash
git clone https://github.com/ss1738/EvaporChain.git
cd EvaporChain

# Run tests
cargo test

# Start a node with API + dashboard
cargo run -p evaporchain-node -- --api --api-port 8080

# Open dashboard
open http://localhost:8080

# Get testnet tokens
open http://localhost:8080/faucet
```

## Connect to Public Testnet

Coming soon. Infrastructure is Terraform-ready (`deploy/terraform/modules/hetzner/`) — scalable to 50 validators on Hetzner EU. See [`scripts/deploy-testnet.sh`](scripts/deploy-testnet.sh) for manual deployment.

## Documentation

| Document | Description |
|----------|-------------|
| [Getting Started](docs/README.md) | API reference, curl examples, how to connect |
| [EvaporScript Guide](docs/EVAPORSCRIPT.md) | Scripting language syntax, types, examples |
| [Architecture](docs/ARCHITECTURE.md) | System diagram, crate descriptions, how decay/proofs/consensus work |
| [Cluster Deploy Runbook](docs/runbooks/cluster-deploy.md) | Stop-the-world deploy procedure for the 5-node Mac+Hetzner WAN cluster (launchd race, systemd surprise, recovery from forked state) |
| [Whitepaper](research/) | Full technical specification |

## Technical Stack

| Layer | Implementation |
|-------|----------------|
| Language | Rust |
| Smart Contracts | 8 template contracts + EvaporScript (custom 44-opcode VM) |
| Consensus | Tendermint BFT with BLS aggregation + VRF leader election |
| Execution | SimpleExecutor with gas metering + PID fee controller |
| ZK Proofs | Nova IVC recursive proof folding |
| State | Energy-Verkle trie (active) + MMR nullifier accumulator (expired) + WAL crash recovery |
| Signatures | ML-DSA Dilithium3 (post-quantum) + BLS12-381 aggregation |
| Hashing | BLAKE3 |
| Networking | libp2p with gossipsub, mDNS, TLS 1.3, block sync, DA shard sampling |
| API | Axum HTTP with live dashboard |

## Crate Map

147 workspace crates total. The core stack:

```
evaporchain-types       Core types (25 tx variants, objects, accounts, energy decay)
evaporchain-crypto      BLAKE3, BLS, ML-DSA, VRF, Verkle trie, Energy-Verkle, MMR
evaporchain-state       Evaporation engine, refresh engine, state DB, WAL, RocksDB
evaporchain-contracts   8 contract templates + rule engine + upgrades
evaporchain-script      EvaporScript parser → compiler (constant fold + DCE) → VM (44 ops)
evaporchain-execution   Block-STM parallel executor, gas, PID fees, privacy execution
evaporchain-consensus   Tendermint BFT, finality tracker, light client, state sync
evaporchain-proving     Nova IVC recursive proofs, privacy proofs, evaporation proofs
evaporchain-network     libp2p gossipsub, block sync, DA shard sampling, Sybil scoring
evaporchain-da          2D erasure coding, PoHA, namespace proofs, DA certificates
evaporchain-oracle      Decentralized oracle with BFT consensus + inclusion proofs
evaporchain-sharding    Dynamic shard assignment, cross-shard messaging, compaction
evaporchain-node        Full node binary + JSON-RPC API + dashboard + persistence
evaporchain-cli         CLI with genesis ceremony + keygen + monitoring
evaporchain-mcp         MCP server for AI agent interaction (26 tools, 13 resources, 6 prompts)
```

Doctrine / frontier primitives:

```
evaporchain-light-cone        Causal-set partial-order DAG (Sorkin/Pratt) + antichain
                              primitives + Phase 4.4 commit-cert digest
evaporchain-lambda-fold       Lambda-Fold Nova IVC accumulator (sublinear light-client
                              verification)
evaporchain-mev-detect        Sandwich-attack detector (Crooks-MEV refund pipeline)
evaporchain-crooks-mev-refund Refund-tx settlement substrate
evaporchain-causal-chsh       Bell-CHSH cartel detector (frontier theorem #1, gate
                              PASS on real Ethereum data 2026-05-04)
evaporchain-llsa              Lambda-Locked Self-Amendment (k-of-n MultiAuditorVerifier
                              + Coq-build-verified invariant preservation)
evaporchain-mcc               Maximum-Caliber Consensus (Jaynes Lagrangian fork choice)
evaporchain-cfm               Crooks-Singh Fee Market (closed-form fee equilibrium)
evaporchain-decay-lamport     Decay-Lamport Time (energy-driven logical clock)
evaporchain-fee-controller    Singh-Lyapunov PID fee controller
evaporchain-entropic-slashing Sanov / large-deviation slashing magnitude
evaporchain-singh-attractor   Singh attractor consensus primitive
evaporchain-bell-beacon       Device-independent randomness beacon
evaporchain-evap-fork-cert    Evaporated-fork certificates
evaporchain-mortis            Four-act narrative state machine
evaporchain-autopoietic       Self-maintaining system primitive
evaporchain-energy-kernel     Coq-verified canonical energy_at_epoch
... and ~120 supporting crates                  (full list: `ls crates/`)
```

## Test Coverage

**12,500+ test functions** across 147 workspace crates. Coverage spans the core pipeline (consensus → execution → DA → proving → contracts → frontier primitives) plus substrate-module tests and top-level "press-claim" tests that assert each crate's doctrine headline as a structural invariant — so the press claim breaks loudly if the implementation drifts.

```bash
cargo test --workspace
```

Note: workspace builds and tests are run on the M4 Mini cluster via SSH (build memory + parallelism). The MacBook is for editing only.

## License

MIT
