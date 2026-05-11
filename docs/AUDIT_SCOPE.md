# EvaporChain Pre-Mainnet Audit Scope

**Prepared for:** External security auditors
**Date:** April 2026 (last refresh 2026-05-11)
**Codebase:** Rust workspace, 147 crates in `crates/` (16 core + 131 substrate / supporting), 25,435+ native tests passing, zero `unsafe` blocks outside the documented WASM bridge (see `crates/evaporchain-crypto-wasm/src/lib.rs` — three compile-time layout guards + runtime regression test). WASM bindings (`evaporchain-crypto-wasm`) ship a separate test corpus runnable via `wasm-pack test`.

## 1. Engagement Overview

EvaporChain is a Layer 1 blockchain with thermodynamic state decay — a novel
mechanism where digital objects lose energy over time and eventually evaporate.
We are seeking a comprehensive security audit before mainnet launch.

### Primary Concerns

1. **Custom cryptographic implementations** (Poseidon, Verkle, MMR)
2. **Zero-knowledge circuit correctness** (Nova IVC, R1CS constraints)
3. **Consensus safety and liveness** (Tendermint-style BFT)
4. **Economic invariants** (balance conservation, fee model)
5. **Smart contract VM security** (EvaporScript)

## 2. Repository Structure

The 18 **core** crates listed below are the in-scope focus of this engagement.
A further 60+ **substrate** crates (Light-Cone Ledger, Bell-Certified Beacon,
Singh-Attractor Consensus, Evaporated-Fork Certificates, Immune Validator Set,
MERA gate, Light-Cone DAG, Singh-Sabi/Migrant patina NFTs, MnemoChain,
SDDC/SFSV/SHLM marketplaces, etc.) implement the Tier-1 invention stack and
launch-dApp lanes; they are documented but secondary to the audit unless the
auditor opts to include them.

```
EvaporChain/
├── crates/                            # 147 crates total
│   ├── evaporchain-crypto/            # Cryptographic primitives (PRIORITY 1)
│   ├── evaporchain-proving/           # Nova IVC proofs (PRIORITY 1)
│   ├── evaporchain-consensus/         # BFT consensus (PRIORITY 1)
│   ├── evaporchain-execution/         # Block/tx execution (PRIORITY 2)
│   ├── evaporchain-state/             # State DB + evaporation (PRIORITY 2)
│   ├── evaporchain-script/            # EvaporScript VM (PRIORITY 2)
│   ├── evaporchain-contracts/         # Template contracts (PRIORITY 3)
│   ├── evaporchain-types/             # Core types (PRIORITY 3)
│   ├── evaporchain-network/           # libp2p networking (PRIORITY 3)
│   ├── evaporchain-da/                # Data availability (PRIORITY 3)
│   ├── evaporchain-oracle/            # Oracle data feeds (PRIORITY 3)
│   ├── evaporchain-fee-controller/    # PID fee controller (PRIORITY 3)
│   ├── evaporchain-sharding/          # Sharding (experimental) (PRIORITY 4)
│   ├── evaporchain-node/              # Node binary (PRIORITY 4)
│   ├── evaporchain-cli/               # CLI tool (PRIORITY 4)
│   ├── evaporchain-mcp/               # MCP server (PRIORITY 4)
│   ├── evaporchain-crypto-wasm/       # WASM crypto bindings (PRIORITY 4)
│   ├── evaporchain-light-cone/        # Light-Cone Ledger DAG (substrate, optional)
│   ├── evaporchain-bell-beacon/       # Bell-Certified Beacon (substrate, optional)
│   ├── evaporchain-singh-attractor/   # Singh Attractor Consensus (substrate, optional)
│   ├── evaporchain-evap-fork-cert/    # Evaporated-Fork Certificates (substrate, optional)
│   ├── evaporchain-ib-validators/     # Immune Validator Set (substrate, optional)
│   ├── evaporchain-mera/              # MERA gate (substrate, optional)
│   └── …59 further substrate crates (see ARCHITECTURE.md)
├── SECURITY.md                        # Vulnerability disclosure policy
├── docs/
│   ├── THREAT_MODEL.md                # Threat model document
│   ├── CRYPTO_SPEC.md                 # Cryptographic specification
│   └── ARCHITECTURE.md                # System architecture
└── research/
    └── whitepaper.md                  # Protocol whitepaper
```

## 3. Priority 1: Cryptographic Primitives

### 3.1 Poseidon Hash (Custom)

**File:** `crates/evaporchain-crypto/src/hash.rs`
**Spec:** `docs/CRYPTO_SPEC.md` §1.2

**Review Checklist:**
- [ ] Round constant generation is correct and deterministic
- [ ] MDS matrix is maximum distance separable (Cauchy construction valid)
- [ ] S-box exponent (x^5) is valid for Pallas Fp (gcd(5, p-1) = 1)
- [ ] Number of rounds (8F + 56P) provides adequate security margin
- [ ] Sponge construction correctly implements absorption and squeezing
- [ ] Field element conversion (bytes_to_field) does not introduce bias
- [ ] No information leakage through timing or memory access patterns

### 3.2 Verkle Trie (Custom)

**File:** `crates/evaporchain-crypto/src/verkle.rs`
**Spec:** `docs/CRYPTO_SPEC.md` §3.1

**Review Checklist:**
- [ ] Generator points are independent (no known discrete log relations)
- [ ] Pedersen vector commitment is binding under ECDLP
- [ ] Proof generation includes all necessary siblings
- [ ] Proof verification correctly reconstructs root commitment
- [ ] Insert/delete operations maintain trie invariants
- [ ] No hash collision vulnerabilities in node_hash
- [ ] BTreeMap iteration provides deterministic proof serialization

### 3.3 MMR Accumulator (Custom)

**File:** `crates/evaporchain-crypto/src/accumulator.rs`
**Spec:** `docs/CRYPTO_SPEC.md` §3.2

**Review Checklist:**
- [ ] Leaf-to-node position formula is correct: `2i - popcount(i)`
- [ ] Peak calculation handles all binary decompositions
- [ ] Proof includes correct peak set for root reconstruction
- [ ] "Bagging the peaks" produces a unique root
- [ ] Append-only property cannot be violated
- [ ] EnergyStampedNullifier serialization is canonical

### 3.4 Nova IVC Circuit

**File:** `crates/evaporchain-proving/src/nova.rs`
**Spec:** `docs/CRYPTO_SPEC.md` §4.1

**Review Checklist:**
- [ ] Energy decay constraints correctly encode E(t) = E₀ × 2^(−t/τ)
- [ ] Range checks prevent field arithmetic wraparound
- [ ] Balance conservation: Σ sender_debits = Σ receiver_credits
- [ ] Nonce constraints: new_nonce = old_nonce + 1
- [ ] No under-constrained witness variables
- [ ] Padding (unused slots) cannot be exploited
- [ ] Soundness holds: no valid proof for invalid state transition

## 4. Priority 2: Execution and State

### 4.1 Block Execution

**File:** `crates/evaporchain-execution/src/lib.rs`

**Review Checklist:**
- [ ] Nonce replay protection (strict ordering)
- [ ] Balance underflow impossible (checked before debit)
- [ ] Gas fee deduction before execution (not after)
- [ ] Failed tx reverts state but keeps gas fee
- [ ] Block gas limit enforcement
- [ ] Self-transfer rejection

### 4.2 Evaporation Engine

**File:** `crates/evaporchain-state/src/evaporation.rs`

**Review Checklist:**
- [ ] Energy decay formula matches spec: E₀ >> (epochs / half_life)
- [ ] Grace period correctly tracked (immutable once set)
- [ ] Ghost record data_hash matches original data
- [ ] Object deletion and ghost creation are atomic
- [ ] MMR append happens before object deletion

### 4.3 EvaporScript VM

**Files:** `crates/evaporchain-script/src/{vm.rs,parser.rs,compiler.rs}`

**Review Checklist:**
- [ ] Stack depth limit enforced (1,024)
- [ ] Loop iteration limit enforced (100,000)
- [ ] Gas consumed per opcode
- [ ] Checked arithmetic (no wrapping)
- [ ] Division/modulo by zero handled
- [ ] No arbitrary memory access
- [ ] Jump targets validated (within bytecode bounds)

### 4.4 PID Fee Controller

**File:** `crates/evaporchain-execution/src/fees.rs`

**Review Checklist:**
- [ ] No division by zero (gas_limit = 0 case)
- [ ] No overflow in fee calculation (u64 boundaries)
- [ ] Fee bounded by [min_fee, max_fee]
- [ ] Controller converges (no oscillation or runaway)

## 5. Priority 3: Consensus and Network

### 5.1 Tendermint BFT Consensus

**File:** `crates/evaporchain-consensus/src/tendermint.rs`

**Review Checklist:**
- [ ] 2/3+1 threshold correctly computed for validator set
- [ ] Equivocation detection (double-vote)
- [ ] Proposal validation (well-formed, from valid proposer)
- [ ] Timeout behavior doesn't break safety
- [ ] BLS signature verification on votes
- [ ] Aggregate signature verification on commits

### 5.2 Network Layer

**File:** `crates/evaporchain-network/src/service.rs`

**Review Checklist:**
- [ ] Message size validation before deserialization
- [ ] RwLock poisoning recovery
- [ ] No unbounded allocations from network input
- [ ] Peer connection limits

## 6. Known Issues (Disclosed)

Status reflects pre-mainnet hardening work completed against the threat model.

| Issue | Severity | Status | Notes |
|-------|----------|--------|-------|
| Slashing | High | **Closed** | Live: 10% equivocation slash, downtime jailing, signed evidence; tested in `evaporchain-consensus` |
| BLS rogue-key | Medium | **Closed** | Proof-of-possession enforced at validator registration (`add_validator()` and genesis-time `verify_pop`) |
| Encrypted mempool | Medium | **Closed** | AES-256-GCM mempool encryption landed |
| DA-2D wiring | Medium | **Closed** | `data_root` now built from `build_block_da_inputs(txs)`, encoded at proposal time and verified on serve |
| BLS key-at-rest | Medium | **Closed** | Encrypted-Validator-Private-Key-Layout (EVPL) format: Argon2id KDF + XChaCha20-Poly1305; magic-byte detection for migration |
| Plaintext-bytes coordinator pubkey | Low | **Closed** | `MAINNET_COORDINATOR_PK` length-checked at startup; `Option<&[u8]>` API with explicit None default |
| No weak subjectivity | Medium | Open | Long-range attacks possible; weak-subjectivity checkpoints not yet enforced |
| Poseidon on Pallas vs BN254 | Low | By design | Separate domains, documented in `CRYPTO_SPEC.md` §1.2 |
| Block-STM O(N²) under high contention | Medium | Open | MVCC retry storm possible; tracked in audit backlog |

## 7. Test Suite

```
Total: 25,435+ native tests across 147 workspace crates, all passing.
       Additional WASM-binding tests in evaporchain-crypto-wasm
       (run separately via `wasm-pack test`).
       ~162 TypeScript tests for dApps/SDK/website.
```

Per-crate counts shift as substrate crates are added; auditors should treat
`cargo test --workspace 2>&1 | tail -20` as the source of truth for the
current totals at the start of an engagement.

Zero `unsafe` blocks in the codebase. All native tests pass on the reference
3-Mini Tailscale cluster (Apple M4, macOS) used for sustained run-time
verification.

## 8. Build and Test Instructions

```bash
# Prerequisites: Rust 1.75+, macOS or Linux
git clone <repo>
cd EvaporChain

# Run all tests
cargo test --workspace

# Run specific crate tests
cargo test -p evaporchain-crypto
cargo test -p evaporchain-proving  # ~10s (Nova proofs)
cargo test -p evaporchain-execution  # ~9s (stress tests)

# Build release (for benchmarking)
cargo build --release
```

## 9. Recommended Audit Firms

Based on the codebase profile (Rust, ZK proofs, BFT consensus, custom crypto):

| Firm | Strengths | Fit |
|------|-----------|-----|
| Trail of Bits | Rust, crypto, formal verification | Excellent |
| Least Authority | ZK systems, protocol review | Excellent |
| Zellic | ZK circuits, blockchain | Excellent |
| OtterSec | Rust blockchain, Solana experience | Good |
| Veridise | Formal verification of ZK circuits | Good (R1CS focus) |
| Sigma Prime | Rust blockchain, Lighthouse audit | Good |

## 10. Timeline

| Milestone | Target |
|-----------|--------|
| Audit engagement signed | Q2 2026 |
| Audit begins | Q2 2026 |
| Findings delivered | Q3 2026 |
| Remediation complete | Q3 2026 |
| Re-audit (if needed) | Q3 2026 |
| Mainnet launch | Q4 2026 |
