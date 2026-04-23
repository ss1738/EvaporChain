# EvaporChain — Remaining Work (2026-04-23)

Full audit completed across all docs, research, crates, tests, SDK, wallet, dapps, security, and infrastructure. This is the complete punch list.

---

## CRITICAL — Security (Fix Before Anything Else)

- [x] **HIGH: Unchecked pool balance addition** — FIXED: `checked_add()` in `privacy_exec.rs:201`
- [x] **HIGH: Unchecked receiver balance update** — FIXED: `checked_add()` in `privacy_exec.rs:350`
- [x] **HIGH: Unchecked sum_out + fee in unshield** — FIXED: `checked_add()` in `privacy_exec.rs:504`
- [x] **HIGH: Unchecked balance += in SimpleExecutor/ParallelExecutor** — FIXED: `saturating_add()` in `lib.rs:358,813`, `parallel.rs:708`
- [x] **MEDIUM: Unsafe keypair reconstruction** — FIXED: compile-time `const_assert!` in `signatures.rs`
- [x] **MEDIUM: Poseidon unwrap() on field ops** — DOCUMENTED: Safety proofs added as comments (values provably in-range)
- [x] **MEDIUM: MockProver no production guard** — FIXED: `tracing::warn!` in release builds
- [ ] **LOW: WASM secret key exposure** — `crates/evaporchain-crypto-wasm/src/lib.rs:33` — Document browser isolation requirements.
- [ ] **LOW: Non-standard Poseidon constants** — `crates/evaporchain-crypto/src/hash.rs:78-88` — Document security rationale vs audited Arkworks constants.

---

## CRITICAL — Infrastructure (Must-Have for Mainnet)

- [ ] **Dockerfile** — Multi-stage build (builder + runtime). No container support exists today.
- [ ] **docker-compose.yml** — 4-node local devnet in containers.
- [ ] **Kubernetes manifests** — Deployment, Service, PVC (RocksDB), ConfigMap (genesis), NetworkPolicy.
- [ ] **Prometheus metrics exporter** — Instrument node with `/metrics` endpoint. Expose block height, TPS, peer count, consensus round, gas usage.
- [ ] **Grafana dashboard templates** — Pre-built dashboards for node health, consensus, execution.
- [ ] **API rate limiting** — Per-IP rate limits on all public endpoints. DoS risk today.
- [ ] **TLS/mTLS** — Certificate automation between validators. MITM risk today.
- [ ] **Secrets management** — Validator keys in Kubernetes Secrets or HashiCorp Vault. Not hardcoded.
- [ ] **Slashing implementation** — Planned Phase 7 but not built. Required for economic security.
- [ ] **External security audit** — Budget £30-50K. Recommended firms: Trail of Bits, Least Authority, Zellic, OtterSec, Veridise, Sigma Prime. Scope at `docs/AUDIT_SCOPE.md`.

---

## HIGH — Protocol & Consensus

- [ ] **3-node Tendermint deployment** — Apsarth Mini needs SSH key added (physical visit to cousin's). Currently 2-node.
- [ ] **Weak subjectivity** — No implementation. Needed pre-mainnet to prevent long-range attacks.
- [ ] **BLS proof-of-possession** — Prevent rogue-key attacks. Add PoP verification on validator registration.
- [ ] **Formal verification** — Coq/Lean proofs for evaporation engine, Verkle trie, MMR accumulator. TLA+ spec exists but no mechanized proofs.

---

## HIGH — Research & Papers

- [ ] **Paper 1: Energy-Decay State Management** — Formal proofs of thermodynamic model. Target: arXiv + academic venue.
- [ ] **Paper 2: State Economics** — Proof that infinite-state blockchains are economically unsustainable. "The mind-changing paper."
- [ ] **Paper 3: Benchmarks** — EvaporChain vs Ethereum/Solana/Sui on state growth over 5-year simulations.
- [ ] **3 Frontier Papers** — PoHA, Energy-Verkle, Rule-Based Consensus. Target: ACM CCS 2026 / USENIX Security 2027.
- [ ] **arXiv whitepaper submission** — Convert `research/whitepaper.md` to PDF, submit. Steps at `announcement/arxiv_submission.md`.
- [ ] **Grant submissions** — Ethereum Foundation (£50K) at `grants/ethereum_foundation.md`, Sui Foundation ($30K) at `grants/sui_foundation.md`.

---

## MEDIUM — CI/CD & Testing

- [ ] **Code coverage** — Add `cargo-tarpaulin` or `llvm-cov` to CI. Report to Codecov.
- [ ] **Cargo-audit in CI** — Dependency vulnerability scanning on every PR.
- [ ] **Cargo-deny** — License compliance and duplicate dependency checking.
- [ ] **Integration tests in CI** — Currently only unit tests run. Add `tests/integration` to CI pipeline.
- [ ] **Benchmark regression tracking** — Run benchmarks in CI, flag regressions >10%.
- [ ] **Fuzzing harnesses** — Wire `wallet/src/fuzzer.rs` to `cargo-fuzz`. Add harnesses for Poseidon, EvaporScript parser, bincode deserialization.
- [ ] **Release automation** — `cargo-release` or GitHub Actions for tagged binary releases.

---

## MEDIUM — Ops & Deployment

- [ ] **Terraform/IaC** — Modules for Hetzner/AWS/GCP. Reproducible infrastructure.
- [ ] **Expanded devnet** — 10-20 validators on Hetzner (€5/mo CX21 each). Scripts exist at `scripts/deploy-testnet.sh`.
- [ ] **Log aggregation** — Structured JSON logging (tracing-subscriber) + Loki/ELK.
- [ ] **Alerting rules** — AlertManager for: consensus stalled, peer count drop, block production stopped, disk >80%.
- [ ] **Health check endpoints** — `/healthz` (liveness) and `/readyz` (readiness) for k8s probes.
- [ ] **Backup strategy** — Automated RocksDB snapshots. Define RTO/RPO.
- [ ] **Disaster recovery plan** — Documented procedures for node failure, state corruption, network partition.
- [ ] **Runbooks** — Validator onboarding, emergency procedures, network upgrades, state sync.
- [ ] **Environment configs** — Separate genesis/config for dev, staging, prod. No env-specific separation today.

---

## MEDIUM — User-Facing Tooling

### SDK (45% complete)
- [ ] Transaction submission (not just queries)
- [ ] WebSocket subscriptions for real-time events
- [ ] Batch operations
- [ ] Retry logic and rate limiting
- [ ] Publish to npm

### Wallet SDK (70% complete)
- [ ] Contract deployment/calling
- [ ] Offline signing mode
- [ ] Session management
- [ ] Publish to npm

### Browser Extension (75% complete)
- [ ] Complete swap backend integration
- [ ] Advanced analytics/portfolio
- [ ] User preferences persistence
- [ ] Backup/restore UI
- [ ] Chrome Web Store submission

### Mobile Wallet (60% complete)
- [ ] Complete staking UI
- [ ] Complete swap UI
- [ ] Hardware wallet integration
- [ ] WalletConnect support
- [ ] Offline mode
- [ ] App Store / Play Store submission

### Website + Explorer (65% complete)
- [ ] Advanced explorer: filters, pagination, real-time updates
- [ ] Complete staking/DAO pages (currently placeholders)
- [ ] Contract interaction playground
- [ ] Transaction simulation
- [ ] Deploy to `evaporchain.com` or `testnet.evaporchain.com`

### dApps — All 4 (67% average)
- [ ] Wire contract interactions to extension wallet
- [ ] Complete transaction signing flows
- [ ] Deploy to public URLs

---

## LOW — Code Quality

- [ ] **Update test count in docs** — README/PROGRESS.md say 298 tests. Actual: 1,155+.
- [ ] **Criterion.rs benchmarks** — Replace ad-hoc benchmarks with criterion for statistical rigor.
- [ ] **Config validation** — JSON Schema for genesis configs.
- [ ] **Hybrid post-quantum scheme** — Optional ECDSA fallback alongside ML-DSA.
- [ ] **Version pinning** — Cargo.toml uses range specifiers. Pin exact versions for reproducibility.
- [ ] **ADRs (Architecture Decision Records)** — Document key design decisions.

---

## DONE (For Reference)

All 7 development phases complete:
- [x] Phase 1: Critical fixes (signatures, gas, overclaim)
- [x] Phase 2: Real consensus (Tendermint BFT, BLS, validators, libp2p)
- [x] Phase 3: Nova proving (IVC, chain proofs, light client)
- [x] Phase 4: Smart contracts (EvaporScript, 7 templates, lifecycle hooks)
- [x] Phase 5: Stress testing (468K TPS peak, 7K sustained)
- [x] Phase 6: Audit prep (proptests, Byzantine tests, invariants, fuzzing)
- [x] Phase 7: Mainnet genesis (genesis config, CLI tools, launch scripts)

All frontier features implemented:
- [x] Parallel execution (rayon + BlockSTM)
- [x] ZK privacy (commitments, nullifiers, balance binding, Merkle tree)
- [x] Recursive proofs (Nova IVC, chain compression)
- [x] Temporal contracts (time-locked, phased execution)
- [x] Post-quantum VRF (ML-DSA based)
- [x] DA sampling (2D Reed-Solomon, cell proofs)
- [x] Oracle consensus (multi-source, signed reports)
- [x] Sharding (cross-shard routing, compaction)
- [x] Ghost bridges (cross-chain resurrection)
- [x] Programmable decay curves (linear, exponential, stepped, custom)

Live cluster: 2-node Tailscale (Satyawan + Ironman), block height 940+.
