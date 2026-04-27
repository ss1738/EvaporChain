# EvaporChain — Audit-Readiness Pack

**Document version:** 2026-04-27
**Audience:** prospective external security auditors (NDA-protected)
**Companion documents:**
- `cross_verification_2026_04_27.md` — currently-tracked findings the auditor should be aware of
- `external_audit_rfp_2026_04_27.md` — engagement brief, scope, and shortlist
- `FULL_AUDIT_2026_04_24.md` (repo root) — prior internal multi-agent audit

This pack is what every serious auditor asks for before pricing the engagement. It is a snapshot of the system today, not a marketing document.

---

## 1. Trust model

**Honest-majority assumption:** ≥ 2/3 of total stake is honest at any time.

**Trusted parties:**
- Genesis validator set (initial stake distribution).
- Trusted checkpoint (`--checkpoint-height` + `--checkpoint-state-root`) for weak-subjectivity defence against long-range attacks.
- Operator of the deployment binary on each validator host.
- The host operating system (no TEE, no secure enclave assumed).

**Untrusted parties:**
- Up to 1/3 of stake may be byzantine (equivocate, withhold, censor, replay).
- All P2P network participants — assume libp2p peers may be malicious.
- All transaction submitters and contract callers.
- Light clients — assumed honest about their own state but not relied on for validity.

**Crypto assumptions:**
- BLS12-381 pairing-based signatures are unforgeable in the standard model with the random-oracle assumption.
- BLAKE3 is a collision-resistant hash function.
- ML-DSA (Dilithium3) is unforgeable under chosen-message attack at NIST PQC Level 3.
- Poseidon is collision-resistant for the parameter set in use (current parameters are non-standard — see §5 known issues).
- Nova / arkworks groups behave as ideal cryptographic primitives.

**Storage assumptions:**
- RocksDB write-ahead log is durable across power loss.
- File-system mode 0600 is enforced by the host OS.

## 2. Attacker capabilities

| Capability | Defended by |
|------------|-------------|
| Submit forged transactions | ML-DSA / ECDSA signature verification + per-account nonce |
| Submit transactions claiming another sender | Sender-derived address from public key |
| Replay old transactions | Per-account nonce + chain_id |
| Replay old votes / commits | Vote height validation + duplicate-signer guards (consensus); current oracle path is broken — see cross-verification |
| Equivocate at the same height | Detected and slashable (not yet automatic — see §5) |
| Withhold blocks (censorship) | View-change in Tendermint; slashing for downtime via vote-liveness slashing |
| Eclipse-attack a peer | Multiple bootstrap peers + libp2p Kademlia; not yet hardened against sybil discovery |
| DoS the mempool | Per-account limits, TTL eviction, gas cap, signature verification before admit |
| Re-finalize a lower height with replayed certificate | Currently only partially defended — finality monotonicity removed; gap-fill replays insert ghost records (cross-verification §1) |
| Forge oracle votes | NOT defended — see cross-verification §2 (CRITICAL open) |
| Upgrade a contract without authorization | Outcome unclear — see cross-verification §3 |
| Read validator BLS private key from disk | OS file mode 0600 only; not encrypted (cross-verification §4) |
| Long-range attack | Trusted checkpoint with weak-subjectivity period |
| Re-org via stake withdrawal | Validator unbonding period enforcement |
| State-sync poisoning | Sync chain-tip validation + quorum check (cross-verification §5 needs read) |
| Cross-shard message replay | Receipt root deduplicates by message_id before Merkle computation |

## 3. In-scope vs out-of-scope

### In-scope (audit these)

| Crate | LOC | Tests | Status |
|-------|-----|-------|--------|
| evaporchain-consensus | 13,900 | 258+ | Complete — Tendermint BFT, validator sets, finality, DA attestation, light clients |
| evaporchain-execution | 10,500 | 165+ | Complete — STM, parallel, fees, rewards, privacy, MMR, evaporation |
| evaporchain-node | 9,500 | 28+ | Complete — API, persistence, key load (validator + wallet), oracle/shard bridges |
| evaporchain-proving | 5,600 | 95+ | Complete — Nova IVC, ZK evaporation proofs |
| evaporchain-crypto | 4,746 | 149 | Complete — ML-DSA, BLS, BLAKE3, Verkle, MMR, EnergyVerkleTrie |
| evaporchain-script | 4,452 | 65 | Partial — EvaporScript VM, 44 opcodes (`compiler.rs:11 enum Op`) |
| evaporchain-da | 3,316 | 66 | Library complete; integration into block production gap |
| evaporchain-state | 3,400 | 83+ | Complete — RocksDB backend, evaporation, ghost bridge |
| evaporchain-contracts | 2,897 | 40 | Complete — 6 template contracts + rule engine |
| evaporchain-types | 1,600 | 25 | Complete |
| evaporchain-oracle | 1,400 | 60+ | Complete — current vote-verification path broken (cross-verification §2) |
| evaporchain-network | 1,017 | 8 | Has 8 tests; TLS validator keys at plaintext |
| evaporchain-sharding | 700 | 30 | Complete — assignment, cross-shard routing, compaction |

**Total in-scope:** ~63K LOC, ~1,100 tests in critical-path crates.

### Out-of-scope (excluded unless added by addendum)

- `evaporchain-cli` (1,363 LOC) — operator UX
- `evaporchain-mcp` (771 LOC, 0 tests) — stub
- `evaporchain-crypto-wasm` (143 LOC) — browser bindings (only relevant if key handling in browser is in threat model)
- `wallet/`, `wallet-sdk/`, `mobile-wallet/`, `extension/` — client UX
- `dapps/` — example dapps
- `sdk/` (TypeScript) — protocol invariants are in-scope as enforced by node, not SDK
- Frontend explorer (static HTML)
- Tokenomics parameter calibration (auditors may flag, project owns)

## 4. Test corpus

- **Total tests:** 4,486+ as of 2026-04-27 (README.md current value).
- **Adversarial / byzantine scenarios:** ≥19 dedicated tests in consensus + execution.
- **Property-based:** `proptest` harnesses in execution, consensus, script.
- **Fuzz harnesses:** `fuzz/` directory at repo root — targets for parser, deserialization, opcodes.
- **Coverage:** report not yet generated. **Action:** run `cargo-llvm-cov --workspace` on a Mini before audit kickoff and supply HTML report.

## 5. Known issues / risk-acceptance items

Auditors should not flag these as novel — they are tracked.

| ID | Item | Status | Risk taken |
|----|------|--------|-----------|
| H-13 | `pqc_dilithium` upstream crate is itself unaudited | Pinned version; no in-house alternative | NIST PQC Level 3 implementation risk |
| H-15 | Poseidon constants are non-standard | Pending RFC alignment | Theoretical: collision-resistance with non-standard params |
| K-01 | ~~MockConsensus is the binary default~~ | **RESOLVED 2026-04-27 (commit 4afe27f).** Tendermint is the binary default; `--mock-consensus` is opt-in (`main.rs:751`). `--mainnet` strict mode hard-fails on `--mock-consensus`. | — |
| K-02 | ~~`bls_key.bin` validator key plaintext on disk, mode 0600 only~~ | **RESOLVED 2026-04-27 (commit 0af4bb2).** Opt-in EVK1 encryption (Argon2id + XChaCha20-Poly1305) via `EVAPORCHAIN_VALIDATOR_KEY_PASS`; `--mainnet` strict mode requires it. | — |
| K-03 | ~~`EVAPORCHAIN_KEY_MASTER` env var defaults to dev string~~ | **RESOLVED 2026-04-27 (commit 4afe27f).** `--mainnet` strict mode hard-fails on unset, dev-default, or sub-16-char value. | — |
| K-04 | ~~DA layer 2D erasure exists but not wired into `produce_block`~~ | **RESOLVED 2026-04-27 (commit 1fc67c0).** `compute_block_da` calls `BlockDA2D::encode_block_with_blobs` from `MockConsensus::produce_block`, `produce_block_with_reveals`, and `RotatingConsensus::produce_block_if_leader`. Empty blocks still use sentinel data_root. Tendermint already had its own wiring (`tendermint.rs:1958-2030`). | — |
| K-05 | ~~Equivocation slashing not yet automatic~~ | **RESOLVED — already wired before audit pack capture.** `slash_equivocation` is invoked in-line at all three detection sites: proposal (`tendermint.rs:1113`), prevote (`tendermint.rs:1377`), precommit (`tendermint.rs:1473`). Penalty applies stake reduction + jail + auto-remove below MIN_STAKE in one call (`validator_set.rs:341-356`). Three regression tests cover the path. | — |
| K-06 | Cross-verification §1-§4 findings | All four resolved 2026-04-27 (commits 674be1d, c49a2fe, 0af4bb2). See `cross_verification_2026_04_27.md` for original details. | — |
| K-07 | Multi-validator BFT cluster splits without a shared genesis | **Open as of 2026-04-27.** Stress run 20260427-215707 launched 4 nodes locally with `--validators 4 --validator-id N` and *no* `--genesis-config`. Each loaded its own hardcoded genesis; the cluster split 2-vs-2 (nodes 0+3 advanced ~3.3 b/s with state_root `7c077440…`, nodes 1+2 stalled at height 453 with state_root `a50837d4…`). All four reported `peer_count=3` so networking was healthy — failure is in genesis-derived validator-set agreement. Mainnet launch must use a single `--genesis-config` file shared across all validators; ops runbook should hard-fail the launcher if `--validators > 1` and no shared genesis is supplied. | Cluster won't produce a usable chain; group A's "advancing" state has no real 2/3 quorum |

## 6. Invariant catalogue

The following must hold under any execution. Auditors are invited to challenge each.

### Consensus

- I-CONS-01: Block finalized only when `signing_stake * 3 ≥ total_stake * 2`. (`bridge.rs:79`)
- I-CONS-02: Aggregate BLS signature verifies against committed validator-set pubkey before block accepted. (`bridge.rs:82-103`, `da_attestation.rs:149`)
- I-CONS-03: `state_root` is part of the proposal payload, not derived after the fact. (`bridge.rs:116`, Block struct)
- I-CONS-04: `latest_finalized` is monotone non-decreasing. (`finality.rs:189`) **— note: the records map is currently NOT monotone after `d70ab4c`; see cross-verification §1.**
- I-CONS-05: Validator unbonding period enforced before stake withdrawal.
- I-CONS-06: Trusted checkpoint defines weak-subjectivity period for long-range defence.
- I-CONS-07: Equivocation produces detectable evidence; slashing condition correctly identifies the offence.
- I-CONS-08: Vote height validation prevents acceptance of votes for wrong height.

### Execution

- I-EXEC-01: Per-account nonce strictly increases by 1 per accepted transaction.
- I-EXEC-02: Reentrancy bounded by `MAX_CALL_DEPTH`. (`execution/lib.rs:141,643`)
- I-EXEC-03: All balance arithmetic uses `checked_add` / `saturating_sub` — no silent overflow. (`block_stm.rs:540,728-737,851,876`)
- I-EXEC-04: Block-STM serial-fallback determinism: parallel and serial paths produce identical state on the same input.
- I-EXEC-05: Failed tx reverts state but keeps fee burn. (`lib.rs:1334-1348`)
- I-EXEC-06: Multisig signature set deduplicated, all signers in authorized signers list. (`lib.rs:973-985`)
- I-EXEC-07: UserOp paymaster charged `call_gas_limit + GAS_USER_OP`, balance check precedes deduction. (`lib.rs:1013-1024`)
- I-EXEC-08: Storage rent / decay applied each block according to per-object decay curve.

### Cryptography

- I-CRYP-01: ML-DSA secret keys zeroized on Drop with volatile writes. (`crypto/signatures.rs:39-49`)
- I-CRYP-02: All keypair generation uses `OsRng` (in-progress; uncommitted).
- I-CRYP-03: BLS aggregate signatures verify against a committed validator-set pubkey, not a vote-supplied pubkey.
- I-CRYP-04: Poseidon hash is collision-resistant for the parameter set in use (caveat K-H-15).

### State

- I-STATE-01: Address space partitioned by hash-prefixed keys (`b"acct"` / `b"obj"` + blake3). (`state/db.rs:10-36`)
- I-STATE-02: Failed block apply triggers RocksDB rollback; in-memory state restored to pre-block snapshot. (`rocksdb_backend.rs:277-309`)
- I-STATE-03: Verkle trie membership proofs verify against the committed root.
- I-STATE-04: MMR append maintains correct cumulative hash.
- I-STATE-05: Snapshot pruning never removes ranges still required for state-sync requests.

### Script VM

- I-VM-01: Stack depth ≤ `MAX_STACK_DEPTH` (1024). (`script/vm.rs:34,111-113`)
- I-VM-02: Jump opcodes validate bounds before transfer of control.
- I-VM-03: All arithmetic opcodes use `checked_add` / `checked_mul`; overflow returns explicit error.
- I-VM-04: Gas metered per opcode; out-of-gas returns explicit error before VM state change.
- I-VM-05: Call depth bounded; recursion does not unwind the host stack.

### DA

- I-DA-01: DA certificate verified to have ≥ 2/3 stake-weighted BLS signatures before acceptance. (`da/poha.rs:131-133`, `da_attestation.rs:279`)
- I-DA-02: 2D erasure encoding produces valid row/col commitments and per-cell proofs.
- I-DA-03: Light-client samples deterministically reproducible from `data_root`.
- I-DA-04: **Not yet enforced:** every produced block's `data_root` is computed from real 2D erasure, not the sentinel hash.

### Oracle

- I-OR-01: Oracle vote authenticity — vote signed by the validator whose ID it carries.  **— currently broken; see cross-verification §2.**
- I-OR-02: Round mismatch rejected.
- I-OR-03: Duplicate voter per round rejected.
- I-OR-04: Median / TWAP rejects values outside `max_spread_pct` of the cohort.
- I-OR-05: Outlier rejection rule (`outlier_factor`) applied before aggregation.

### Network

- I-NET-01: Mempool admission verifies signatures before pooling. (`6cc8e2d`)
- I-NET-02: Per-account mempool quota prevents single-sender DoS. (`3810495`)
- I-NET-03: TTL eviction caps mempool memory.
- I-NET-04: Connection error events are logged, not silently swallowed. (`2026-04-26 hardening`)

### Cross-shard

- I-XS-01: Receipt root deduplicates by message_id before Merkle computation. (`87c8e1c`)
- I-XS-02: Cross-shard messages cannot be replayed across epochs.

## 7. Build, test, deploy reproducibility

```sh
# Build (run on Mini, NOT MacBook — see CLAUDE.md constraint)
cargo build --workspace --release

# Run full test suite
cargo test --workspace

# Run fuzz targets
cd fuzz && cargo +nightly fuzz run <target>

# Single-node devnet
./target/release/evaporchain-node \
  --port 9000 --validators 1 --stake 1000 \
  --network --no-da-enforcement --demo --api

# 3-node Tendermint testnet (current Tailscale deployment)
./target/release/evaporchain-node \
  --port 9000 --validators 3 --stake 1000 \
  --network --tendermint-mode --no-da-enforcement \
  --demo --api \
  --bootstrap /ip4/<peer_ip>/tcp/9000
```

Toolchain: Rust 1.75+ (workspace `rust-toolchain.toml` to be added). Genesis files at repo root: `genesis-mainnet.json`, `genesis-tailscale-3node.json`.

## 8. Critical dependencies

| Dependency | Purpose | Notes for auditor |
|------------|---------|-------------------|
| `pqc_dilithium` | ML-DSA signatures | Upstream unaudited (H-13) |
| `blstrs` | BLS12-381 | Pinned; widely used |
| `arkworks` ecosystem | Pairing-friendly groups | Multiple sub-crates |
| `nova-snark` | Nova IVC proofs | Active research code |
| `rocksdb` | Persistent state | Vendored bindings |
| `libp2p` | P2P networking | Specific transport set in use |
| `chacha20poly1305` | Wallet key encryption | Standard implementation |
| `bcrypt` | Password hashing | cost=10 in user-auth |
| `axum` / `tokio` | API + async runtime | |

`Cargo.lock` is the source of truth; supply this file to auditors.

## 9. Architecture diagrams (TODO before RFP issue)

To be produced before audit kickoff:
- D-01: Transaction lifecycle (submit → mempool → block → execute → finalize → DA attest).
- D-02: Consensus state machine (propose → prevote → precommit → commit).
- D-03: DA flow (encode → row/col commitments → cell proofs → light-client sample → certificate).
- D-04: Validator key lifecycle (generate → store → load → sign → rotate).
- D-05: Cross-shard messaging (origin shard → receipt → destination shard execute).

## 10. Operational parameters (TODO — to be tabulated)

| Parameter | Current value |
|-----------|---------------|
| Block time target | TBD |
| Max gas per block | per `--block-gas-limit` flag |
| Validator unbonding period | TBD (recently added in `bb65654`) |
| Slashing % for double-vote | TBD |
| Slashing % for downtime | TBD (added in `f9ef6c8`) |
| Epoch length | TBD |
| Max blob size | 128 KiB (per `87c8e1c`) |
| Max validator set size | TBD |
| Reward per block | per `RewardAccumulator` (wired in `0cbb859`) |
| Fee burn ratio | TBD |
| `MAX_CALL_DEPTH` | per `execution/lib.rs:141` |
| `MAX_STACK_DEPTH` | 1024 (`script/vm.rs:34`) |
| Max records in `FinalityTracker` | 10,000 (`finality.rs:126`) |

To be filled in by reading the constants out of source and confirming with operator-facing CLI defaults.

## 11. Pre-RFP checklist

In approximate priority order:
- [ ] Resolve all CRITICAL items in `cross_verification_2026_04_27.md`.
- [ ] Resolve HIGH items or document as accepted risk if deferred.
- [ ] Generate code-coverage report.
- [ ] Produce architecture diagrams D-01 through D-05.
- [ ] Tabulate all values in §10.
- [ ] Wire `BlockDA2D::encode_block()` into block production.
- [ ] Pin `pqc_dilithium` commit; document upstream-audit status.
- [ ] Encrypt `bls_key.bin` (mainnet gate; not strictly required for testnet).
- [ ] Mutual NDA template ready.
- [ ] Decide budget envelope.
