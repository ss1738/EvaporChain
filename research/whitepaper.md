# EvaporChain: A Thermodynamic State-Decay Blockchain with Post-Quantum Security

**Version 1.0 — March 2026**

**Satyawan Singh**
Infonova Solutions, Leicester, United Kingdom

---

## Abstract

We present EvaporChain, a novel Layer-1 blockchain that introduces *thermodynamic state decay* as a first-class protocol primitive. Every on-chain object carries an energy value that decays exponentially over time according to a configurable half-life. Objects whose energy reaches zero enter a grace period and, if not refreshed, are *evaporated* — removed from active state and compressed into cryptographic ghost records within a Merkle Mountain Range (MMR) nullifier accumulator. This mechanism provides automatic state pruning, an economic incentive for state maintenance, and a new paradigm for ephemeral data on-chain.

EvaporChain combines this core innovation with post-quantum ML-DSA (Dilithium3) transaction signatures, Verkle trie state commitments, an encrypted commit-reveal mempool for MEV resistance, a PID-controlled adaptive fee market, Nova-based recursive SNARK proofs for succinct chain verification, and a custom stack-based smart contract VM (EvaporScript). The result is a blockchain where state bloat is physically impossible — unused data decays and disappears, just as heat dissipates in thermodynamic systems.

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [Thermodynamic State Decay](#2-thermodynamic-state-decay)
3. [State Model](#3-state-model)
4. [Consensus: Rotating Leader with Health Scores](#4-consensus-rotating-leader-with-health-scores)
5. [Post-Quantum Cryptography](#5-post-quantum-cryptography)
6. [Verkle Trie State Commitments](#6-verkle-trie-state-commitments)
7. [MMR Nullifier Accumulator](#7-mmr-nullifier-accumulator)
8. [MEV Protection: Encrypted Mempool](#8-mev-protection-encrypted-mempool)
9. [Transaction Execution and Fee Market](#9-transaction-execution-and-fee-market)
10. [EvaporScript Virtual Machine](#10-evaporscript-virtual-machine)
11. [Nova Recursive Proofs](#11-nova-recursive-proofs)
12. [Persistence and Storage](#12-persistence-and-storage)
13. [Networking](#13-networking)
14. [Economic Model](#14-economic-model)
15. [Security Analysis](#15-security-analysis)
16. [Comparison with Existing Chains](#16-comparison-with-existing-chains)
17. [Future Work](#17-future-work)
18. [Conclusion](#18-conclusion)
19. [References](#19-references)

---

## 1. Introduction

### 1.1 The State Bloat Problem

Every major blockchain suffers from unbounded state growth. Ethereum's state trie exceeds 200 GB and grows monotonically — data written on-chain persists forever, regardless of whether anyone accesses it again. Solana's account model requires rent-exempt minimum balances but never actually reclaims abandoned accounts. Sui's object model tracks ownership but provides no mechanism for natural expiry.

This creates three compounding problems:

1. **Validator resource burden**: Full nodes must store and serve the entire state, raising hardware requirements over time and centralizing validation.
2. **Economic misalignment**: A single transaction pays a one-time fee but imposes perpetual storage costs on the network.
3. **Semantic noise**: Stale data — expired NFTs, closed positions, abandoned contracts — pollutes the state and degrades query performance.

### 1.2 The Thermodynamic Insight

Physical systems do not suffer from state bloat. In thermodynamics, ordered states naturally decay toward entropy. Heat dissipates. Radioactive isotopes decay. Information encoded in physical media degrades over time unless energy is continuously applied to maintain it.

EvaporChain applies this principle to blockchain state. Every on-chain object is assigned an *energy* value and a *half-life*. Energy decays exponentially:

```
E(t) = E₀ · 2^(−t / τ)
```

where `E₀` is the initial energy, `t` is the elapsed time in epochs, and `τ` is the half-life. When energy reaches zero, the object enters a *grace period*. If no participant refreshes the object during grace, it *evaporates* — its data is hashed into a ghost record, the ghost is appended to a nullifier accumulator, and the object is deleted from active state.

This is not garbage collection. It is a protocol-level physical law. Objects that matter are refreshed by stakeholders who value them. Objects that no one values decay and vanish, exactly as they should.

### 1.3 Design Principles

EvaporChain is built on five principles:

1. **State has mass**: On-chain data costs energy to maintain. No free storage.
2. **Decay is natural**: Unused state decays automatically. No manual cleanup.
3. **Ghosts remember**: Evaporated objects leave cryptographic proofs of existence. History is not lost — it is compressed.
4. **Post-quantum from day one**: All transaction signatures use ML-DSA (FIPS 204), not ECDSA.
5. **Provable execution**: Every block is folded into a recursive SNARK. Any participant can verify the entire chain history with a single proof.

---

## 2. Thermodynamic State Decay

### 2.1 Energy Decay Formula

Every `StateObject` carries two parameters: `energy` (a 64-bit unsigned integer) and `half_life` (epochs until energy halves). The energy at epoch `t` relative to the last refresh is:

```
E(t) = E₀ >> (t / τ)  −  (E₀ >> (t / τ)) · (t mod τ) / (2τ)
```

This integer approximation avoids floating-point arithmetic entirely. The implementation uses bit-shifting for full halvings and linear interpolation for the fractional remainder:

```rust
fn energy_at_epoch(initial: u64, half_life: u64, epochs_elapsed: u64) -> u64 {
    if half_life == 0 { return 0; }
    let full_halvings = epochs_elapsed / half_life;
    let remainder = epochs_elapsed % half_life;
    if full_halvings >= 64 { return 0; }
    let after_halvings = initial >> full_halvings;
    let fractional_decay = after_halvings * remainder / (2 * half_life);
    after_halvings.saturating_sub(fractional_decay)
}
```

The time until energy reaches zero is bounded by:

```
T_zero ≤ (⌊log₂(E₀)⌋ + 1) · τ
```

### 2.2 Object Lifecycle

Every object follows a deterministic lifecycle:

```
┌──────────┐     E(t)=0      ┌───────┐    grace expires    ┌───────┐
│  Active   │ ──────────────> │ Grace │ ──────────────────> │ Ghost │
└──────────┘                  └───────┘                     └───────┘
     ▲                            │                              │
     │         refresh()          │                              │
     └────────────────────────────┘          resurrect()         │
     ▲                                                           │
     │              ┌─────────────┐                              │
     └───────────── │ Resurrected │ <────────────────────────────┘
                    └─────────────┘
```

**Active**: The object exists in full, decaying each epoch.

**Grace**: Energy has reached zero. The object remains accessible for `grace_period` epochs (default: 5). Any participant may refresh it back to Active by depositing new energy.

**Ghost**: Grace expired without refresh. The object's data is hashed (BLAKE3), the hash is stored in a `GhostRecord`, and the ghost is appended to the MMR nullifier accumulator. The original data is retained in the ghost for potential resurrection.

**Resurrected**: A ghost object restored to active state by paying a resurrection fee and depositing fresh energy.

### 2.3 Evaporation Engine

The `EvaporationEngine` runs once per block, after transaction execution:

```
EVAPORATE(state, current_epoch, grace_period):
  entered_grace = []
  evaporated = []

  for each object in state:
    if object.state == Ghost:
      continue  // already dead

    if object.state == Grace:
      if current_epoch >= object.grace_epoch + grace_period:
        ghost = GhostRecord {
          object_id: object.id,
          owner: object.owner,
          evaporated_at: current_epoch,
          data_hash: blake3(object.data),
          original_data: object.data,
        }
        state.put_ghost(ghost)
        state.delete_object(object.id)
        evaporated.push(object.id)
      // else: still in grace, keep waiting

    else:  // Active or Resurrected
      current_energy = energy_at_epoch(
        object.energy,
        object.half_life,
        current_epoch - object.last_refreshed
      )
      if current_energy == 0:
        object.state = Grace
        object.grace_epoch = current_epoch
        entered_grace.push(object.id)

  return EvaporationResult { entered_grace, evaporated }
```

### 2.4 Refresh and Resurrection

**Refresh** deposits additional energy into an Active or Grace object:

- For Active/Resurrected objects: `energy += deposit`, `last_refreshed = current_epoch`
- For Grace objects: `energy = deposit`, `state = Active`, `grace_epoch = None`

**Resurrection** restores a Ghost to active state:

- Requires paying a resurrection fee (60% of original creation cost, minimum 500)
- Creates a new `StateObject` with state `Resurrected`, fresh energy, and the ghost's original data
- Removes the ghost record from state (the MMR entry persists as historical proof)

---

## 3. State Model

### 3.1 Core Data Structures

**StateObject** — the fundamental unit of on-chain state:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `[u8; 32]` | Unique object identifier |
| `owner` | `[u8; 32]` | Owner's account address |
| `energy` | `u64` | Current energy units |
| `half_life` | `u64` | Epochs to halve energy |
| `created_at` | `u64` | Creation epoch |
| `last_refreshed` | `u64` | Last refresh epoch (decay origin) |
| `state` | `ObjectState` | Lifecycle state (Active/Grace/Ghost/Resurrected) |
| `grace_epoch` | `Option<u64>` | Epoch when grace period began |
| `data` | `Vec<u8>` | Arbitrary application data |

**Account** — balance and nonce tracking:

| Field | Type | Description |
|-------|------|-------------|
| `address` | `[u8; 32]` | 32-byte account address |
| `balance` | `u64` | Token balance |
| `nonce` | `u64` | Transaction counter (replay protection) |

**GhostRecord** — compressed proof of evaporated objects:

| Field | Type | Description |
|-------|------|-------------|
| `object_id` | `[u8; 32]` | Original object ID |
| `owner` | `[u8; 32]` | Original owner |
| `evaporated_at` | `u64` | Epoch of evaporation |
| `data_hash` | `[u8; 32]` | BLAKE3 hash of original data |
| `original_data` | `Vec<u8>` | Full data (for resurrection) |
| `mmr_position` | `Option<u64>` | Position in MMR accumulator |

### 3.2 Dual-Commitment State Root

EvaporChain maintains two parallel commitment structures:

1. **Verkle Trie** — commits to all active objects and accounts
2. **MMR Accumulator** — commits to all evaporated nullifiers

The combined state commitment is:

```
DualCommitment {
  verkle_root: [u8; 32],   // Active state
  mmr_root: [u8; 32],      // Evaporation history
  epoch: u64,
  active_count: usize,
  ghost_count: usize,
}
```

This allows clients to prove both that an object exists in active state *and* that a specific object was evaporated at a specific epoch.

### 3.3 Transaction Types

```
Transaction = Transfer      — Value transfer between accounts
            | CreateObject  — Create a new state object with energy
            | Refresh       — Deposit energy into an existing object
            | DeployContract — Deploy an EvaporScript contract template
            | CallContract  — Invoke a deployed contract method
            | DeployScript  — Deploy a standalone EvaporScript
            | CallScript    — Execute a deployed script method
```

Every transaction type carries optional `signature: Vec<u8>` and `public_key: Vec<u8>` fields. A canonical `signable_bytes()` method produces the message to be signed, excluding the signature fields themselves.

---

## 4. Consensus: Rotating Leader with Health Scores

### 4.1 Validator Model

Each validator in the set is described by:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `u64` | Unique validator ID |
| `stake` | `u64` | Staked token amount |
| `address` | `[u8; 32]` | Validator address |
| `blocks_produced` | `u64` | Lifetime block count |
| `evaporations_processed` | `u64` | Lifetime evaporation count |
| `health_score` | `f64` | 0.0–1.0, updated per block |

### 4.2 Health Score Dynamics

The health score creates a feedback loop that incentivizes validators to process evaporations honestly:

**After producing a block with `n` evaporations:**

```
health_score = min(health_score + n × 0.05, 1.0)
```

**Per-epoch decay (applied to all validators):**

```
health_score = max(health_score − 0.01, 0.0)
```

Constants:
- `HEALTH_PER_EVAPORATION` = 0.05
- `HEALTH_DECAY_RATE` = 0.01 per epoch
- `MAX_HEALTH_SCORE` = 1.0
- `HEALTH_BONUS_CAP` = 0.20 (20% max weight bonus)

### 4.3 Leader Election

Leader selection is deterministic and weighted by effective stake:

```
effective_weight(v) = v.stake × (1.0 + min(v.health_score, 1.0) × 0.2)
```

For epoch `e`:

```
seed = blake3("leader" || e)
index = seed mod Σ effective_weight(v)
Leader = first validator v where cumulative_weight ≥ index
```

This gives validators with higher health scores (those who faithfully process evaporations) a modest advantage in leader selection — up to 20% bonus weight.

### 4.4 Block Production

The leader for each epoch:

1. Drains the mempool (and encrypted mempool if MEV protection is enabled)
2. Executes transactions against the state database
3. Runs the evaporation engine
4. Computes the new state root (Verkle trie)
5. Constructs the block with `parent_hash = blake3(prev_block_hash || prev_state_root)`
6. Signs and broadcasts the block

Followers validate:
- Block producer ID matches expected leader for the epoch
- Re-execute all transactions and evaporation
- Verify state root matches

### 4.5 Light-Cone Full DAG Mode (Optional, governance-gated)

In addition to the rotating-leader Tendermint engine described above, EvaporChain ships a **DAG-mode consensus path** that operators can activate via the `light_cone_state_branches_enabled` governance flag. When the flag is `true`, the chain replaces single-tip linear consensus with **partial-order causal-set consensus** over a Light-Cone DAG. This is the doctrine's "Light-Cone Consensus" primitive (`research/INVENTION_STACK.md §A1.2 row 1`); the full plan + locked decisions are in `LIGHT_CONE_FULL_DAG_PLAN.md` + `research/light_cone/PHASE_3_DECISIONS.md` + `research/light_cone/PHASE_4_DECISIONS.md`.

Default chain behaviour is bit-compat with §§4.1-4.4 above. The DAG-mode pieces below activate only under the rollout flag.

#### 4.5.1 Multi-parent block format

`Block` carries an optional `parents: Vec<[u8; 32]>` field (with `serde(default, skip_serializing_if = "Vec::is_empty")`) for multi-parent merge nodes. Single-parent legacy blocks serialize bit-identically (the field is omitted when empty); the canonical `block_hash` does NOT include `parents`, preserving chain-id continuity. `Block::effective_parents()` returns the explicit parents when non-empty, else `vec![parent_hash]` (single-parent fallback). `Block::validate_parents_wire_format()` enforces the soft-fork gate: `parents.len() > 1` requires `protocol_version >= 3`.

#### 4.5.2 Tip selection — Maximum-Caliber fork choice

`MccForkChoice` walks the DAG's leaves (via `LightCone::leaves()`), scores each tip's first-parent trajectory by `path_caliber` (the Maximum-Caliber path-entropy primitive from `evaporchain-mcc`), and returns the leaf with the highest score. Tie-break is the smaller `BlockId` byte ordering (validator-deterministic since `leaves()` is `BTreeMap`-sorted). `TendermintConsensus::current_tip()` uses this when `parent_acceptance_mode = "mcc"`; the proposer at `create_proposal` builds its block on top of the DAG-derived head.

#### 4.5.3 Per-fork state branches

`TendermintConsensus::state_branches: HashMap<BlockId, LightConeBranchMetadata>` tracks per-tip metadata (created_at_block, last_touched_block, caliber). Each entry can hold an `Arc<dyn LightConeBranchSnapshot + Send + Sync>` reference to the executor's per-tip RocksDB snapshot (Phase 3.2 contract). Concurrent forks are capped at `light_cone_max_concurrent_forks` (default 4); LRU eviction by lowest caliber drops the metadata AND triggers a DAG-side cascade prune via `LightCone::prune_orphan_branch` (Phase 5 contract).

#### 4.5.4 Antichain finalization

A set of blocks `S` is finalized iff:

1. **Antichain:** `is_antichain(lc, &S)` — every pair concurrent (vacuous on the DAG's leaves).
2. **Quorum per block:** every `b ∈ S` has `dag_round_states[b].precommits.len() ≥ 2f + 1`, where `f = (|validator_set| - 1) / 3`.
3. **Closing antichain coverage:** `S` covers `closing_antichain(lc)` — implicit when `S` is the subset of leaves meeting condition 2.

`TendermintConsensus::try_finalize_antichain()` computes this predicate. The chain finalizes block sets, not single heights — the height-indexed `committed_at: HashMap<u64, u64>` is paired with a block-indexed `committed_at_block: HashMap<BlockId, u64>` (dual-mode bookkeeping per `PHASE_4_DECISIONS.md` Decision 4).

#### 4.5.5 Cross-fork equivocation

When validator `V` precommits on tip B with a different `block_hash` than they did for concurrent tip A at the same round, `cross_fork_equivocations[V]` increments. Operators feed `[counts]` into `evaporchain_entropic_slashing::entropic_slash(stake, counts)` to derive the slash amount — same pattern as Crooks-MEV's `MissingRefund` counter (§8.4.3). Counts-based detection cannot distinguish honest re-vote from malicious double-vote at this layer; certificate-based equivocation evidence with on-chain proof is a Phase 4.3d follow-up.

#### 4.5.6 Performance

Benchmarked on a Mac Mini M4 under release with a 1000-block DAG @ 4 concurrent forks:

| Operation | Measured | Budget |
|---|---|---|
| `LightCone::insert` per block | 418 ns | < 100 ms |
| `MccForkChoice::select_tip` over 1000 blocks | 365 µs | < 50 ms |
| 4-fork state-branch metadata + LRU prune | 15.8 µs | < 200 ms |

All hot operations are 100×–10⁵× under their plan budgets, leaving ample headroom for production load.

#### 4.5.7 Rollout

DAG mode activates via two governance flags:

- `light_cone_state_branches_enabled` — `"true"` / `"false"` (default `false`).
- `light_cone_max_concurrent_forks` — `u8 in 1..=8` (default `4`).
- `light_cone_orphan_caliber_threshold` — caliber floor for Phase 5.1 orphan detection.

Operators flip on testnet first, observe `state_branches` + `dag_round_states` + `try_finalize_antichain` behaviour, then flip mainnet via governance once cluster operations are validated. Linear Tendermint stays as the fallback governance mode for emergency rollback.

---

## 5. Post-Quantum Cryptography

### 5.1 ML-DSA (FIPS 204 / Dilithium3)

All transaction signatures use ML-DSA at NIST Security Level 3, based on the Module-Lattice Digital Signature Algorithm (formerly CRYSTALS-Dilithium):

| Parameter | Value |
|-----------|-------|
| Public Key Size | 1,952 bytes |
| Signature Size | 3,293 bytes |
| Security Level | NIST Level 3 (AES-192 equivalent) |
| Hardness Assumption | Module-LWE / Module-SIS |
| Algorithm | `pqcrypto_dilithium::dilithium3` |

**Signing flow:**

```
msg = tx.signable_bytes()  // Canonical serialization excluding sig/pk
sig = dilithium3::detached_sign(msg, secret_key)
tx.signature = sig.as_bytes()
tx.public_key = public_key.as_bytes()
```

**Verification flow (in block execution):**

```
msg = tx.signable_bytes()
pk = dilithium3::PublicKey::from_bytes(tx.public_key)
sig = dilithium3::DetachedSignature::from_bytes(tx.signature)
result = dilithium3::verify_detached_signature(&sig, &msg, &pk)
```

Transactions without valid signatures are rejected with `ExecutionError::MissingSignature` or `ExecutionError::InvalidSignature`.

### 5.2 BLAKE3 Hashing

All internal hashing uses BLAKE3, a cryptographic hash function based on the Bao tree structure:

- Output: 256 bits (32 bytes)
- Performance: ~1 byte/cycle on modern x86
- Used for: block hashes, state root computation, nullifier hashing, commitment generation, Verkle node commitments

### 5.3 Rationale

ECDSA (secp256k1), used by Ethereum and Bitcoin, is vulnerable to Shor's algorithm on a sufficiently large quantum computer. While no such computer exists today, blockchain data is immutable — signatures made today must remain secure for decades. EvaporChain adopts post-quantum signatures from genesis, avoiding the complex migration that legacy chains will eventually face.

---

## 6. Verkle Trie State Commitments

### 6.1 Structure

EvaporChain uses a Verkle trie with Inner Product Argument (IPA) commitments over the Pallas elliptic curve:

| Parameter | Value |
|-----------|-------|
| Branching Factor | 256 (8-bit path indices) |
| Maximum Depth | 32 levels |
| Key Size | 32 bytes |
| Value Size | 32 bytes |
| Commitment Scheme | Pedersen vector commitment (Pallas curve) |

**Node types:**
- `InternalNode`: Up to 256 children, each a node reference
- `LeafNode`: Terminal node with (key, value) pair
- `Empty`: Placeholder for absent branches

### 6.2 Commitment Computation

For an internal node with children `{(i, child_hash_i)}`:

```
C = Σ scalar(child_hash_i) · G_i
```

where `G_i` are independent generator points derived as:

```
G_i = scalar_hash("EvaporChain_Verkle_Gen_" || i) · G_base
```

The root commitment is a 32-byte hash of the root node's Pallas point.

### 6.3 State Root Construction

The state root is computed from all accounts and active objects:

```
for each account (addr, balance, nonce):
  key = blake3("acct" || addr)
  val = blake3(balance_le_bytes || nonce_le_bytes)
  trie.insert(key, val)

for each active object (id, energy, state_byte):
  key = blake3("obj" || id)
  val = blake3(energy_le_bytes || state_byte)
  trie.insert(key, val)

state_root = trie.root()
```

### 6.4 Advantages over Merkle-Patricia Tries

Verkle tries provide:
- **Smaller proofs**: O(log₂₅₆ n) ≈ O(n/256) vs O(log₂ n) for Merkle
- **Constant-size witnesses**: Each level requires one opening proof, not 15 sibling hashes
- **Stateless verification**: Verkle proofs are small enough for light clients to verify state transitions without storing the full trie

---

## 7. MMR Nullifier Accumulator

### 7.1 Purpose

When an object evaporates, its existence must remain provable even though it has been deleted from active state. The Merkle Mountain Range (MMR) serves as an append-only accumulator of evaporation nullifiers.

### 7.2 Nullifier Construction

Each evaporated object produces an `EnergyStampedNullifier`:

```
nullifier = blake3(
  object_id       ||  // 32 bytes
  value_hash      ||  // 32 bytes (blake3 of object data)
  evaporation_epoch || // 8 bytes LE
  energy_at_death ||  // 8 bytes LE
  owner              // 32 bytes
)
```

This 32-byte nullifier is appended to the MMR.

### 7.3 MMR Operations

**Append** (O(log n)):

```
1. Push leaf = nullifier to nodes array
2. While (position >> height) & 1 == 1:
   left_sibling = nodes[position - (1 << height)]
   parent = blake3(left_sibling || current)
   Push parent
   height += 1
3. Return (leaf_index, mmr_size)
```

**Root** (peak bagging):

```
peaks = collect all peak nodes (local roots of complete binary trees)
root = fold_right(peaks, |a, b| blake3(a || b))
```

**Proof generation** (O(log n)):
- Collect sibling hashes from leaf to peak
- Collect all other peak hashes
- Return `(siblings, peak_hashes, leaf_index)`

**Proof verification**:
- Rebuild from leaf using siblings (left/right determined by index bits)
- Bag peaks including the rebuilt peak
- Compare with expected root

### 7.4 Properties

- **Append-only**: Nullifiers can only be added, never removed
- **Deterministic**: Same evaporation sequence always produces the same root
- **Compact**: O(log n) storage for peaks, O(n) total
- **Proof size**: O(log n) elements per inclusion proof

---

## 8. MEV Protection: Encrypted Mempool

### 8.1 Commit-Reveal Scheme

EvaporChain implements an optional encrypted mempool to prevent Maximal Extractable Value (MEV) attacks such as front-running, back-running, and sandwich attacks.

**Commit phase** (user submits encrypted transaction):

```
1. User generates random nonce: [u8; 32]
2. plaintext = serialize(transaction)
3. commitment = blake3(plaintext || nonce)
4. aes_key = blake3("EvaporChain_MEV_Key_" || nonce)
5. gcm_nonce = blake3(nonce)[0..12]
6. encrypted_payload = AES-256-GCM.encrypt(aes_key, gcm_nonce, plaintext)
7. Submit: (commitment, encrypted_payload, blake3(nonce), epoch)
```

**Reveal phase** (after `reveal_delay` epochs):

```
1. User reveals the nonce
2. Validator verifies blake3(nonce) == stored nonce_hash
3. Decrypt payload using derived AES key
4. Verify blake3(plaintext || nonce) == stored commitment
5. Deserialize and execute transaction
```

### 8.2 Ordering

Encrypted transactions are ordered by their commitment hash (deterministic, unmanipulable by validators). Plaintext transactions are appended after all revealed encrypted transactions in FIFO order. This prevents validators from reordering transactions for profit.

### 8.3 Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `reveal_delay` | 3–5 epochs | Epochs between commit and reveal |
| Encryption | AES-256-GCM | Authenticated encryption |
| Commitment | BLAKE3 | Binding and hiding |

### 8.4 Crooks-MEV Restitution (Restitutive)

Where the encrypted mempool is the **preventive** layer, Crooks-MEV is the **restitutive** layer: when a sandwich-shaped attack lands despite the encryption (e.g., the chain runs with a non-encrypted relay path, or the attacker's tx flow is observable through some side channel), Crooks-MEV identifies the dissipative work and refunds it to the victim. The mechanism is grounded in the Crooks 1999 fluctuation theorem (per `INVENTION_STACK.md §A1.3`).

#### 8.4.1 Detection

Per committed block, `evaporchain_mev_detect::scan_block` walks the ordered transaction list looking for **strict sandwich triples**:

```
tx_i (attacker → target)   ← front-run leg
tx_j (victim   → target)   ← victim
tx_k (attacker → target)   ← back-run leg
```

where `tx_i.from == tx_k.from` (same attacker), `tx_j.from != attacker` (victim), and all three Transfer txs target the same `to` account. Phase 1 of `CROOKS_MEV_INTEGRATION_PLAN.md` ships only this strict shape; front-run-only and time-delayed sandwiches are not detected.

The detector is O(n²) over Transfer txs; benchmarked at **13.6 ms on a 1000-tx block** (Mac Mini M4, release) — well under the 50 ms hot-path budget.

#### 8.4.2 Crooks-fluctuation refund formula

For each detected observation, the chain computes:

```
log_ratio    = log₂(P_F / P_R)    [millibits — Crooks identity LHS]
ΔF           = W − (1/β) · log_ratio
refund       = max(0, W − ΔF)
```

where `W = work_estimate = front_amount + back_amount` (upper bound, since EvaporChain has no native LP/AMM accounting); `P_F` is the rolling **per-attacker sandwich rate** over a 256-block window; `P_R` is a fixed noise floor (~10⁻⁶ rate); `β` is a governance constant (default 1000 millibits per fee unit). The pmf substitution is rate-based rather than the rigorous forward/reverse path Crooks 1999 calls for — see `research/crooks_mev/PHASE_2_DECISIONS.md` for why the substitution is sound for a non-AMM substrate and what the deferred research follow-up looks like.

`refund_amount = max(0, W − ΔF)` is the dissipative work fed into the settlement transaction.

#### 8.4.3 Settlement

A new protocol-issued transaction variant `Transaction::Refund(RefundTx)` carries the settlement. When governance flag `crooks_mev_settlement_mode = "enforce"`:

1. After the **grace period** (default 5 blocks — operator dispute window) and within the **refund window** (default 256 blocks), the proposer MUST include a `RefundTx` for every due observation.
2. Validators reject blocks that omit a required refund (`MissingRefund`), include unexpected refunds (`UnexpectedRefund`), or carry mismatched payloads (`MismatchedRefund`).
3. The executor's `execute_refund` debits attacker, credits victim, no nonce mutation (refund is not user-signed).
4. Per-proposer `MissingRefund` count feeds `evaporchain_entropic_slashing::entropic_slash(stake, counts)` for slash derivation.

Default `observe` mode keeps the chain bit-compatible with the pre-Crooks-MEV behaviour while operators monitor `/api/mev/observations` to validate the detector's precision before flipping to `enforce`.

#### 8.4.4 Anti-gaming

- **Self-MEV** (attacker == victim) is filtered at detection time — never reaches the buffer.
- **Confidence threshold** (governance flag, default 500_000 ppm) — observations below threshold are recorded but not settled. Phase 1's detector emits `confidence_score_ppm = 1_000_000` for every match; the threshold is in place for when the detector learns to score.
- **Operator dispute** via `POST /api/mev/dispute` — within the grace period, an operator can cancel a pending refund (e.g., false-positive). Past grace, dispute is rejected.

#### 8.4.5 Sublinearity properties carried through to light clients

Light clients verify the chain's MEV-observation buffer state via the `mev_state_digest` (canonical-ordered, blake3 over observations + attacker stats), enabling sublinear-time validator-convergence checks complementary to the Lambda-Fold IVC verifier covered in §11.

### 8.5 Composition: encrypted mempool + Crooks-MEV

| Stage | Encrypted mempool | Crooks-MEV |
|---|---|---|
| When it acts | At submission (commit phase) | At commit + N blocks later (settlement) |
| What it protects | Front-running and reordering by validators | Dissipative work captured by sandwich attacks |
| Failure mode it covers | Validator-side MEV | Network-side MEV that bypassed encryption |
| Cost | Double-round-trip latency | One protocol-issued tx per detected event |
| Governance flag | (none — feature) | `crooks_mev_settlement_mode ∈ {observe, enforce}` |

The two layers are designed to compose: a chain running encrypted mempool with `crooks_mev_settlement_mode = "enforce"` has near-complete MEV defense. An operator running `observe` only logs MEV-shaped events for monitoring without changing block contents — useful for empirical detector tuning before enforcement goes live.

---

## 9. Transaction Execution and Fee Market

### 9.1 Execution Pipeline

For each block:

```
1. Verify all transaction signatures (ML-DSA)
2. Execute transactions in order:
   - Transfer: debit sender, credit receiver, check nonce
   - CreateObject: deduct creation deposit, insert StateObject
   - Refresh: deduct refresh fee, add energy to object
   - DeployContract/Script: store bytecode, deduct deployment gas
   - CallContract/Script: execute VM, deduct gas
3. Run EvaporationEngine (decay, grace, evaporate)
4. Compute new state root (Verkle trie)
5. Fold block into Nova recursive proof
```

### 9.2 Gas Schedule

| Transaction Type | Gas Cost |
|-----------------|----------|
| Transfer | 21,000 |
| CreateObject (base) | 50,000 |
| CreateObject (per data byte) | 200 |
| Refresh | 30,000 |
| Deploy Contract | 100,000 |
| Call Contract | 40,000 |
| Deploy Script | 150,000 |
| Call Script | 50,000 |

### 9.3 State Fees

**Creation Deposit:**

```
deposit = max(data_size × 100, 1000)
```

**Refresh Fee:**

```
fee = max(⌊energy_deposit × 0.20⌋, 100)
```

**Resurrection Fee:**

```
fee = max(⌊creation_cost × 0.60⌋, 500)
```

### 9.4 PID Adaptive Fee Controller

EvaporChain uses a PID (Proportional-Integral-Derivative) controller to adjust the base fee, targeting 50% block utilization:

```
error = (gas_used / gas_limit) − 0.5
integral = clamp(integral + error, −10, 10)
derivative = error − prev_error
adjustment = 0.125·error + 0.01·integral + 0.05·derivative
base_fee = clamp(base_fee × (1 + adjustment), 100, 1_000_000)
```

| Parameter | Symbol | Value |
|-----------|--------|-------|
| Proportional gain | Kp | 0.125 |
| Integral gain | Ki | 0.01 |
| Derivative gain | Kd | 0.05 |
| Target utilization | — | 0.50 |
| Minimum base fee | — | 100 |
| Maximum base fee | — | 1,000,000 |
| Anti-windup clamp | — | ±10.0 |

The PID controller avoids the sharp fee oscillations seen in EIP-1559's exponential adjustment, providing smoother convergence to the target utilization.

---

## 10. EvaporScript Virtual Machine

### 10.1 Architecture

EvaporScript is a stack-based bytecode VM designed for smart contracts that are aware of the thermodynamic lifecycle:

- **Stack-based execution**: operands pushed/popped from a value stack
- **Typed values**: `U64`, `Bool`, `Str`, `Address([u8;32])`, `Map`, `Null`
- **Contract state**: key-value store persisted per contract object
- **Gas metering**: every opcode has a fixed gas cost

### 10.2 Instruction Set

**Stack Operations:**

| Opcode | Gas | Description |
|--------|-----|-------------|
| `Push(Value)` | 1 | Push constant onto stack |
| `Pop` | 1 | Discard top of stack |
| `Load(name)` | 2 | Push local variable |
| `Store(name)` | 2 | Pop and store to local |
| `StateLoad(key)` | 5 | Load from contract state |
| `StateStore(key)` | 10 | Store to contract state |

**Arithmetic (u64):**

| Opcode | Gas | Description |
|--------|-----|-------------|
| `Add` | 3 | Pop two, push sum |
| `Sub` | 3 | Pop two, push difference |
| `Mul` | 5 | Pop two, push product |
| `Div` | 5 | Pop two, push quotient |

**Comparison & Logic:**

| Opcode | Gas | Description |
|--------|-----|-------------|
| `Eq`, `Neq`, `Gt`, `Lt`, `Gte`, `Lte` | 3 | Pop two, push bool |
| `And`, `Or`, `Not` | 3 | Boolean operations |

**Control Flow:**

| Opcode | Gas | Description |
|--------|-----|-------------|
| `Jump(offset)` | 2 | Unconditional jump |
| `JumpIf(offset)` | 2 | Pop bool, jump if true |
| `JumpIfFalse(offset)` | 2 | Pop bool, jump if false |
| `Call(name, argc)` | 10 | Call built-in function |
| `Return` | 1 | Return top of stack |

**Map & State:**

| Opcode | Gas | Description |
|--------|-----|-------------|
| `MapGet(field)` | 10 | Pop key, load from map |
| `MapSet(field)` | 20 | Pop value+key, store in map |

**System:**

| Opcode | Gas | Description |
|--------|-----|-------------|
| `Require` | 5 | Pop bool+msg, revert if false |
| `Emit` | 8 | Pop string, emit event |
| `Halt` | 0 | Stop execution |

### 10.3 Lifecycle Hooks

EvaporScript contracts can define lifecycle methods that are called automatically by the evaporation engine:

- `on_grace()` — called when the contract object enters grace period
- `on_evaporate()` — called just before evaporation (last chance to emit events)
- `on_refresh()` — called when the contract receives an energy deposit

### 10.4 Bytecode Format

```rust
EvaporBytecode {
  name: String,                         // Contract name
  methods: HashMap<String, usize>,      // Method name → opcode offset
  opcodes: Vec<Op>,                     // Flat instruction array
  state_schema: StateSchema,            // Typed state field definitions
}
```

### 10.5 Execution Context

Every VM invocation receives:

```rust
ExecutionContext {
  caller: [u8; 32],    // Transaction sender
  owner: [u8; 32],     // Contract owner
  epoch: u64,           // Current epoch
  energy: u64,          // Contract's current energy
}
```

This allows contracts to reason about their own energy level and adjust behavior as they approach evaporation.

---

## 11. Nova Recursive Proofs

### 11.1 Incremental Verifiable Computation (IVC)

EvaporChain uses the Nova proving system to fold each block into a recursive SNARK. After `n` blocks, a single compressed proof attests to the validity of the entire chain:

```
Proof(block_1, block_2, ..., block_n) → CompressedSNARK
```

Any verifier can check this proof without re-executing any transactions.

### 11.2 Circuit Design

EvaporChain ships **two step-circuit variants** in `crates/evaporchain-proving/src/nova.rs`:

- `BlockStepCircuit` — arity 2 `[state_hash, epoch]`. Minimal binding circuit, retained for testing and as a fallback.
- `RealBlockCircuit` — arity 8 `[state_root_poseidon, mmr_root_hash, epoch, block_number, note_tree_root, pool_balance, total_energy_remaining, step_count]`. Production circuit used by the `--prove` runtime path. Arity bumped from 6 to 8 in Phase 2 of the Lambda-Fold Nova plan to carry the chain-aggregate `total_energy_remaining` (z[6]) and `step_count` (z[7]) inside the IVC state vector.

The state-root binding is now via Poseidon-128 hash over the 4 u64 limbs of the 32-byte verkle root: `z[0] = Poseidon(limb[0], limb[1], limb[2], limb[3])`. This replaces the prior 8-byte truncation (`state_root_to_u64`) which left 192 bits of the state root unbound. Genesis `z0[0]` is computed natively via `nova-snark`'s vanilla `Sponge<Scalar, U24>` to match what the in-circuit `SpongeCircuit` writes to `z_new[0]` at every fold step. The circuit additionally enforces shielded-pool balance conservation `pool_new = pool_old + shields − unshields`, note-tree-root transitions, and a 5-equation chain-aggregate energy-fold gadget.

**Per-step constraints (real circuit):**

1. Epoch monotonicity: `epoch_new = epoch_old + 1`.
2. Block-number monotonicity: `block_number_new = block_number_old + 1`.
3. State-hash binding: Poseidon hash of all 4 u64 limbs of the 32-byte verkle root, bound into `z[0]` (~250 constraints).
4. MMR-root binding: 4-limb decomposition of the 32-byte MMR root.
5. Energy decay (per object): integer thermodynamic model `E(t) = E₀ ≫ (Δepoch / τ)` with saturation, in-circuit.
6. **Chain-aggregate energy fold (Phase 2.3):** 5-equation gadget folding `total_energy_remaining` from one step to the next via the same integer-division + linear-interpolated fractional correction used per-object, with `range_check_bits(128)` on the `u128` total (~130 constraints) and `range_check_bits(64)` on `step_energy` and `step_count` (~64+64 constraints).
7. Transfer balance conservation: `Σ debits = Σ credits` per block.
8. Shielded-pool balance conservation: `pool_new = pool_old + shields − unshields`.
9. Note-tree-root transition: bound to witness deltas.
10. Evaporation nullifier integrity: every nullifier in the block is bound.

**Witness per block (real circuit):** `RealBlockWitness` (see `nova.rs`) carries the 32-byte state root (as 4 u64 limbs), MMR root, epoch and block number, the transfer / energy / evaporation deltas, the shield / unshield aggregates, and the energy-fold witness fields (`prev_total_energy: u128`, `step_energy: u64`, `epochs_elapsed_at_step: u64`, plus 8 intermediate fields for the constraint-(a) through constraint-(e) gadget).

**Constraint count (verified 2026-05-04):** 25,129 R1CS constraints split as 14,575 in the step circuit + 10,554 in the fold/recursion circuit. The step-circuit growth from 14,041 → 14,575 (+534 constraints) accounts for the Poseidon state-root binding (+250), `range_check_bits(128)` on total energy (+130), step-count and step-energy bit decompositions (+64+64), and 5 `cs.enforce` calls for the energy-fold gadget. See `docs/CRYPTO_SPEC.md` §4.1 for the per-section breakdown and `LAMBDA_FOLD_NOVA_PLAN.md` Phase 2.6 for the detailed accounting.

**Sublinearity (verified 2026-05-04):** the `vk` from `CompressedSNARK::setup` is cached on the prover (Phase 3.2 contract). On a Mac Mini M4 under release, `verify_proof` wall-clock is **21.5 ms at 10 folds, 22.9 ms at 50 folds, 23.3 ms at 100 folds** — verify(100)/verify(10) = 1.083, essentially flat. The chain ships the first sublinear-in-active-energy verifier as defined in `INVENTION_STACK.md §4.1` row 8.

### 11.3 Curve Parameters

| Component | Curve/Scheme |
|-----------|-------------|
| Primary curve | BN256 |
| Secondary curve | Grumpkin |
| S1 SNARK | Spartan + HyperKZG |
| S2 SNARK | Spartan + IPA |

### 11.4 Proving Engine Interface

```rust
trait ProvingEngine {
  fn fold_block(&mut self, block, old_root, new_root) -> Result<()>;
  fn get_proof(&self) -> Result<CompressedProof>;
  fn verify_proof(&self, proof, num_blocks, genesis_state) -> Result<bool>;
  fn accumulator_size(&self) -> usize;
  fn num_blocks_folded(&self) -> usize;
}
```

The `CompressedProof` contains the serialized SNARK, the number of folded steps, and the initial state `z₀` for verification.

---

## 12. Persistence and Storage

### 12.1 Write-Through Cache Architecture

EvaporChain uses a dual-layer storage architecture:

```
┌─────────────────────────────┐
│     In-Memory HashMaps      │  ← All reads served here (zero overhead)
│  objects | ghosts | accounts│
└──────────┬──────────────────┘
           │ write-through
┌──────────▼──────────────────┐
│        RocksDB              │  ← Durable persistence
│  CF: objects | ghosts | accts│
└─────────────────────────────┘
```

**Read path**: Directly from in-memory `HashMap` — no disk I/O, no deserialization.

**Write path**: Every mutation writes to both the in-memory cache and RocksDB simultaneously:

```
put_object(obj):
  rocksdb.put(CF_OBJECTS, obj.id, bincode(obj))
  hashmap.insert(obj.id, obj)
```

**Startup**: All data loaded from RocksDB into memory. The node resumes exactly where it left off:

```
open(path):
  db = RocksDB::open(path)
  objects = load_all(CF_OBJECTS)    // bincode deserialization
  ghosts = load_all(CF_GHOSTS)
  accounts = load_all(CF_ACCOUNTS)
```

### 12.2 Column Families

**State DB** (bincode serialization):
- `objects` — StateObject records
- `ghosts` — GhostRecord records
- `accounts` — Account records

**Chain Store** (JSON serialization):
- `blocks` — Block history (big-endian u64 keys for ordered iteration)
- `chain_meta` — Consensus metadata (block number, epoch, parent hash), chain stats, events
- `stores` — DeFi module state (NFT, token, staking, DAO stores)

### 12.3 Crash Recovery

On startup:
1. Check `has_data()` — if accounts exist, it's a restart
2. Load all state from RocksDB into memory
3. Load consensus metadata (block number, epoch, parent hash)
4. Restore consensus engine state via `restore_state()`
5. Load block history, chain stats, DeFi stores
6. Resume block production from the persisted chain tip

If fresh (no data), load genesis state and begin from block 0.

---

## 13. Networking

### 13.1 P2P Protocol

EvaporChain uses libp2p with the following protocol stack:

| Layer | Protocol |
|-------|----------|
| Transport | TCP |
| Encryption | Noise (XX handshake) |
| Multiplexing | Yamux |
| Pub/Sub | GossipSub v1.1 |
| Discovery | mDNS (local), Identify |

### 13.2 Topics

| Topic | Purpose |
|-------|---------|
| `evaporchain/txs/1` | Transaction gossip |
| `evaporchain/blocks/1` | Block propagation |

### 13.3 Message Deduplication

Messages are deduplicated using a hash-based message ID:

```
message_id = hash(message.data || message.topic)
```

This prevents duplicate processing when the same transaction or block is received from multiple peers.

---

## 14. Economic Model

### 14.1 Incentive Alignment

The thermodynamic model creates natural economic incentives:

1. **State creators pay upfront**: Creating objects requires energy deposits and creation fees.
2. **State maintainers pay ongoing**: Refreshing objects costs fees proportional to the energy deposited.
3. **Validators earn from evaporation**: Processing evaporations increases health scores, leading to more block production opportunities.
4. **Resurrection is expensive**: Restoring evaporated objects costs 60% of the original creation cost, discouraging neglect.

### 14.2 State Rent vs. State Decay

Unlike Solana's rent-exemption model (which never actually reclaims state) or proposed Ethereum state expiry (which is complex and controversial), EvaporChain's decay is:

- **Automatic**: No governance votes, no protocol upgrades needed
- **Gradual**: Energy decreases smoothly, giving owners time to react
- **Reversible**: Grace periods and resurrection provide safety nets
- **Economically honest**: The cost of keeping state alive is explicit and continuous

### 14.3 Half-Life as a Market Signal

Different half-lives encode different expectations about data permanence:

| Use Case | Suggested Half-Life | Meaning |
|----------|-------------------|---------|
| Chat messages | 2–5 epochs | Ephemeral by design |
| Session tokens | 3–10 epochs | Short-lived authentication |
| Price feeds / caches | 5–20 epochs | Frequently updated data |
| NFTs / certificates | 50–200 epochs | Long-lived but not permanent |
| Governance tokens | 100–500 epochs | Quasi-permanent with maintenance |
| Validator stakes | 150+ epochs | Very long-lived |

---

## 15. Security Analysis

### 15.1 Quantum Resistance

ML-DSA (Dilithium3) is secure under the Module-LWE and Module-SIS hardness assumptions. No known quantum algorithm provides a significant speedup against lattice-based cryptography. EvaporChain is secure against both classical and quantum adversaries from genesis.

### 15.2 MEV Resistance

The encrypted mempool prevents validators from observing transaction contents before commitment. The deterministic ordering by commitment hash prevents reordering attacks. The reveal delay ensures sufficient separation between commit and execution phases.

### 15.3 State Root Integrity

Verkle trie commitments bind the state root to the exact set of active objects and accounts. Any modification to state produces a different root. Followers independently compute state roots after re-executing blocks, rejecting blocks with mismatched roots.

### 15.4 Evaporation Safety

- **Grace periods** prevent accidental data loss — owners have multiple epochs to notice and refresh
- **Ghost records** preserve cryptographic proof of existence — evaporation is not erasure
- **Resurrection** allows recovery — evaporated objects can be restored (at a cost)
- **MMR accumulator** provides non-repudiable proof of evaporation history

### 15.5 Replay Protection

Transaction nonces prevent replay attacks. Each account tracks a monotonically increasing nonce; transactions with stale or duplicate nonces are rejected.

---

## 16. Comparison with Existing Chains

| Feature | EvaporChain | Ethereum | Solana | Sui |
|---------|------------|----------|--------|-----|
| **State Model** | Thermodynamic decay | Permanent | Rent-exempt | Object (permanent) |
| **State Pruning** | Automatic (evaporation) | None (grows forever) | Theoretical (never enforced) | None |
| **Signatures** | ML-DSA (post-quantum) | ECDSA (secp256k1) | Ed25519 | Ed25519 |
| **State Commitment** | Verkle trie | Merkle-Patricia trie | None (bank hash) | Merkle tree |
| **MEV Protection** | Encrypted mempool | None (flashbots external) | None (Jito external) | None |
| **Fee Model** | PID controller | EIP-1559 exponential | Fixed priority fees | Gas-based |
| **Recursive Proofs** | Nova IVC | None (planned) | None | None |
| **Ghost Records** | Yes (MMR accumulator) | No | No | No |
| **State Resurrection** | Protocol-native | N/A | N/A | N/A |
| **Smart Contracts** | EvaporScript (lifecycle-aware) | EVM (Solidity) | SBF (Rust) | Move VM |

### 16.1 Key Differentiators

1. **Only chain with automatic state decay**: No other production blockchain implements protocol-level state expiry that actually works. Ethereum has discussed state expiry for years without implementation. Solana charges rent but never reclaims accounts.

2. **Post-quantum from genesis**: While all other chains will need complex signature migration, EvaporChain starts with ML-DSA.

3. **Lifecycle-aware smart contracts**: EvaporScript contracts can react to their own energy decay via `on_grace()` and `on_evaporate()` hooks — a paradigm impossible on other chains.

4. **Dual-commitment model**: The combination of Verkle trie (active state) and MMR (evaporation history) is unique. Clients can prove both existence and non-existence of state.

---

## 17. Future Work

### 17.1 Sharding

Thermodynamic decay simplifies sharding: each shard maintains its own Verkle trie and MMR. Cross-shard state references naturally expire if not refreshed, eliminating the "dangling reference" problem that plagues other sharding designs.

### 17.2 ZK Proofs for Evaporation

Replace the mock prover with a full ZK circuit that proves correct evaporation — i.e., that the evaporation engine was applied correctly and the MMR was updated honestly. This would allow light clients to verify evaporation without re-executing.

### 17.3 Programmable Decay Curves

Currently, decay follows a fixed exponential curve. Future work will explore customizable decay functions (linear, stepped, conditional) to support richer application semantics.

### 17.4 Cross-Chain Ghost Bridges

Ghost records could be relayed to other chains via bridge protocols, allowing evaporation proofs to be verified cross-chain. An object evaporated on EvaporChain could trigger actions on Ethereum via an MMR inclusion proof.

### 17.5 Formal Verification

Formally verify the evaporation engine, Verkle trie, and MMR accumulator using tools like Coq or Lean, providing mathematical guarantees of correctness.

---

## 18. Conclusion

EvaporChain introduces a fundamentally new approach to blockchain state management. By treating on-chain data as a thermodynamic system — where energy decays, objects evaporate, and ghosts persist as cryptographic proofs — we solve the state bloat problem that threatens the long-term viability of every existing blockchain.

The protocol combines this core innovation with post-quantum cryptography (ML-DSA), efficient state commitments (Verkle tries), historical proof accumulation (MMR), MEV resistance (encrypted mempool), adaptive fee markets (PID controller), lifecycle-aware smart contracts (EvaporScript), and succinct chain verification (Nova recursive proofs).

State bloat is not inevitable. In EvaporChain, it is thermodynamically impossible.

---

## 19. References

1. NIST. *FIPS 204: Module-Lattice-Based Digital Signature Standard (ML-DSA)*. 2024.
2. Khovratovich, D. and Dunkelman, O. *Verkle Trees*. Ethereum Research, 2021.
3. Todd, P. *Merkle Mountain Ranges*. Open Timestamps, 2012.
4. Kothapalli, A., Setty, S., and Tzialla, I. *Nova: Recursive Zero-Knowledge Arguments from Folding Schemes*. CRYPTO 2022.
5. Daian, P. et al. *Flash Boys 2.0: Frontrunning, Transaction Reordering, and Consensus Instability in Decentralized Exchanges*. IEEE S&P, 2020.
6. Buterin, V. *EIP-1559: Fee market change for ETH 1.0 chain*. Ethereum Improvement Proposals, 2019.
7. Ang, K.H., Chong, G., and Li, Y. *PID Control System Analysis, Design, and Technology*. IEEE Transactions on Control Systems Technology, 2005.
8. Aumasson, J.-P. et al. *BLAKE3: One function, fast everywhere*. 2020.

---

*This whitepaper describes the EvaporChain protocol as implemented in the reference Rust codebase. The protocol is under active development. Parameters and constants are subject to change.*
