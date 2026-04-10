# EvaporChain Development Progress

## Overview

Novel L1 blockchain with energy-based state decay. 13 Rust crates, post-quantum signatures (ML-DSA), Tendermint BFT consensus, browser extension, mobile wallet, SDK, 3 dApps.

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

**Test count:** 96 tests in consensus crate (all passing)

**Remaining Phase 2 work:**
- Run stress test on live devnet and tune timeouts
- End-to-end multi-node block finality verification

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
| VM choice | Done | EvaporScript — 91 opcodes, stack-based, deterministic, energy-aware |
| Parser + Compiler | Done (pre-existing) | Source → AST → bytecode, state schema declaration |
| EvaporVM execution | Done (pre-existing) | Gas-metered, bounded loops (100K), max stack 1024 |
| Template contracts | Done (pre-existing) | 7 templates (DecayingToken, MortalNFT, Escrow, Auction, Staking, DAO, Temporal) |
| Deploy/call lifecycle | Done | Full end-to-end: API → mempool → executor → VM → state |
| Script API endpoints | Done | `/api/tx/deploy-script`, `/api/tx/call-script`, `/api/scripts`, `/api/script/:id` |
| Gas metering | Done (pre-existing) | Per-opcode costs, GAS_DEPLOY_SCRIPT=150K, GAS_CALL_SCRIPT=50K |
| Lifecycle hooks | Done (pre-existing) | `on_evaporate()`, `on_grace()`, `on_refresh()` |
| list() for ScriptEngine | Done | Query all deployed scripts via API |

**Test count:** 53 script VM tests, 99 consensus tests

### Phase 5: Stress Testing — NOT STARTED
- Target: 1000+ TPS sustained
- Parallel execution tuning (Block-STM already implemented)
- Network layer optimization

### Phase 6: Audit Prep — NOT STARTED
- Docs: AUDIT_SCOPE.md, CRYPTO_SPEC.md, THREAT_MODEL.md already written
- Formal verification of consensus safety
- Fuzzing campaign

### Phase 7: Mainnet Genesis — NOT STARTED
- Genesis config finalized
- Validator onboarding
- Launch

---

## 6 Frontier Features

| Feature | Status |
|---------|--------|
| Parallel execution (Block-STM) | Implemented |
| ZK privacy layer | Partial (Shield/Unshield txs exist) |
| Recursive proof compression (Nova) | Wired into block pipeline, light client API |
| Temporal smart contracts | Implemented (DeferredTx, TemporalGuard) |
| Post-quantum VRF | Implemented (ML-DSA VRF) |
| Data availability sampling | In progress (erasure coding exists) |

---

## Ecosystem

| Component | Location | Status |
|-----------|----------|--------|
| CLI Wallet | `wallet/` | 90+ modules, 57 behavior tests |
| Browser Extension | `extension/` | Chrome MV3, ML-DSA via WASM |
| Mobile Wallet | `mobile-wallet/` | Tier 2 (onboarding, QR, staking) |
| Wallet SDK | `wallet-sdk/` | v0.2.0 |
| NFT Marketplace dApp | `dapps/nft-marketplace/` | Complete |
| Energy Pool dApp | `dapps/energy-pool/` | Complete |
| Mortal Messages dApp | `dapps/mortal-messages/` | Complete |
| Governance DAO dApp | `dapps/governance/` | Complete |
| Testnet Explorer | `website/` | 6 pages, live decay viz |

---

*Last updated: 2026-04-10 (Phase 3 complete)*
