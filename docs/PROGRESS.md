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

### Phase 3: Complete Nova Proving System — NOT STARTED
- Wire recursive proof generation into block pipeline
- Proof verification in consensus
- Light client proof support

### Phase 4: Smart Contract Story — NOT STARTED
- Pick ONE VM (WASM vs EvaporScript)
- Deploy/call lifecycle
- Gas metering

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
| Recursive proof compression (Nova) | Scaffolded, not wired |
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

*Last updated: 2026-04-10*
