# EvaporChain Full Security Audit Report

**Date:** 2026-04-24
**Scope:** 100% codebase — all crates, docs, tests, infrastructure
**Method:** 12 parallel audit agents, each assigned 1-2 crates, cross-referenced against documentation
**Codebase:** 16 Rust crates, 113,143 LOC, 4,668 test functions

---

## CRITICAL findings — code-verified status (re-verified 2026-04-29)

All 13 CRITICALs from §2 confirmed FIXED in the current source tree:

| ID | Status | Evidence |
|---|---|---|
| C-01 stake-weighted quorum | FIXED | `check_prevote_quorum` + `check_precommit_quorum` aggregate per-validator stake against `stake_quorum_threshold()` at `crates/evaporchain-consensus/src/tendermint.rs:2654-2693` |
| C-02 BLS verify before counting | FIXED | `cert.verify_signatures()` gates DA cert acceptance at `crates/evaporchain-consensus/src/tendermint.rs:3272`; per-vote BLS sig check on prevote/precommit messages |
| C-03 Zero state_root in proposals | FIXED | Explicit zero-hash guard at `tendermint.rs:1604`/`1616` |
| C-04 Reentrancy guard | FIXED | `MAX_CALL_DEPTH` + `call_depth` enforcement at `crates/evaporchain-script/src/lib.rs:332-343` |
| C-05 Map key type discriminant | FIXED | `Value::to_map_key` prefixes by type (`u:`, `b:`, `s:`, `a:`, `n:`, `m:`, `r:`) at `crates/evaporchain-script/src/lib.rs:60-71` — eliminates `U64(42)` vs `Str("42")` collision |
| C-06 Parser depth limit | FIXED | `expr_depth` + `MAX_EXPR_DEPTH` at `crates/evaporchain-script/src/parser.rs:472-991` |
| C-07 ML-DSA layout safety | FIXED | `signing_key.to_bytes()` (public API) replaces byte slicing at `crates/evaporchain-crypto/src/signatures.rs:208` |
| C-08 RocksDB rollback in-memory | FIXED | `BatchUndoLog` reverts in-memory caches at `crates/evaporchain-state/src/rocksdb_backend.rs` |
| C-09 DA cert BLS verify | FIXED | `verify_signatures()` + dedicated test suite at `crates/evaporchain-da/src/certificate.rs:56-281` |
| C-10 Plaintext private keys | FIXED | `chacha20poly1305` AEAD + blake3-derived `master_encryption_key` at `crates/evaporchain-node/src/auth.rs:122-132` |
| C-11 Contracts access control | FIXED | `caller != creator` guards on every privileged template method (token mint/burn at `contracts/src/lib.rs:889-944`, NFT/staking/temporal at 1027-1788) returning `ContractError::PermissionDenied` |
| C-12 Block-STM checked arithmetic | FIXED | `checked_add`/`checked_sub` at `crates/evaporchain-execution/src/block_stm.rs:540, 557, 745, 752, 761` |
| C-13 Parser overflow | FIXED | "integer literal overflows u64" error path at `crates/evaporchain-script/src/parser.rs:265` + dedicated regression test `test_c13_integer_overflow_returns_parse_error` |

**Verification method:** direct source grep of each cited file path; no behavioural test run as part of this verification pass — relies on the existing 2,170 unit tests for behaviour confirmation.

---

---

## Executive Summary

**Verdict: NOT production-safe.**

13 CRITICAL vulnerabilities, 23 HIGH, 30+ MEDIUM. Several critical findings are in consensus and execution paths — an attacker with minority stake could forge blocks, steal funds, or crash nodes. The codebase is architecturally ambitious and well-tested for a solo effort, but multiple security-critical paths lack validation that would be expected before any testnet exposure.

**Must fix before testnet:** BLS vote verification, stake-weighted quorum, reentrancy guard, key encryption, gas limit enforcement, parser depth limit.

---

## 1. Documentation vs Reality

The existing docs (ARCHITECTURE.md, THREAT_MODEL.md, AUDIT_SCOPE.md, PROGRESS.md, CRYPTO_SPEC.md) contain significant discrepancies from the actual codebase:

### 1.1 Stale Metrics

| Metric | AUDIT_SCOPE.md Claims | PROGRESS.md Claims | Actual (2026-04-24) |
|--------|-----------------------|---------------------|---------------------|
| Total LOC | ~29,000 | — | **113,143** |
| Total tests | 408 | 3,472+ | **4,668** |
| Number of crates | 13 | 13 | **16** (includes crypto-wasm, oracle, sharding) |
| Script tests | 53 | 53 | **84** |
| Consensus tests | 30 | 96-99 | **285** |
| Crypto tests | 71 | 71 | **153** |

### 1.2 Architecture Mismatches

| Doc Claim | Reality |
|-----------|---------|
| ARCHITECTURE.md: "Rotating Leader Selection" | Code has full Tendermint BFT state machine (Propose→Prevote→Precommit→Commit) |
| ARCHITECTURE.md: "6 pre-built contract templates" | Actually 7 templates (includes Temporal) |
| ARCHITECTURE.md: "91 opcodes" | ~95 opcodes (array ops added, plus recent additions) |
| Pitch deck / external: "MoveVM" | Not present anywhere in codebase — EvaporScript only |
| THREAT_MODEL.md: "Checked arithmetic (errors on overflow)" | Parser silently truncates to 0 on overflow (CRITICAL #13) |
| THREAT_MODEL.md: "Gas deducted per opcode before execution" | gas_limit=0 (unlimited) in ScriptEngine::call() (HIGH) |
| THREAT_MODEL.md: "Gas-proportional [memory] limits" | No memory cap on VM — unbounded allocations possible (HIGH) |
| AUDIT_SCOPE.md: "zero unsafe blocks" | Not verified but likely true for Rust crates; crypto-wasm has FFI boundary |

### 1.3 Threat Model False Positives

The THREAT_MODEL.md lists the following as "Implemented" mitigations, but the audit found them broken or incomplete:

| Claimed Mitigation | Audit Finding |
|--------------------|---------------|
| "2/3+1 threshold correctly computed for validator set" | CRITICAL: Count-based quorum, not stake-weighted |
| "BLS signature verification on votes" | CRITICAL: BLS sigs not verified before counting |
| "Checked arithmetic (errors on overflow)" for EvaporScript | CRITICAL: Integer overflow silently becomes 0 in parser |
| "Gas-proportional limits" for memory | HIGH: No memory cap on VM |
| "Gas consumed per opcode" | HIGH: gas_limit=0 everywhere in ScriptEngine::call() |

---

## 2. CRITICAL Findings (13)

### C-01: Consensus — Count-based quorum, not stake-weighted
- **Location:** `crates/evaporchain-consensus/src/tendermint.rs:487`
- **Impact:** A minority-stake coalition with majority of validator count can finalize blocks. Example: 100 validators with 1 token each outvote 2 validators with 1M tokens each.
- **Fix:** Replace `votes.len() >= threshold` with `votes.iter().map(|v| v.stake).sum() >= stake_threshold`. Effort: 1 day.

### C-02: Consensus — BLS signatures not verified before counting votes
- **Location:** `crates/evaporchain-consensus/src/tendermint.rs:1028`
- **Impact:** Any node can forge votes for any validator. Entire consensus is bypassable — attacker creates fake prevote/precommit messages and reaches quorum.
- **Fix:** Verify BLS signature on each vote message before accepting. Effort: 1 day.

### C-03: Consensus — Zero state_root in proposals
- **Location:** `crates/evaporchain-consensus/src/tendermint.rs:1484`
- **Impact:** Proposals carry `[0u8; 32]` as state root. Validators accept blocks without verifying state correctness. Invalid state transitions are finalized.
- **Fix:** Compute state root from executed transactions before proposing. Effort: 2 days.

### C-04: Script VM — Cross-contract reentrancy
- **Location:** `crates/evaporchain-script/src/lib.rs:306-334`
- **Impact:** Contract A calls Contract B, which calls Contract A again. Contract A's state snapshot is stale from before B's call, enabling double-spend or fund theft.
- **Fix:** Add reentrancy guard (call depth counter or mutex per contract address). Effort: 2 days.

### C-05: Script VM — Map key type collisions
- **Location:** `crates/evaporchain-script/src/lib.rs:60-70`
- **Impact:** `U64(42)` and `Str("42")` hash to the same map key. Attacker can overwrite any map entry by choosing a colliding key of a different type.
- **Fix:** Prefix key hashing with type discriminant byte. Effort: 2 hours.

### C-06: Parser — No recursion depth limit
- **Location:** `crates/evaporchain-script/src/parser.rs:975`
- **Impact:** Deeply nested expression `(((((...))))` crashes the node via stack overflow. Any user can submit a malicious script that takes down validators.
- **Fix:** Add depth counter to `parse_expr()`, reject at depth > 64. Effort: 1 hour.

### C-07: Crypto — Unsafe ML-DSA keypair layout assumption
- **Location:** `crates/evaporchain-crypto/src/signatures.rs:42`
- **Impact:** Keypair bytes split at assumed offset to extract secret key. If `pqc_dilithium` changes internal layout, secret key material could be misinterpreted or leaked.
- **Fix:** Use `pqc_dilithium`'s public API for key extraction instead of byte slicing. Effort: 1 day.

### C-08: State — RocksDB rollback doesn't revert in-memory state
- **Location:** `crates/evaporchain-state/src/rocksdb_backend.rs:234`
- **Impact:** After a RocksDB batch rollback (e.g., failed block), in-memory caches still reflect the rolled-back state. Subsequent reads return stale/invalid data, causing chain split between nodes.
- **Fix:** Clear in-memory cache on rollback or use RocksDB snapshot isolation. Effort: 2 days.

### C-09: DA — Certificate accepts unverified BLS signatures
- **Location:** `crates/evaporchain-da/src/certificate.rs:104`
- **Impact:** DA certificates with forged BLS signatures are accepted as valid. Attacker can claim data availability without actually providing data.
- **Fix:** Verify BLS signature on DA certificate before acceptance. Effort: 1 day.

### C-10: Node — Private keys stored plaintext in SQLite
- **Location:** `crates/evaporchain-node/src/auth.rs:329`
- **Impact:** Wallet private keys (ML-DSA secret keys) stored unencrypted in SQLite database. Any process with file read access can extract all keys.
- **Fix:** Encrypt keys at rest with a user-provided passphrase (argon2 KDF + AES-256-GCM). Effort: 1 day.

### C-11: Contracts — No access control on mint/burn/transfer
- **Location:** `crates/evaporchain-contracts/src/lib.rs:776`
- **Impact:** Template contract actions (mint, burn, transfer) have no owner/authority check. Any caller can mint infinite tokens or burn others' balances.
- **Fix:** Add `msg.sender == contract.owner` check on privileged actions. Effort: 1 day.

### C-12: Execution — Block-STM balance overflow
- **Location:** `crates/evaporchain-execution/src/block_stm.rs:674`
- **Impact:** Parallel execution (Block-STM) uses unchecked arithmetic for balance updates. A crafted set of concurrent transactions could overflow a balance to zero.
- **Fix:** Use `checked_add`/`checked_sub` in Block-STM balance operations. Effort: 1 day.

### C-13: Parser — Integer overflow silently becomes 0
- **Location:** `crates/evaporchain-script/src/parser.rs:265`
- **Impact:** Parsing `99999999999999999999` silently becomes `0`. A contract transferring that amount transfers nothing — or worse, contract logic branches on the zero value.
- **Fix:** Return parse error on overflow instead of defaulting to 0. Effort: 1 hour.

---

## 3. HIGH Findings (23)

### Consensus (5)

| ID | Finding | Location | Impact |
|----|---------|----------|--------|
| H-01 | No validator membership check on vote messages | `tendermint.rs` | Non-validators can cast votes |
| H-02 | Bridge signature bypass — anyone can relay | `tendermint.rs` | Fake bridge messages accepted |
| H-03 | Wrong hash used for finality certificate | `tendermint.rs` | Finality proof doesn't match block |
| H-04 | Wrong lock target in prevote | `tendermint.rs` | Can prevote for block they don't have |
| H-05 | No view-change timeout escalation | `tendermint.rs` | Stuck rounds if proposer is malicious |

### Script VM (5)

| ID | Finding | Location | Impact |
|----|---------|----------|--------|
| H-06 | Neg opcode broken for non-integer types | `vm.rs` | Runtime panic on negating non-numbers |
| H-07 | JumpIf bypasses loop iteration limit | `vm.rs` | Infinite loops via computed jumps |
| H-08 | gas_limit=0 (unlimited) in ScriptEngine::call() | `lib.rs` | No gas limit on user script execution |
| H-09 | No memory cap on VM — unbounded allocations | `vm.rs` | OOM crash via large string/map/array |
| H-10 | String concatenation costs only 3 gas | `vm.rs` | Build gigabyte strings cheaply |

### Compiler (2)

| ID | Finding | Location | Impact |
|----|---------|----------|--------|
| H-11 | Double key evaluation in CompoundAssign MapEntry | `compiler.rs` | Side effects from key expr run twice |
| H-12 | State block overwrites previous state definition | `compiler.rs` | Contract state silently redefined |

### Crypto (3)

| ID | Finding | Location | Impact |
|----|---------|----------|--------|
| H-13 | Unaudited pqc_dilithium crate (not NIST reference) | `signatures.rs` | Unknown implementation quality |
| H-14 | Non-constant-time VRF comparison | `vrf.rs` | Timing side-channel on VRF output |
| H-15 | Unaudited Poseidon round constants | `hash.rs` | Constants derived from BLAKE3, not independently verified |

### State (3)

| ID | Finding | Location | Impact |
|----|---------|----------|--------|
| H-16 | Ghost bridge has no signature verification | `bridge.rs` | Fake ghost resurrections |
| H-17 | Dirty state not flushed before snapshot | `state.rs` | Snapshot captures partial writes |
| H-18 | Snapshot doesn't clear existing entries | `state.rs` | Stale data persists across snapshots |

### Proving (1)

| ID | Finding | Location | Impact |
|----|---------|----------|--------|
| H-19 | MockProver leaks to production code path | `nova.rs` | Production blocks accepted without real proof |

### Network (2)

| ID | Finding | Location | Impact |
|----|---------|----------|--------|
| H-20 | No rate limiting on any endpoint | `service.rs` | DoS via message flood |
| H-21 | No sync validation — accept any block at any height | `sync.rs` | Invalid chain state from malicious peers |

### Node (2)

| ID | Finding | Location | Impact |
|----|---------|----------|--------|
| H-22 | Unauthenticated oracle endpoint | `oracle.rs` | Anyone can push price data |
| H-23 | World-readable keygen output files | `keygen.rs` | Private keys readable by any local user |

---

## 4. MEDIUM Findings (30+)

### Consensus
- Timeouts not randomized — predictable scheduling attacks
- No block size limit validation
- Leader rotation deterministic without VRF — predictable leader
- Epoch transition can be triggered by any node

### Script VM
- `push()` builtin has no length cap — quadratic memory
- `keys()` / `values()` allocate full copy of map
- No fuel/step limit independent of gas
- `to_string()` on arrays shows `[N items]` — not inspectable
- Contract storage unbounded per contract

### Compiler
- No dead code elimination
- Type checker is shallow (no flow analysis)
- Struct/enum support absent — complex data requires maps

### Crypto
- BLS key aggregation doesn't check proof-of-possession
- Verkle trie not benchmarked under adversarial load
- MMR peak calculation untested beyond 1000 leaves

### State
- No state pruning — disk grows unbounded
- Evaporation engine processes all objects every epoch (O(n))
- No WAL for state transitions — crash = inconsistency

### Execution
- Block-STM conflict resolution untested under adversarial tx ordering
- Fee controller PID gains not empirically tuned
- No block reward halving or emission schedule implemented

### Network
- No peer scoring or banning
- Gossip propagation not tested beyond 4 nodes
- mDNS discovery unsuitable for production (LAN only)

### Node
- Dashboard serves over HTTP (no TLS)
- Faucet has no rate limiting
- API has no authentication

---

## 5. Crate-by-Crate Status

| Crate | LOC | Tests | Critical | High | Production Ready? |
|-------|-----|-------|----------|------|-------------------|
| evaporchain-consensus | 14,991 | 285 | 3 | 5 | NO |
| evaporchain-execution | 10,393 | 163 | 1 | 0 | NO |
| evaporchain-node | 11,494 | 31 | 1 | 2 | NO |
| evaporchain-script | 5,497 | 84 | 3 | 5 | NO |
| evaporchain-crypto | 4,862 | 153 | 1 | 3 | NO |
| evaporchain-state | 4,102 | 93 | 1 | 3 | NO |
| evaporchain-proving | 5,798 | 94 | 0 | 1 | PARTIAL |
| evaporchain-da | 3,912 | 79 | 1 | 0 | NO |
| evaporchain-contracts | 2,906 | 40 | 1 | 0 | NO |
| evaporchain-types | 1,538 | 25 | 0 | 0 | YES |
| evaporchain-network | 1,468 | 12 | 0 | 2 | NO |
| evaporchain-cli | 1,363 | 28 | 0 | 0 | YES |
| evaporchain-oracle | 2,090 | 60 | 0 | 1 | NO |
| evaporchain-crypto-wasm | 41,298 | 3,491 | 0 | 0 | PARTIAL |
| evaporchain-mcp | 771 | 0 | 0 | 0 | NO (no tests) |
| evaporchain-sharding | 660 | 30 | 0 | 0 | PARTIAL |

---

## 6. Layer-by-Layer Production Readiness

### Layer 1: Cryptography — 60% ready
- BLAKE3: production-grade (library)
- ML-DSA: functional but depends on unaudited crate, unsafe byte slicing
- BLS12-381: functional, missing proof-of-possession
- Poseidon: custom implementation, unverified constants
- Verkle: functional, needs adversarial testing
- MMR: functional, needs scale testing

### Layer 2: Consensus — 30% ready
- Tendermint BFT state machine: structurally complete
- BLS vote verification: BROKEN (not verified before counting)
- Quorum: BROKEN (count-based, not stake-weighted)
- State root in proposals: BROKEN (zero)
- Validator membership: NOT CHECKED
- Slashing: implemented but bypassed via fake votes

### Layer 3: Execution — 50% ready
- Sequential execution: functional with checked arithmetic
- Block-STM parallel: has overflow bug
- Fee controller: functional, untuned
- Gas metering: per-opcode costs exist but gas_limit=0 in script calls

### Layer 4: Smart Contracts — 40% ready
- EvaporScript VM: functional with 95 opcodes
- Parser: missing depth limit, integer overflow bug
- Compiler: double evaluation bugs
- Cross-contract calls: REENTRANCY vulnerability
- Template contracts: no access control
- Gas: not enforced at ScriptEngine level

### Layer 5: State Management — 45% ready
- In-memory StateDB: functional
- RocksDB: rollback bug (in-memory not reverted)
- Evaporation engine: functional but O(n) scaling
- Ghost records + MMR: functional
- No pruning, no WAL

### Layer 6: Networking — 25% ready
- libp2p + gossipsub: basic functionality
- No rate limiting, no peer scoring, no sync validation
- mDNS only (no production discovery)
- Only tested with 4 nodes

### Layer 7: Data Availability — 35% ready
- 2D erasure coding: implemented
- DA certificates: BROKEN (unverified BLS sigs)
- Sampling: not implemented

### Layer 8: Node Infrastructure — 30% ready
- API endpoints: functional but unauthenticated
- Key storage: PLAINTEXT (critical)
- Dashboard: HTTP only
- Faucet: no rate limiting
- Genesis: functional

---

## 7. Top 10 Fix Priorities

### P0 — Must fix before any testnet (estimated 7 days)

| # | Fix | Effort | Blocking |
|---|-----|--------|----------|
| 1 | Verify BLS signatures before vote counting (C-02) | 1 day | Entire consensus is bypassable |
| 2 | Stake-weighted quorum instead of count-based (C-01) | 1 day | Minority takeover possible |
| 3 | Add reentrancy guard to cross-contract calls (C-04) | 2 days | Fund theft |
| 4 | Encrypt private keys at rest (C-10) | 1 day | Total wallet compromise |
| 5 | Set gas_limit > 0 in ScriptEngine::call() (H-08) | 0.5 days | Unlimited computation |
| 6 | Parser recursion depth limit (C-06) | 1 hour | Node crash DoS |
| 7 | Fix integer overflow → 0 in parser (C-13) | 1 hour | Silent fund loss |

### P1 — Must fix before public testnet (estimated 5 days)

| # | Fix | Effort | Blocking |
|---|-----|--------|----------|
| 8 | DA certificate BLS verification (C-09) | 1 day | Fake availability proofs |
| 9 | Map key type-prefixed hashing (C-05) | 2 hours | Key collision attacks |
| 10 | Contract access control on mint/burn (C-11) | 1 day | Infinite mint |

### P2 — Must fix before mainnet (estimated 8 days)

- RocksDB rollback + in-memory cache fix (C-08) — 2 days
- Block-STM checked arithmetic (C-12) — 1 day
- ML-DSA keypair API (C-07) — 1 day
- Compute real state_root in proposals (C-03) — 2 days
- Validator membership check (H-01) — 0.5 days
- MockProver → real prover in production path (H-19) — 1 day
- VM memory cap (H-09) — 0.5 days

---

## 8. Recommended 6-Month Sprint Sequence

### Month 1 (May 2026): P0 Consensus + VM Hardening
- Fix C-01, C-02, C-03 (consensus)
- Fix C-04, C-05, C-06, C-13 (script VM)
- Fix H-08 (gas limit)
- Add integration tests for each fix

### Month 2 (June 2026): P1 + State Layer
- Fix C-08, C-10, C-11
- Fix C-09 (DA cert)
- Fix H-16, H-17, H-18 (state)
- Add WAL for state transitions
- Begin state pruning design

### Month 3 (July 2026): Execution + Crypto
- Fix C-07, C-12
- Fix H-13, H-14, H-15 (crypto)
- Replace pqc_dilithium with NIST reference implementation or audited crate
- Tune PID fee controller with simulations
- Add BLS proof-of-possession

### Month 4 (August 2026): Network + Node Hardening
- Fix H-20, H-21 (network)
- Fix H-22, H-23 (node)
- Add TLS to dashboard/API
- Implement peer scoring + banning
- Replace mDNS with DHT-based discovery for production
- Add API authentication

### Month 5 (September 2026): DA Completion + Multi-Node Testing
- Complete DA sampling
- 10+ node testnet deployment on Minis
- Adversarial testing (byzantine validators, network partitions)
- Stress testing under load with all fixes applied
- Fuzz testing with cargo-fuzz

### Month 6 (October 2026): External Audit + Launch Prep
- Engage Trail of Bits or Zellic (per AUDIT_SCOPE.md recommendations)
- Remediate audit findings
- Update all documentation to match reality
- Genesis config finalization
- Testnet launch

---

## 9. Documentation Fixes Required

The following docs must be updated to reflect reality:

| Document | Issues |
|----------|--------|
| AUDIT_SCOPE.md | LOC: 29K → 113K. Tests: 408 → 4,668. Crates: 13 → 16. Add oracle, sharding, crypto-wasm |
| THREAT_MODEL.md | Remove "Implemented" from 5+ mitigations that are broken. Add reentrancy, key storage, gas limit as known gaps |
| ARCHITECTURE.md | Update "Rotating Leader" to "Tendermint BFT". Templates: 6 → 7. Opcodes: 91 → ~95 |
| PROGRESS.md | Test counts stale. Update to actual per-crate numbers |
| CRYPTO_SPEC.md | Add note that Poseidon constants are BLAKE3-derived (not independently verified). Add pqc_dilithium caveat |
| CLAUDE.md (project) | Test count: 4,159 → 4,668 |

---

## 10. What Works Well

Despite the critical findings, the codebase demonstrates:

- **Architectural ambition:** Tendermint BFT + Nova IVC + ML-DSA + EvaporScript VM + Energy Decay is genuinely novel
- **Test depth:** 4,668 tests for a solo project is exceptional; includes proptest, fuzzing, adversarial scenarios
- **Performance:** 468K TPS peak (single-node) shows the execution engine is well-optimized
- **Thermodynamic model:** The evaporation/grace/ghost lifecycle is correctly implemented and well-tested
- **Ecosystem breadth:** CLI wallet, browser extension, mobile wallet, 4 dApps, SDK — full stack
- **ZK integration:** Nova IVC wired into block pipeline with light client API

The foundation is strong. The issues are primarily at security boundaries (input validation, authentication, verification) rather than core logic errors. This is fixable.

---

*Generated by 12 parallel audit agents covering all 16 crates. Based on static analysis of 113,143 LOC.*
*This report supplements the existing THREAT_MODEL.md and AUDIT_SCOPE.md — it does not replace them.*
