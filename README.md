# EvaporChain

**A blockchain where state expires by default and the entire chain history compresses into a single recursive proof.**

EvaporChain introduces thermodynamic state decay — every piece of on-chain state has an energy budget that depletes exponentially over time. Unused state evaporates automatically, leaving only a cryptographic nullifier proof. Combined with Nova recursive proof folding, the chain gets *lighter* over time, not heavier.

## Status: Mainnet Sprint In Flight · Public Devnet Live

Public devnet running 24/7 at **`http://89.167.52.40:8099`** (Hetzner Helsinki, single-node, chain-id `evaporchain-testnet-1`). Mainnet launch sprint started 2026-06-01 — see [`docs/MAINNET_LAUNCH.md`](docs/MAINNET_LAUNCH.md) for the operator-facing strict-mode launch path.

- [x] Research corpus (1.2 MB across 5 phases)
- [x] Whitepaper (188 KB, 70 citations)
- [x] Core types and cryptographic layer (BLAKE3, ML-DSA, Verkle, MMR) — typed chain-id constants pinned at `evaporchain_types::chain_ids`
- [x] State layer (evaporation engine, refresh engine, dual commitment)
- [x] Execution engine (gas, fees, PID controller, signature verification)
- [x] Smart contracts — **30 first-class catalogue templates** (Marketplace + NFT + Wallet UX + Consumer + Cultural + Paradigm + Governance lanes) backed by **46 .es reference contracts** + rule engine
- [x] EvaporScript V2 (parser, compiler, 44-opcode VM with gas metering, `<<` / `>>` / compound-assign operators)
- [x] Consensus (Tendermint BFT — Propose/Prevote/Precommit/Commit, BLS aggregation, encrypted mempool, validator sets)
- [x] ZK proving (Nova recursive proof folding, real `--prove` chain proofs verified on 3-Mini cluster)
- [x] P2P networking (chain-id-scoped gossipsub, block propagation, tx gossip, Sybil scoring)
- [x] Full node with API, dashboard, faucet, and CLI; `--mainnet` strict-mode pre-flight (11 refusal checks)
- [x] **Doctrine arc shipped 2026-05** — Lambda-Fold Nova IVC (sublinear light-client verification), Crooks-MEV refund pipeline (sandwich detection → settlement → stake-slash), Light-Cone Full DAG mode (multi-parent blocks, antichain finality, cross-validator commit-cert digest), Causal-CHSH cartel detection (real-Eth gate PASS), MultiAuditorVerifier k-of-n governance attestation, M2 Coq build-verified under Rocq 9.1.1
- [x] **Browser-side light client** — `evaporchain-light-client-wasm` (310 KB `.wasm` post-`wasm-pack`) verifies BFT BLS aggregate signatures + Verkle Pasta-curve Pedersen state proofs entirely in-browser; pure-Rust `bls12_381` backend (10 cross-backend interop tests vs. native `blst`)
- [x] **Death-is-final doctrine enforcement** (2026-05-08) — tombstoned validators jailed automatically (`enforce_validator_tombstones`), would-be block rewards redirected to refresh pool, eulogy trie + evaporation MMR + ghost-object counters surfaced at `/api/four_act`. Empirically validated live: val-3 + val-1 organically tombstoned; refresh pool absorbed redirected energy under §1.2 conservation
- [x] **Singh Pool AMM API** (2026-05-08) — decay-aware xy=k AMM with energy-tagged LP shares (mercenary-resistant: holders below `energy_floor` cannot withdraw). Full HTTP surface + auto-routing on `/api/swap`
- [x] **AUDIT_2026_05_17 closure trail** (closed 2026-05-28) — 9 CRITICAL + 14 HIGH + 25 MEDIUM + 13 LOW findings: Verkle DST drift (CR-1/2/3), VRF chain-id-scoping (H-1), address-derivation DST (H-2), MMR structural validation (H-3), non-validator BLS PoP (H-4), DA-cert forgery class (Q1-Q3/Q8), Tendermint strict-quorum (Q4), Nova IVC running-total decay (L0-A). Per-finding trail at [`docs/AUDIT_SCOPE.md`](docs/AUDIT_SCOPE.md) §6.2.
- [x] **#469 P0 launch-blocker remediation** (closed 2026-05-28) — PRIV-001/002 (shielded-tx v1-gating), DA-001 (`verify_signatures_bound`), VM-001 (DecayingToken `refresh_balance` checked_add), API-001 (wallet master key fails closed), ECON-001 (slash redistribute conservation). See [`docs/AUDIT_SCOPE.md`](docs/AUDIT_SCOPE.md) §6.3.
- [x] **Chain-as-keeper escrow triplet** (2026-05-31 / 2026-06-01) — DEADMAN_SWITCH + SUBSCRIPTION_SERVICE + OPEN_BOUNTY in the Marketplace lane: the doctrine that lapse detection, deadline enforcement, and refund/release closure are performed by the chain runtime itself, no off-chain keeper needed. Each ships with .es contract + cargo pilot + typed dApp client + visceral simulator UI.
- [x] **25,435+ test functions** across **141 active workspace crates** (163 directories incl. 2 excluded WASM crates) + Coq (5 proofs zero-Admitted under Rocq 9.1.1) + TLA+ (5 specs bounded model-check clean) + 300+ TypeScript tests for dApp typed clients
- [ ] **Mainnet launch** — gated on external security audit (T0.12), tokenomics ceremony (28 open Q's in [`docs/TOKENOMICS.md`](docs/TOKENOMICS.md)), multi-validator soak (T0.6), and `MAINNET_COORDINATOR_PK_BYTES` bake-in. Target Q4 2026 / Q1 2027 per [`docs/AUDIT_SCOPE.md`](docs/AUDIT_SCOPE.md) §10.

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

## Connect to Public Devnet

A single-node devnet runs 24/7 at **`http://89.167.52.40:8099`** (Hetzner Helsinki, chain-id `evaporchain-testnet-1`). Useful endpoints:

```bash
curl http://89.167.52.40:8099/api/chain              # chain identity + version
curl http://89.167.52.40:8099/api/network/health     # block height, finality, peers
curl http://89.167.52.40:8099/api/blocks?limit=10    # latest 10 blocks
curl http://89.167.52.40:8099/api/four_act           # death ledger (eulogies + tombstones)
```

The faucet (`POST /api/faucet`) is admin-gated; reach out to `security@evaporchain.io` for testnet tokens. Multi-validator infrastructure is Terraform-ready (`deploy/terraform/modules/hetzner/`) — scalable to 50 validators on Hetzner EU. See [`scripts/deploy-testnet.sh`](scripts/deploy-testnet.sh) for manual deployment.

## Documentation

### Read-this-first

| Document | Audience | Description |
|----------|----------|-------------|
| [docs/SPEC.md](docs/SPEC.md) | Everyone | One-page skim summary |
| [docs/MAINNET_LAUNCH.md](docs/MAINNET_LAUNCH.md) | Operators | `--mainnet` strict-mode launch playbook (11 pre-flight checks + governance-flag defaults + pre-launch checklist) |
| [docs/AUDIT_SCOPE.md](docs/AUDIT_SCOPE.md) | Auditors | Engagement scope, priority tiers, per-finding closure trail (AUDIT_2026_05_17 + #469 P0 pack), recommended firms |
| [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md) | Auditors / security researchers | Adversary model, attack surface, known gaps with status |
| [docs/RUN_A_NODE.md](docs/RUN_A_NODE.md) | Validators | Local devnet, testnet, mainnet launch commands |

### Reference

| Document | Description |
|----------|-------------|
| [Getting Started](docs/README.md) | API reference, curl examples, how to connect |
| [EvaporScript Guide](docs/EVAPORSCRIPT.md) | Scripting language syntax, types, examples |
| [Architecture](docs/ARCHITECTURE.md) | System diagram, crate descriptions, how decay/proofs/consensus work |
| [Cryptographic Spec](docs/CRYPTO_SPEC.md) | Poseidon, Verkle, MMR, BLS, ML-DSA, Nova details |
| [Tokenomics (ceremony in progress)](docs/TOKENOMICS.md) | 28 open Q's pending tokenomics advisory |
| [Genesis Ceremony](docs/GENESIS_CEREMONY.md) | Protocol-level genesis: committee selection, validator set, key derivation |
| [Validator Onboarding](docs/VALIDATOR_ONBOARDING.md) | Post-launch joining + runbooks |
| [Bug Bounty (scoping draft)](docs/BUG_BOUNTY.md) | Pre-mainnet bounty scope; not yet active |
| [Cluster Deploy Runbook](docs/runbooks/cluster-deploy.md) | Stop-the-world deploy procedure for the 5-node Mac+Hetzner WAN cluster |
| [Whitepaper](research/) | Full technical specification |

## Technical Stack

| Layer | Implementation |
|-------|----------------|
| Language | Rust |
| Smart Contracts | 30 first-class catalogue templates + 46 .es reference contracts + EvaporScript V2 (custom 44-opcode VM, `<<` / `>>` / compound-assign) |
| Consensus | Tendermint BFT with BLS aggregation + VRF leader election |
| Execution | SimpleExecutor with gas metering + PID fee controller |
| ZK Proofs | Nova IVC recursive proof folding |
| State | Energy-Verkle trie (active) + MMR nullifier accumulator (expired) + WAL crash recovery |
| Signatures | ML-DSA Dilithium3 (post-quantum) + BLS12-381 aggregation |
| Hashing | BLAKE3 |
| Networking | libp2p with gossipsub, mDNS, TLS 1.3, block sync, DA shard sampling |
| API | Axum HTTP with live dashboard |

## Crate Map

163 crate directories (141 active workspace members + 2 excluded WASM crates). The core stack:

```
evaporchain-types       Core types (25 tx variants, objects, accounts, energy decay, chain_ids consts)
evaporchain-crypto      BLAKE3, BLS, ML-DSA, VRF, Verkle trie, Energy-Verkle, MMR
evaporchain-state       Evaporation engine, refresh engine, state DB, WAL, RocksDB
evaporchain-contracts   Template contracts + rule engine + upgrades (30 catalogue templates wired)
evaporchain-script      EvaporScript V2 parser → compiler (constant fold + DCE) → VM (44 ops)
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

**25,435+ test functions** across 141 active workspace crates + 5 Coq proofs (zero-Admitted under Rocq 9.1.1) + 5 TLA+ specifications (bounded model-check clean) + 300+ TypeScript tests for dApp typed clients. Coverage spans the core pipeline (consensus → execution → DA → proving → contracts → frontier primitives) plus substrate-module tests, the catalogue anti-regression gate (`every_catalogue_default_binds`), and top-level "press-claim" tests that assert each crate's doctrine headline as a structural invariant — so the press claim breaks loudly if the implementation drifts.

```bash
cargo test --workspace
```

Note: workspace builds and tests are run on the M4 Mini cluster via SSH (build memory + parallelism). The MacBook is for editing only.

## License

MIT
