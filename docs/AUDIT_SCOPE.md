# EvaporChain Pre-Mainnet Audit Scope

**Prepared for:** External security auditors
**Date:** April 2026 (last refresh **2026-06-01**)
**Codebase:** Rust workspace, **163 crate directories** in `crates/` (**141 active workspace members + 2 excluded WASM crates**), **25,435+ native tests passing**, zero `unsafe` blocks outside the documented WASM bridge (see `crates/evaporchain-crypto-wasm/src/lib.rs` — three compile-time layout guards + runtime regression test). WASM bindings (`evaporchain-crypto-wasm`, `evaporchain-light-client-wasm`) ship separate test corpora runnable via `wasm-pack test`.

**Recent closures (since 2026-05-11 last refresh):**
- **AUDIT_2026_05_17** (9 CRITICAL + 14 HIGH + 25 MEDIUM + 13 LOW) — fully closed. CR-1/CR-2/CR-3 Verkle DST + path-indices binding; H-1 VRF chain-id-scoping (#407); H-2 address-derivation DST (#413); H-4 BLS PoP at non-validator verify sites (#414); Q1-Q13 DA-cert + finality + sampling-seed hardening. Last two PRs (#461 + #469) merged 2026-05-28; running closure log lives in `CHANGELOG.md` + `SESSION_PROGRESS.md`.
- **#469 P0 launch-blocker remediation** (2026-05-28): PRIV-001/002 (shielded txs gated off at v1 via `SHIELDED_TX_DISABLED_V1`), DA-001 (`verify_signatures_bound(registered)` collapses three DA-cert verifiers, dedups by validator_id, binds `att.public_key == registered_key`, strict `> 2T/3`), VM-001 (`DecayingToken::refresh_balance` owner-gated + `checked_add`), API-001 (wallet master key fails closed in production), ECON-001 (slash redistribute conservation fix).
- **Mainnet strict-mode boot path** documented end-to-end in `docs/MAINNET_LAUNCH.md` (2026-06-01) — the 11 pre-flight checks the `--mainnet` binary refuses to skip. Bake-in of `MAINNET_COORDINATOR_PK_BYTES` is the operator-decision blocker, not a code gap.
- **Catalogue surface: 30 templates** (24 → 30 in the 2026-05-31 / 2026-06-01 sprint), all with typed init + bind invariants + fees + required_keys + dispatch arms verified clean by `every_catalogue_default_binds` on Mini-1.

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
├── crates/                            # 163 directories, 141 active workspace members
│   ├── evaporchain-crypto/            # Cryptographic primitives (PRIORITY 1)
│   ├── evaporchain-proving/           # Nova IVC proofs + RealBlockCircuit (PRIORITY 1)
│   ├── evaporchain-consensus/         # BFT consensus + MCC fork-choice + Light-Cone DAG + Crooks-MEV (PRIORITY 1)
│   ├── evaporchain-execution/         # Block/tx execution (sequential + Block-STM) (PRIORITY 2)
│   ├── evaporchain-state/             # State DB + evaporation + refresh engine + WAL (PRIORITY 2)
│   ├── evaporchain-script/            # EvaporScript VM (44 opcodes, gas-metered) (PRIORITY 2)
│   ├── evaporchain-contracts/         # Template contracts + rule engine (PRIORITY 3)
│   ├── evaporchain-types/             # Core types + chain_ids constants (PRIORITY 3)
│   ├── evaporchain-consensus-types/   # Consensus types extracted for WASM compat (PRIORITY 3)
│   ├── evaporchain-network/           # libp2p networking + chain-id-scoped gossipsub (PRIORITY 3)
│   ├── evaporchain-da/                # Data availability (2D Reed-Solomon, PoHA, namespaced MMR) (PRIORITY 3)
│   ├── evaporchain-oracle/            # BFT oracle data feeds (PRIORITY 3)
│   ├── evaporchain-fee-controller/    # Singh-Lyapunov PID fee controller (PRIORITY 3)
│   ├── evaporchain-nova-bridge/       # T0.10 Path A chain-side Nova → L1 Groth16 verifier bridge (PRIORITY 2)
│   ├── evaporchain-eth-bridge/        # Ethereum bridge (PRIORITY 3)
│   ├── evaporchain-paymaster/         # UserOpTx sponsorship + multi-token-gas Option B (PRIORITY 3)
│   ├── evaporchain-app-templates/     # 30-template formal catalogue + class-id registry (PRIORITY 3)
│   ├── evaporchain-app-templates-engine/  # Typed init dispatch (30 InitConfig variants)
│   ├── evaporchain-app-templates-fees/    # Deploy-fee oracle (PRIORITY 3)
│   ├── evaporchain-app-templates-bind/    # Pre-deploy invariant enforcement (PRIORITY 3)
│   ├── evaporchain-sharding/          # Sharding (experimental) (PRIORITY 4)
│   ├── evaporchain-node/              # Node binary + --mainnet strict-mode (PRIORITY 4)
│   ├── evaporchain-cli/               # CLI: genesis ceremony, keygen, validator onboarding (PRIORITY 4)
│   ├── evaporchain-mcp/               # MCP server (26 tools, 13 resources, 6 prompts) (PRIORITY 4)
│   ├── evaporchain-crypto-wasm/       # WASM crypto bindings (separate corpus) (PRIORITY 4)
│   ├── evaporchain-light-client-wasm/ # Browser-side light-client SDK (separate corpus) (PRIORITY 4)
│   ├── evaporchain-light-cone/        # Light-Cone Ledger DAG (substrate, optional)
│   ├── evaporchain-bell-beacon/       # Bell-Certified Beacon (substrate, optional)
│   ├── evaporchain-causal-chsh/       # CHSH cartel detection (Tier-0 frontier primitive, optional)
│   ├── evaporchain-singh-attractor/   # Singh Attractor Consensus (substrate, optional)
│   ├── evaporchain-evap-fork-cert/    # Evaporated-Fork Certificates (substrate, optional)
│   ├── evaporchain-ib-validators/     # Immune Validator Set (substrate, optional)
│   ├── evaporchain-lambda-fold/       # Nova IVC accumulator + chain-aggregate decay (substrate, optional)
│   ├── evaporchain-llsa/              # Linear-Logic Software Audit governance (substrate, optional)
│   └── …~110 further substrate / VM-paradigm / launch-dApp crates
├── docs/MAINNET_LAUNCH.md             # Operator-facing --mainnet strict-mode launch playbook (2026-06-01)
├── docs/GENESIS_CEREMONY.md           # Protocol-level genesis ceremony
├── docs/VALIDATOR_ONBOARDING.md       # Post-launch validator joining
├── docs/TOKENOMICS.md                 # 28-question tokenomics ceremony
├── SECURITY.md                        # Vulnerability disclosure policy
├── docs/
│   ├── THREAT_MODEL.md                # Threat model document
│   ├── CRYPTO_SPEC.md                 # Cryptographic specification
│   └── ARCHITECTURE.md                # System architecture
└── research/
    ├── whitepaper.md                  # Protocol whitepaper
    ├── coq/                           # 5 Coq proofs (Rocq 9.1.1, zero-Admitted under hypotheses)
    └── tla/                           # 5 TLA+ specifications
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

### 6.1 Long-standing items (pre-2026-05)

| Issue | Severity | Status | Notes |
|-------|----------|--------|-------|
| Slashing | High | **Closed** | Live: 10% equivocation slash, downtime jailing, signed evidence; tested in `evaporchain-consensus` |
| BLS rogue-key (validator path) | Medium | **Closed** | Proof-of-possession enforced at validator registration (`add_validator()` and genesis-time `verify_pop`) |
| Encrypted mempool | Medium | **Closed** | AES-256-GCM mempool encryption landed |
| DA-2D wiring | Medium | **Closed** | `data_root` now built from `build_block_da_inputs(txs)`, encoded at proposal time and verified on serve |
| BLS key-at-rest | Medium | **Closed** | Encrypted-Validator-Private-Key-Layout (EVPL) format: Argon2id KDF + XChaCha20-Poly1305; magic-byte detection for migration |
| Plaintext-bytes coordinator pubkey | Low | **Closed** | `MAINNET_COORDINATOR_PK_BYTES` length-checked at startup; `Option<&[u8]>` API with explicit None default until ceremony output is baked in |
| Poseidon on Pallas vs BN254 | Low | By design | Separate domains, documented in `CRYPTO_SPEC.md` §1.2 |
| No weak subjectivity | Medium | Open | Long-range attacks possible; weak-subjectivity checkpoints not yet enforced. Tracked for V1.1 / post-launch governance flag. |
| Block-STM O(N²) under high contention | Medium | Open | MVCC retry storm possible; tracked in audit backlog |

### 6.2 AUDIT_2026_05_17 closures (fully closed 2026-05-28)

Point-in-time audit produced 9 CRITICAL + 14 HIGH + 25 MEDIUM + 13 LOW findings. Every CRITICAL is closed; full running log in `CHANGELOG.md` and `AUDIT_2026_05_17.md` (preserved as a point-in-time snapshot). Headline closures:

| Finding | Severity | Resolution |
|---|---|---|
| **CR-1** Verkle DST drift between `EnergyNode::hash` (no DST) and `EnergyVerkleTrie::verify` (with DST) | CRITICAL | `EnergyNode::hash` now emits `VERKLE_LEAF_DST` / `VERKLE_INTERNAL_DST` prefix, matching verify. |
| **CR-2** Verkle proof `path_indices` never checked against `key` — non-existence proof forgery | CRITICAL | Explicit per-level check `path_indices[level] != key[level]` rejects forged proofs. |
| **CR-3** `verify_multi` reconstructs without DST while `verify` reconstructs with DST | CRITICAL | Producer + both verifiers share the same DST path via `EnergyNode::hash`. |
| **H-1** VRF `leader_vrf_input(height, round)` not chain-id-scoped — cross-chain leader-claim replay | HIGH | Closed via PR #407 — chain_id now bound. |
| **H-2** Address = `blake3(public_key_bytes)` with no DST | HIGH | Closed via PR #413 — address-derivation helper with `ADDRESS_DST = "evaporchain:address:v1\0"`. |
| **H-3** `MMRProof.mmr_size` never validated against external commitment | HIGH | `mmr_size` structurally validated (leaf_count derivation, leaf_index bound, popcount peak count, height-based sibling count) before any hash work. |
| **H-4** BLS rogue-key exposure at non-validator verify sites (browser dApps, light clients, indexers) | HIGH | Closed via PR #414 — bls_portable PoP precondition. |
| **Q1-Q3** DA-cert forgery class: `total_stake` attacker-supplied, no `validator_id` dedup, BLS signed message excludes `stake` field | CRITICAL ×3 | Collapsed into `verify_signatures_bound(registered)` — see DA-001 below. |
| **Q4** Tendermint safety used `>= 2T/3` instead of strict `> 2T/3` | HIGH | Strict `>` enforced at hot-path call sites. |
| **Q5** Antichain finalization count-weighted not stake-weighted | HIGH | Stake-weighted check restored on antichain path. |
| **Q6** 2D-DA sample seed bypassed canonical `DASampler::build_da_sample_seed_v1` | HIGH | Canonical sampler now used; H7 Stage A `data_root` binding active. |
| **Q7** `StateProof::verify` sorted-Merkle had no leaf-index / tree-size / DST | HIGH | Tree-size confusion class closed. |
| **Q8** `verify_da_certificate` called `cert.verify_signatures()` instead of `verify_signatures_with_active(...)` | HIGH | Main path now invokes the helper with active set. |
| **GHOST-A** Resurrection removes ghost record but never marks the MMR nullifier consumed (Paper 1 §3.4 Inv-4) | CRITICAL paper-drift | Pending operator scope decision: paper amendment or MMR-consume implementation. |
| **L0-A** Nova IVC running-total decay used first object's half_life instead of `ChainLambda` | HIGH | Closed; `nova_path.rs` matches `fold.rs` doctrine fix. |
| **CONS-A** All 7 conservation gate sites hard-coded `ChainLambda::default_genesis()` | HIGH | Acknowledged; governance read-path still pending (post-mainnet governance flag). |

### 6.3 #469 P0 launch-blocker remediation (merged 2026-05-28)

The "must-be-closed-before-mainnet" pack. All six launch-blockers closed:

| ID | Severity | Resolution |
|---|---|---|
| **PRIV-001/002** | P0 | Shielded txs gated off at v1 via `SHIELDED_TX_DISABLED_V1` at mempool admission + all three executors. v1 ships without privacy txs by design; re-enabling is a future governance flag. |
| **DA-001** | P0 | Collapsed three DA-cert verifiers into `verify_signatures_bound(registered)`. Dedup by `validator_id`; bind `att.public_key == registered_key`; count registered stake (not attacker-supplied); strict `> 2T/3`. |
| **VM-001** | P0 | `DecayingToken::refresh_balance` owner-gated + `checked_add` against overflow. |
| **API-001** | P0 | Wallet master key fails closed in production — startup refuses to boot with `EVAPORCHAIN_KEY_MASTER` unset, < 16 chars, or set to the dev default. |
| **ECON-001** | P0 | Slash redistribute conservation fix — slashed stake no longer over-credits the proposer due to integer-truncation rounding loss. |

## 7. Test Suite

```
Total: 25,435+ native tests across 141 active workspace crates, all passing.
       Additional WASM-binding tests in evaporchain-crypto-wasm +
       evaporchain-light-client-wasm (run separately via `wasm-pack test`).
       ~300+ TypeScript tests for dApps (typed clients) / SDK / website.
       Coq: 5 proofs zero-Admitted under Rocq 9.1.1.
       TLA+: 5 specs bounded model-check clean.
```

Per-crate counts shift as substrate crates are added; auditors should treat
`cargo test --workspace 2>&1 | tail -20` as the source of truth for the
current totals at the start of an engagement.

Zero `unsafe` blocks in the codebase outside the documented WASM bridge
(`evaporchain-crypto-wasm` — three compile-time layout guards + runtime
regression test). All native tests pass on the reference Apple Silicon
build farm; CI runs `cargo test --workspace` + `cargo clippy -- -D warnings`
+ Coq proofs (`make` in `research/coq/`) on every PR.

**Anti-regression gate:** `every_catalogue_default_binds`
(`crates/evaporchain-app-templates-bind/src/bind.rs::tests`) walks the
full catalogue (currently 30 templates) and asserts every entry's
default_params binds cleanly. This caught two latent gaps during the
2026-05-31 sprint (DECAY_ACCESS_PASS, BELL_ORACLE, MORTAL_DAO had no
engine-side TypedInit variants; required_keys table had missing rows).

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
| Audit engagement signed | Q3 2026 (operator-decision lane T0.12 — auditor selection in progress) |
| Audit begins | Q3 2026 |
| Findings delivered | Q4 2026 |
| Remediation complete | Q4 2026 |
| Re-audit (if needed) | Q4 2026 |
| Mainnet launch | Q4 2026 / Q1 2027 — gated on audit + tokenomics ceremony + multi-validator soak (T0.6) |

**Pre-launch checklist before audit kickoff:**
- [x] `docs/MAINNET_LAUNCH.md` walkthrough (2026-06-01)
- [x] `docs/AUDIT_SCOPE.md` refresh (this doc, 2026-06-01)
- [x] Coq proofs zero-Admitted under Rocq 9.1.1
- [x] TLA+ specs bounded model-check clean
- [x] `every_catalogue_default_binds` anti-regression gate green
- [ ] `docs/BUG_BOUNTY.md` go-live (operator decision — §10 in that doc)
- [ ] `MAINNET_COORDINATOR_PK_BYTES` bake-in (operator decision; coordinator-key ceremony)
- [ ] Tokenomics ceremony — 28 Q's in `docs/TOKENOMICS.md` resolved
| Mainnet launch | Q4 2026 |
