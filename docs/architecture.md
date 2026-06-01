# EvaporChain — Technical Architecture

## System Diagram

```
                        ┌─────────────────────────────────────────┐
                        │              EvaporChain Node            │
                        │                                         │
  Clients ──────────────┤  ┌───────────┐    ┌──────────────────┐  │
  (curl, dashboard,     │  │  API +    │    │   Consensus      │  │
   faucet, wallets)     │  │  Dashboard │    │   (Tendermint    │  │
                        │  │  (Axum)   │    │    BFT)           │  │
                        │  └─────┬─────┘    └────────┬─────────┘  │
                        │        │                   │             │
                        │        ▼                   ▼             │
                        │  ┌─────────────────────────────────┐    │
                        │  │        Execution Engine          │    │
                        │  │  ┌───────────┐ ┌──────────────┐ │    │
                        │  │  │ Template  │ │ EvaporScript │ │    │
                        │  │  │ Contracts │ │ VM           │ │    │
                        │  │  └───────────┘ └──────────────┘ │    │
                        │  └──────────────┬──────────────────┘    │
                        │                 │                        │
                        │                 ▼                        │
                        │  ┌──────────────────────────────────┐   │
                        │  │          State Layer              │   │
                        │  │  ┌────────────┐ ┌─────────────┐  │   │
                        │  │  │ Evaporation│ │   Refresh    │  │   │
                        │  │  │ Engine     │ │   Engine     │  │   │
                        │  │  └────────────┘ └─────────────┘  │   │
                        │  │  ┌────────────┐ ┌─────────────┐  │   │
                        │  │  │ Verkle     │ │ MMR         │  │   │
                        │  │  │ Trie       │ │ Nullifiers  │  │   │
                        │  │  └────────────┘ └─────────────┘  │   │
                        │  └──────────────────────────────────┘   │
                        │                 │                        │
                        │                 ▼                        │
                        │  ┌──────────────────────────────────┐   │
                        │  │     Cryptographic Layer           │   │
                        │  │  BLAKE3 · ML-DSA · Verkle · MMR  │   │
                        │  │  Nova recursive proof folding     │   │
                        │  └──────────────────────────────────┘   │
                        │                 │                        │
                        │                 ▼                        │
                        │  ┌──────────────────────────────────┐   │
                        │  │     Network Layer (P2P)           │   │
                        │  │  Block propagation · Tx gossip    │   │
                        │  └──────────────────────────────────┘   │
                        └─────────────────────────────────────────┘
```

## Crate Descriptions

### evaporchain-types
Core domain types shared across all crates. Defines `StateObject`, `Account`, `Block`, `Transaction` (25 tx variants), `GhostRecord`, `DualCommitment`, the energy decay formula, and the typed `chain_ids` constants (`MAINNET` / `TESTNET` / `DEVNET`) bound into BLS signing message + VRF leader input + paymaster sponsorship payload + gossipsub topic namespace. The canonical type definitions live here — no business logic, just data structures and serialization.

### evaporchain-consensus-types
Consensus-types extracted from `evaporchain-consensus` so the browser-side light-client SDK can depend on them without transitively pulling in RocksDB. Made the WASM light client viable.

### evaporchain-crypto
Cryptographic primitives: BLAKE3 hashing, ML-DSA post-quantum digital signatures (key generation, signing, verification), Verkle trie implementation for state commitments, and Merkle Mountain Range (MMR) for nullifier accumulation. Provides the `Signer`/`Verifier` traits used by the execution layer.

### evaporchain-state
State management layer. Contains the `StateDB` trait (with `InMemoryStateDB` implementation), the **Evaporation Engine** (processes energy decay each epoch, transitions objects through Active → Grace → Ghost lifecycle), and the **Refresh Engine** (handles energy deposits and ghost resurrection). This is where thermodynamic decay is enforced.

### evaporchain-contracts
Template-based smart contract system. The original 8 hard-coded templates (DecayingToken, MortalNFT, ThermodynamicEscrow, DecayingAuction, StakingPool, DAOVote, DecayingDAO, TemporalContract) coexist with the **30 first-class catalogue templates** registered via `evaporchain-app-templates` (see below). The rule engine drives custom behavior (triggers, conditions, actions). Each contract instance has its own energy and half-life — contracts themselves evaporate when unused.

### evaporchain-app-templates + app-templates-{materialise, engine, fees, bind, receipt, eventlog, deploy}
The catalogue pipeline. `evaporchain-app-templates` is the registry: stable u32 class IDs in `0x0001_0000..=0x0001_FFFF`, one `TemplateDescriptor` per registered primitive (currently 30: NFT lane × 6, Marketplace × 9, Wallet UX × 4, Consumer × 4, Cultural × 1, Paradigm × 4, Governance × 1, including the chain-as-keeper triplet — DEADMAN_SWITCH + SUBSCRIPTION_SERVICE + OPEN_BOUNTY). The `-materialise` crate parses `init_calldata` JSON into a `TypedInit` envelope; `-engine` dispatches that envelope to per-template `init_*.rs` modules (one per registered template); `-fees` computes the deploy-fee oracle quote; `-bind` enforces pre-deploy invariants (e.g. Bell-Oracle `threshold_milli >= 2000`); `-deploy` declares the required-keys table per class; `-receipt` and `-eventlog` close the deploy round-trip. Anti-regression: `every_catalogue_default_binds` walks the full catalogue at test-time and asserts every descriptor's default params bind cleanly.

### evaporchain-script
The EvaporScript scripting language (V2). A non-Turing-complete language with three components: **Parser** (lexer + recursive descent parser → AST; V2 added `<<`, `>>`, `*=`, `/=`, and paren-wrapped LHS in `if` conditions), **Compiler** (AST → stack-based bytecode with method table, 44 opcodes incl. `Op::Shl` / `Op::Shr`), and **VM** (executes bytecode with gas metering, built-in functions, state management; shift opcodes are mul-tier `GAS_SHIFT = 5`). Includes the `ScriptEngine` for deploying and managing script contracts with full lifecycle hook support (`on_grace` / `on_refresh` / `on_evaporate`).

### evaporchain-execution
Transaction execution engine. The `SimpleExecutor` processes blocks sequentially: verifies signatures (ML-DSA), estimates gas, dispatches to the appropriate handler (transfer, create object, refresh, deploy/call contract, deploy/call script), runs evaporation at block end, and computes fees via a PID controller. Orchestrates both the template `ContractEngine` and the `ScriptEngine`.

### evaporchain-consensus
Tendermint BFT consensus with full Propose→Prevote→Precommit→Commit state machine. Validators produce blocks via stake-weighted leader election with VRF-seeded randomness from the beacon. BLS12-381 aggregate signatures for vote attestation, equivocation detection with slashing, exponential timeout escalation with per-height jitter. Includes an encrypted mempool (commit-reveal scheme to prevent MEV front-running), cross-chain bridge verifier, epoch transition manager with bonding periods, and DA certificate attestation. `MockConsensus` provides a simplified single-node mode for development.

### evaporchain-proving
Zero-knowledge proof system based on Nova recursive proof folding. Each block's state transition (transfers, energy changes, state root updates) is expressed as an R1CS circuit and folded into a running proof. After N blocks, the proof is constant-size regardless of N — a new node can verify the entire chain history by checking a single proof.

### evaporchain-network
P2P networking layer built on libp2p. GossipSub for transaction/block/consensus message propagation, request-response for block sync and DA shard sampling. Per-peer rate limiting (500 msgs/10s window), mDNS local discovery + bootstrap peer WAN connectivity, TLS 1.3 or Noise transport, and peer allowlist support. Includes block cache for serving sync requests and shard cache for DA light client queries.

### evaporchain-oracle
Oracle data ingestion service. Publishes real-world data feeds (sensor data, energy prices) as on-chain objects with energy and half-life. Authenticated via bearer token.

### evaporchain-sharding
Experimental sharding module for future horizontal scaling.

### evaporchain-node
Full node implementation. Combines all layers into a runnable binary with CLI arguments, an Axum-based HTTP API with a live dashboard, faucet, block explorer endpoints, and a transaction submission interface. Supports multi-node testnet deployment with configurable validator IDs and ports.

### evaporchain-cli
Command-line interface for interacting with a running node. Supports transfers, object creation, refresh, account queries, and block inspection via the HTTP API. Also hosts genesis (`genesis init`/`validate`/`show`), validator keygen (`keygen` produces BLS+ML-DSA+VRF bundle, encrypted at rest in EVPL format), and the validator-onboarding contribution-envelope flow.

### evaporchain-mcp
Model Context Protocol server. Surfaces a node's RPC, block explorer, faucet, and contract-deploy paths to MCP-compatible clients (Claude, IDEs).

### evaporchain-fee-controller
Standalone PID fee controller crate. Tracks recent block gas utilization, adjusts `base_fee_floor` and `base_fee_ceiling` against a configurable `target_gas_utilization`, and feeds the result back into `execute_transfer` / script-call paths.

### evaporchain-da
Data availability layer. 2D Reed-Solomon erasure coding over BLS12-381 field, namespaced Merkle tree (NMT, namespace 0 reserved), light-client sampling, BLS supermajority DA certificates. Block-production path uses `build_block_da_inputs(txs)` so the `data_root` produced at proposal time matches the one served at verify time. DA-cert forgery class (Q1-Q3/Q8) closed via `verify_signatures_bound(registered)` — dedup by `validator_id`, bind `att.public_key == registered_key`, count registered stake (not attacker-supplied), strict `> 2T/3`. The empty-block `data_root` edge case from earlier audit backlog is now closed (see `docs/THREAT_MODEL.md` §6.1).

### evaporchain-eth-bridge / evaporchain-nova-bridge / evaporchain-paymaster
Ethereum bridge (state-proof bridging both ways), the T0.10 Path A Nova IVC → Groth16-on-BN254 verifier for L1 settlement, and the UserOp paymaster (multi-token-gas Option B) that lets users pay fees in any chain token while validators receive native EVAP under the hood.

### evaporchain-light-client-wasm / evaporchain-crypto-wasm
Browser-side WASM bindings. The light-client WASM (310 KB post-`wasm-pack`) verifies BFT BLS aggregate signatures + Verkle Pasta-curve Pedersen state proofs entirely in-browser via the pure-Rust `bls12_381` backend (10 cross-backend interop tests vs. native `blst`). The crypto WASM exposes BLAKE3 / ML-DSA / Verkle / MMR to JavaScript callers.

## Substrate crates (~120 crates implementing the Tier-1 invention stack and launch-dApp lanes)

These crates extend the core node with the protocol's novel primitives. They
are independent crates that compose against the core types/state/consensus
contracts. Tests live with each crate (`cargo test -p <name>`). Names below are
grouped by lane.

### Invention-stack primitives (Tier-1 doctrine)

| Crate | Role |
|---|---|
| `evaporchain-light-cone` | Light-Cone Ledger DAG; pruning wired into consensus tick (every 100 blocks, 1000-epoch retention) |
| `evaporchain-bell-beacon` | Bell-Certified Beacon — entropy source with non-classical certification |
| `evaporchain-singh-attractor` | Singh Attractor Consensus — convergence-by-attractor variant of BFT-style fork choice |
| `evaporchain-evap-fork-cert` | Evaporated-Fork Certificates — proves a fork's blocks are evaporated and unreachable |
| `evaporchain-ib-validators` | Immune Validator Set — adaptive admission against poisoned peer surfaces |
| `evaporchain-mera` | MERA gate (Tier-2; week-25+ window) |
| `evaporchain-causal-cone`, `evaporchain-cone-bridge`, `evaporchain-cmu-gate` | Light-cone bridging + CMU gate |
| `evaporchain-causal-chsh` | **Causal-CHSH** — Bell-style cartel-detection bound on LightCone causal sets. EvaporChain's first 100% original frontier theorem (gate PASS 2026-05-04, see `INVENTION_STACK.md §A1.10`). Tier-0 supporting; live in `TendermintConsensus.cartel_alarm` (Lane O.8.1) |
| `evaporchain-cslc` | Causal-state ledger control |
| `evaporchain-singh-resonance`, `evaporchain-singh-attractor` | Singh resonance + attractor |
| `evaporchain-tur-liveness`, `evaporchain-tropical` | Turing-style liveness + tropical algebra ledger |

### Smart-contract paradigm trifecta (substrate)

| Crate | Role |
|---|---|
| `evaporchain-sgb` | Singh-Girard !/? linear-logic types |
| `evaporchain-sbav` | Singh-Bennett reversible VM (Landauer literal: only DECAY exports entropy) |
| `evaporchain-ssm` | Singh Strategy Machines (Hyland-Ong arenas + AJM innocent strategies) |

### Marketplace + cultural-launch lanes

| Crate | Role |
|---|---|
| `evaporchain-sddc` | Skill-Decay Demand Curve marketplace base |
| `evaporchain-sfsv` | Singh future-self vault (first launch dApp on SDDC) |
| `evaporchain-shlm` | Skill half-life market (B2B wedge) |
| `evaporchain-singh-sabi` | Singh-Sabi patina NFTs (first NFT lane) |
| `evaporchain-singh-migrant` | Wanderwrits NFTs with novel-wallet refund + farm-attack guards |
| `evaporchain-mnemochain` | Anki on-chain + FSRS forgetting curves |
| `evaporchain-gallery-forgets` | The Gallery That Forgets (cultural-launch wedge; Mayfly NFTs) |
| `evaporchain-singh-triage` | Singh triage (wallet UX lane) |
| `evaporchain-childkey` | Childkey (consumer launch) |
| `evaporchain-scl` | Capability-lease primitive |

### Energy / decay / refresh substrate

| Crate | Role |
|---|---|
| `evaporchain-energy-kernel`, `evaporchain-allen-decay`, `evaporchain-decay-forget`, `evaporchain-decay-lamport` | Decay & forget kernels |
| `evaporchain-refresh-market`, `evaporchain-refresh-patronage` | Refresh markets |
| `evaporchain-demurrage`, `evaporchain-tombstone`, `evaporchain-mortis` | Demurrage + tombstoning |
| `evaporchain-eb-fs`, `evaporchain-eg-fss`, `evaporchain-efh`, `evaporchain-epv`, `evaporchain-etlp`, `evaporchain-fee-controller`, `evaporchain-padic`, `evaporchain-pnt`, `evaporchain-prp`, `evaporchain-rg-phase-map` | Fee, evaporation, and protocol-physics substrates |
| `evaporchain-self-annealing`, `evaporchain-autopoietic` | Self-annealing + autopoietic state |

### Consensus / mempool / DA substrate

| Crate | Role |
|---|---|
| `evaporchain-antichain-mempool` | Antichain-aware mempool |
| `evaporchain-braid-sequencer`, `evaporchain-modular-beacon`, `evaporchain-sentinel`, `evaporchain-dsn` | Sequencing + beacon + sentinel |
| `evaporchain-hbct`, `evaporchain-hbct-elexon`, `evaporchain-hlts`, `evaporchain-hlwa`, `evaporchain-mcc`, `evaporchain-mdl-shard` | High-bandwidth coupling, MDL sharding, HLW analytics |
| `evaporchain-cfm`, `evaporchain-llsa` | Concurrent-flow MEV / liveness substrates |
| `evaporchain-entropic-slashing`, `evaporchain-sanov-slashing` | Entropic + Sanov-style slashing |
| `evaporchain-crooks-mev-refund` | Crooks-relation MEV refund |
| `evaporchain-boltzmann-stake`, `evaporchain-hot-cold-stake` | Boltzmann-temperature stake + hot/cold stake split |

### Misc substrate

| Crate | Role |
|---|---|
| `evaporchain-lad-vm`, `evaporchain-script-lad`, `evaporchain-lambda-fold` | LAD (Linear Affine Decay) VM dialect + script + fold |
| `evaporchain-wsbf` | Witness-stamped block format |

For up-to-date discovery, `ls crates/` is canonical; the list above is curated
for narrative grouping, not as a registry of every crate.

## How Thermodynamic Decay Works

Every state object and contract in EvaporChain has two key parameters:

- **Energy** (`u64`): The remaining energy budget
- **Half-life** (`u64`): Number of epochs for energy to halve

Energy decays exponentially using integer math:

```
energy_at(epoch) = initial_energy >> (epochs_elapsed / half_life)
                 - fractional_decay_interpolation
```

### Object Lifecycle

```
    ┌──────────┐     energy = 0     ┌──────────┐    grace expires   ┌──────────┐
    │  Active  │ ──────────────────>│  Grace   │ ─────────────────>│  Ghost   │
    └──────────┘                    └──────────┘                    └──────────┘
         ^                               │                               │
         │          refresh()            │                               │
         └───────────────────────────────┘          resurrect()          │
         ^                                                               │
         └───────────────────────────────────────────────────────────────┘
                                  (Resurrected state)
```

1. **Active**: Object is live. Energy decays each epoch.
2. **Grace**: Energy hit zero. Object is still accessible but will evaporate after `grace_period` epochs if not refreshed.
3. **Ghost**: Object evaporated. Only a nullifier proof (data hash, owner, evaporation epoch) remains in the MMR. Can be resurrected with a new energy deposit.

The evaporation engine runs at the end of every block, processing all objects.

## How ZK Proofs Work

EvaporChain uses Nova-based Incrementally Verifiable Computation (IVC) to fold each block into a running recursive proof:

1. Each block's state transition is encoded as an R1CS circuit:
   - Transfer amounts and balance updates
   - Energy decay computations
   - State root transitions (before → after)

2. The circuit is folded with the previous proof using Nova's folding scheme, producing a new running proof.

3. After N blocks, the proof is constant-size regardless of N. A new node joining the network only needs to verify this single proof to trust the current state.

## How Consensus Works

### Tendermint BFT

EvaporChain uses a full Tendermint BFT state machine:

1. **Propose**: The stake-weighted leader (seeded by VRF randomness beacon) creates a block proposal with BLS and VRF proofs
2. **Prevote**: Validators verify the proposal (parent hash, block size, VRF proof, equivocation check) and broadcast BLS-signed prevotes
3. **Precommit**: Once 2/3+ stake prevotes are collected, validators broadcast BLS-signed precommits
4. **Commit**: Once 2/3+ stake precommits are collected, the block is committed with an aggregate BLS commit certificate

Timeouts escalate exponentially (2^round, capped at 64x) with per-height jitter. Equivocation (double-signing) is detected and slashed (10% stake). Validators that miss blocks lose health score. Epoch transitions occur at height boundaries with bonding period delays.

### Encrypted Mempool (MEV Protection)

Transactions can be submitted encrypted using a commit-reveal scheme:

1. **Commit phase**: Transaction is encrypted and a commitment (hash) is submitted
2. **Ordering**: Commitments are ordered deterministically (by hash)
3. **Reveal phase**: After a delay, transactions are decrypted and executed

This prevents validators from reordering transactions for profit (MEV front-running).

### Block Structure

```
Block {
    number:       u64          // Sequential block number
    epoch:        u64          // Epoch (may advance faster than block number)
    parent_hash:  [u8; 32]     // BLAKE3 hash of parent block
    state_root:   [u8; 32]     // Verkle trie root after execution
    transactions: Vec<Tx>      // All transactions in this block
    timestamp:    u64          // Unix timestamp
    producer_id:  Option<u64>  // Validator that produced this block
}
```
