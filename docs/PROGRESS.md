# EvaporChain Development Progress

## Overview

Novel L1 blockchain with energy-based state decay. 147 Rust workspace crates (per `Cargo.toml` members + filesystem; full 5-Tier-0 + 7-Tier-0-supporting frontier-primitive corpus shipped 2026-05), 25,435+ native test markers (`#[test]` + `#[tokio::test]` + `#[proptest]`), post-quantum signatures (ML-DSA), Tendermint BFT + Light-Cone Full DAG consensus with BLS12-381 aggregate signatures, browser extension, mobile wallet, SDK, 4 dApps.

**Repo:** github.com/ss1738/EvaporChain

---

## 7-Phase Roadmap

### Phase 1: Critical Fixes — COMPLETE
- Signature verification corrections
- Gas limit enforcement
- Overclaim prevention

### Phase 2: Real Consensus — COMPLETE (2026-04-10)
| Task | Status | Commits |
|------|--------|---------|
| Tendermint BFT state machine | Done (pre-existing) | Full Propose→Prevote→Precommit→Commit, 2f+1 quorum |
| BLS aggregate signatures in voting | Done | `c7c5c9e` — CommitCertificate on blocks, sign/aggregate/verify, non-BLS fallback |
| Dynamic validator set | Done | `26cdb0f` — EpochTransitionManager, bonding/unbonding periods, churn limits, min-validator safety |
| Consensus state persistence | Done | `32a0128` — ConsensusStateStore trait, file-based checkpoint + WAL, atomic writes, crash recovery |
| 4-node devnet stress test | Done | `c9ea71a` — 5-phase stress-test.sh + check-consensus.sh |
| libp2p networking | Done (pre-existing) | gossipsub consensus topic, mDNS, block sync |
| Slashing (equivocation + downtime) | Done (pre-existing) | 10% equivocation slash, 1%/miss downtime, jailing |
| 3-node BFT fault tolerance | Done (2026-04-24) | 3 Mac Minis, kill/rejoin cycle proven, identical state roots |
| Consensus meta persistence | Done (2026-04-24) | parent_hash/height persisted across all block commit paths |
| VRF-seeded leader selection | Done (2026-04-24) | Beacon randomness mixed into proposer selection |

**Test count:** 288 tests in consensus crate (all passing)

### Phase 3: Complete Nova Proving System — COMPLETE (2026-04-10)
| Task | Status | Details |
|------|--------|---------|
| nova_proof field on Block | Done | `Option<Vec<u8>>` on Block struct, serde-gated |
| ChainProver in block pipeline | Done | Wraps ProvingEngine with checkpointing (every 100 blocks), replaces raw prover in all 3 commit paths |
| Proof generation on commit | Done | Compressed Nova proof attached to block.nova_proof after execution |
| ProofVerifier trait in consensus | Done | Validators verify nova_proof on proposals; reject invalid proofs |
| Light client API | Done | `/api/proof/latest`, `/api/proof/status`, `/api/proof/verify` |
| Proof metadata in BlockRecord | Done | `has_nova_proof`, `nova_proof_size` exposed in API |
| Tests | Done | 3 new consensus tests (valid proof, invalid proof, no-verifier fallback) |

**Test count:** 99 tests in consensus crate (3 new), 46 in proving, 25 in types

### Phase 4: Smart Contract Story — COMPLETE (2026-04-10)
**Decision: EvaporScript** (custom non-Turing-complete VM, already built)

| Task | Status | Details |
|------|--------|---------|
| VM choice | Done | EvaporScript — 44 opcodes, stack-based, deterministic, energy-aware |
| Parser + Compiler | Done (pre-existing) | Source → AST → bytecode, state schema declaration |
| EvaporVM execution | Done (pre-existing) | Gas-metered, bounded loops (100K), max stack 1024 |
| Template contracts | Done (pre-existing) | 7 templates (DecayingToken, MortalNFT, Escrow, Auction, Staking, DAO, Temporal) |
| Deploy/call lifecycle | Done | Full end-to-end: API → mempool → executor → VM → state |
| Script API endpoints | Done | `/api/tx/deploy-script`, `/api/tx/call-script`, `/api/scripts`, `/api/script/:id` |
| Gas metering | Done (pre-existing) | Per-opcode costs, GAS_DEPLOY_SCRIPT=150K, GAS_CALL_SCRIPT=50K |
| Lifecycle hooks | Done (pre-existing) | `on_evaporate()`, `on_grace()`, `on_refresh()` |
| list() for ScriptEngine | Done | Query all deployed scripts via API |

**Test count:** 84 script VM tests, 288 consensus tests

### Phase 5: Stress Testing — COMPLETE (2026-04-10)
**Target: 1000+ TPS — ACHIEVED (468,385 TPS peak, 6,978 sustained multi-block)**

| Task | Status | Details |
|------|--------|---------|
| Tunable block gas limit | Done | `--block-gas-limit` CLI arg, default 500K |
| High-throughput preset | Done | `--high-throughput` flag: 10M gas, 200ms blocks |
| ThroughputTracker | Done | Rolling 10-second TPS window, 100-block buffer, peak tracking |
| `/api/metrics` endpoint | Done | Real-time TPS, peak TPS, avg block exec time, avg gas/block |
| Execution timing | Done | Microsecond timing on all 4 block execution paths |
| Stress benchmarks (8 tests) | Done | 50K transfer stress (468K TPS), mixed workload (12K), sustained multi-block (7K) |
| Gas limit integration | Done | `new_with_gas_limit()` on both MockConsensus and TendermintConsensus |

**Benchmark results (release mode, single node):**
- Transfer throughput: 146,871 TPS (10K txs)
- Stress test: 468,385 TPS (50K txs single block)
- Mixed workload: 12,066 TPS (5K transfers + 2K creates + 2K refreshes)
- Sustained multi-block: 6,978 TPS (50 blocks × 500 txs)
- Decay engine: 351M objects/s
- Block execution: 1.72ms/block

### Phase 6: Audit Prep — COMPLETE (2026-04-10)

| Task | Status | Details |
|------|--------|---------|
| Security documentation | Done (pre-existing) | AUDIT_SCOPE.md, CRYPTO_SPEC.md, THREAT_MODEL.md |
| Proptest expansion | Done | Property-based testing added to consensus, execution, script crates (was only in crypto+types) |
| Byzantine fault tests | Done | Silent validators (1/4, 2/4), equivocation detection, unknown validator rejection |
| Consensus safety invariants | Done | All-nodes-same-block, multi-height liveness, height/round monotonicity, nil vote safety |
| Message attack resistance | Done | Wrong height, past height, duplicate vote, mempool flooding tests |
| Execution invariants | Done | Balance conservation, replay prevention, nonce gaps, overdraft, gas limit enforcement |
| Energy decay invariants | Done | Monotonic decay, evaporation-to-ghost lifecycle |
| VM safety tests | Done | Gas exhaustion, loop iteration cap, malformed input, stack isolation, state isolation |
| Parser fuzzing | Done | Random string fuzzing (proptest), garbage input, empty source, large energy values |
| Determinism verification | Done | Same inputs → same state root (proptest) |

**New tests added:** 30+ audit-specific tests across 3 crates
- Consensus: 14 tests (11 adversarial + 3 proptest)
- Execution: 12 tests (8 invariant + 4 proptest)
- Script: 12 tests (9 VM safety + 3 proptest)

### Phase 7: Mainnet Genesis — COMPLETE (2026-04-10)
| Task | Status | Details |
|------|--------|---------|
| Genesis config (genesis-mainnet.json) | Done | 4 validators (250K stake each), 8 accounts (1B supply), chain params, tokenomics |
| Genesis block generation | Done | Deterministic `initialize_genesis()`, offline `genesis init` command, state root verification |
| CLI genesis tools | Done | `genesis validate`, `genesis show`, `genesis init` — offline config validation and block generation |
| CLI keygen | Done | `keygen` — generates BLS + ML-DSA + VRF keypair bundle for validator onboarding |
| Node `--genesis-config` flag | Done | Node bootstraps from JSON genesis config instead of hardcoded state |
| Mainnet launch script | Done | `scripts/launch-mainnet.sh` — validator launcher with genesis config, bootstrap peers |
| Devnet genesis support | Done | `GENESIS_CONFIG=path launch-devnet.sh` — devnet can use custom genesis |
| Validator stake/exit execution | Done (pre-existing) | `execute_validator_stake()`, `execute_validator_exit()`, epoch transitions |

**New tests added:** 10 CLI tests (genesis validate/show/init, keygen, determinism)
**Test count:** 28 CLI tests, 12 node tests (all passing)

---

## Frontier Features

### Phase-bracket primitives
| Feature | Status |
|---------|--------|
| Parallel execution (Block-STM) | Implemented |
| ZK privacy layer | Partial (Shield/Unshield txs exist) |
| Recursive proof compression (Nova) | Wired into block pipeline, light client API; real `--prove` chain proofs verified end-to-end on 3-Mini cluster (2026-05-02) |
| Temporal smart contracts | Implemented (DeferredTx, TemporalGuard) |
| Post-quantum VRF | Implemented (ML-DSA VRF) |
| Data availability sampling | Implemented — `data_root` derived from `build_block_da_inputs(txs)`; identical at proposal-time and serve-time; finality gated on DA supermajority |

### Invention-stack primitives (substrate crates)
| Primitive | Crate | Status |
|---|---|---|
| Light-Cone Ledger DAG | `evaporchain-light-cone` | Live; pruning wired into consensus tick (every 100 blocks, 1000-epoch retention) |
| Bell-Certified Beacon | `evaporchain-bell-beacon` | Live |
| Singh Attractor Consensus | `evaporchain-singh-attractor` | Live |
| Evaporated-Fork Certificates | `evaporchain-evap-fork-cert` | Live |
| Immune Validator Set | `evaporchain-ib-validators` | Live |
| ~~MERA gate~~ | `evaporchain-mera` | **DOES NOT SHIP** — empirical gate FAILED on real Ethereum 2026-05-03 (R²=0.66 across 3 independent runs vs threshold 0.85). Per doctrine §A1.8 contingency, Energy-Verkle Trie ships instead (already in `evaporchain-state`). Crate retained as research artefact only. |

### Marketplace + cultural-launch lanes
SDDC (`evaporchain-sddc`) → SFSV future-self vault, SHLM skill half-life market, Singh-Sabi & Singh-Migrant patina NFTs, MnemoChain (Anki + FSRS on-chain), Gallery That Forgets (Mayfly NFTs), all on the SDDC pattern. SGB (Girard !/?) + SBAV (Bennett reversible VM) + SSM (Hyland-Ong arenas) close the smart-contract paradigm trifecta.

---

## Ecosystem

| Component | Location | Status |
|-----------|----------|--------|
| CLI Wallet | `wallet/` | 90+ modules, 57 behavior tests |
| Browser Extension | `extension/` | Chrome MV3, ML-DSA via WASM |
| Mobile Wallet | `mobile-wallet/` | Tier 2 (onboarding, QR, staking) |
| Wallet SDK | `wallet-sdk/` | v0.1.0 |
| NFT Marketplace dApp | `dapps/nft-marketplace/` | Complete |
| Energy Pool dApp | `dapps/energy-pool/` | Complete |
| Mortal Messages dApp | `dapps/mortal-messages/` | Complete |
| Governance DAO dApp | `dapps/governance/` | Complete |
| Testnet Explorer | `website/` | 6 pages, live decay viz |

---

## Post-Phase Security Hardening (2026-04-24)

Full 12-agent security audit performed. Results:
- **13 CRITICAL** — all fixed (BLS verification, reentrancy guard, gas limits, key security, etc.)
- **23 HIGH** — 23/23 fixed in code (BLS PoP enforcement at validator registration, TWAP single-block manipulation, DA attestation DST, plus 21 prior)
- **30+ MEDIUM** — 17 fixed (snapshot finality depth guard added, DA attestation domain separation), 7 acceptable, 3 design gaps, 1 pending benchmarks

Key hardening commits (2026-04-24 → 2026-04-29):
- Bridge BLS signature verification, ghost bridge BLS verification
- Per-peer network rate limiting (500 msgs/10s)
- Oracle endpoint mandatory auth, contract storage cap (10K keys)
- Timeout jitter with per-height variation, block size validation on receipt
- VRF-seeded leader selection, keygen file permissions (0o600)
- MockProver rejects proofs in release builds
- TWAP single-block manipulation: `twap()` returns None if < 2 time-distinct entries
- BLS PoP enforcement: `add_validator()` rejects unverified BLS keys (rogue-key attack)
- DA attestation domain separation: `"evaporchain:da-attestation:v1:"` DST prefix
- Snapshot finality: `create_finalized()` enforces min depth before serving to sync nodes
- Validator BLS key encrypted at rest (Argon2id + XChaCha20-Poly1305)

Integration test suite: 358 tests across 61 modules. All substrate crates covered.

3-node BFT fault tolerance proven on physical hardware (3 Mac Mini M4s via Tailscale).

*Last updated: 2026-05-03 (substrate crate count, test count, Frontier-Features expansion, wallet-sdk version)*
