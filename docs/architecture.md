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
Core domain types shared across all crates. Defines `StateObject`, `Account`, `Block`, `Transaction` (including all 7 transaction types), `GhostRecord`, `DualCommitment`, and the energy decay formula. The canonical type definitions live here — no business logic, just data structures and serialization.

### evaporchain-crypto
Cryptographic primitives: BLAKE3 hashing, ML-DSA post-quantum digital signatures (key generation, signing, verification), Verkle trie implementation for state commitments, and Merkle Mountain Range (MMR) for nullifier accumulation. Provides the `Signer`/`Verifier` traits used by the execution layer.

### evaporchain-state
State management layer. Contains the `StateDB` trait (with `InMemoryStateDB` implementation), the **Evaporation Engine** (processes energy decay each epoch, transitions objects through Active → Grace → Ghost lifecycle), and the **Refresh Engine** (handles energy deposits and ghost resurrection). This is where thermodynamic decay is enforced.

### evaporchain-contracts
Template-based smart contract system. Provides 7 pre-built contract templates (DecayingToken, MortalNFT, ThermodynamicEscrow, DecayingAuction, StakingPool, DAOVote, Temporal) with a rule engine for custom behavior (triggers, conditions, actions). Each contract instance has its own energy and half-life — contracts themselves evaporate when unused.

### evaporchain-script
The EvaporScript scripting language. A non-Turing-complete language with three components: **Parser** (lexer + recursive descent parser → AST), **Compiler** (AST → stack-based bytecode with method table), and **VM** (executes bytecode with gas metering, built-in functions, state management). Includes the `ScriptEngine` for deploying and managing script contracts with full lifecycle hook support.

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
Command-line interface for interacting with a running node. Supports transfers, object creation, refresh, account queries, and block inspection via the HTTP API.

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
