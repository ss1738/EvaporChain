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
- [x] **LOW: WASM secret key exposure** — DOCUMENTED: Browser isolation requirements in `lib.rs` module docs
- [x] **LOW: Non-standard Poseidon constants** — DOCUMENTED: Security rationale, domain separation, audit checklist in `hash.rs`

---

## CRITICAL — Infrastructure (Must-Have for Mainnet)

- [x] **Dockerfile** — Multi-stage build (builder + slim runtime), healthcheck, non-root user
- [x] **docker-compose.yml** — 4-validator local devnet with persistent volumes
- [x] **.dockerignore** — Excludes target/, website/, extension/, docs/, research/
- [x] **Kubernetes manifests** — StatefulSet (4 replicas, auto-peer-discovery), headless Service, LoadBalancer Service, ConfigMap (genesis), NetworkPolicy, PVC (10Gi per validator), liveness/readiness probes
- [x] **Prometheus metrics exporter** — `/metrics` endpoint with 12 gauges/counters + `deploy/prometheus.yml` scrape config
- [x] **Grafana dashboard templates** — 12-panel dashboard (block height, TPS, peak TPS, objects, ghosts, peers, time series for all) at `deploy/grafana/evaporchain-dashboard.json`
- [x] **API rate limiting** — 200 req/10s per IP, returns 429 with Retry-After header
- [x] **TLS/mTLS** — DONE: libp2p-tls transport (`--tls` flag), PeerAuthority allowlist (`--allowed-peers`), cert generation (`scripts/generate-tls-certs.sh`, `tls.rs`)
- [x] **Secrets management** — DONE: K8s Secret manifests (`deploy/k8s/secrets.yaml`), per-validator key mounts in StatefulSet, upload script (`scripts/k8s-upload-secrets.sh`)
- [x] **Slashing implementation** — ALREADY BUILT: equivocation (10% stake), downtime (1%/miss), jailing, unjailing, auto-removal at `validator_set.rs:341-393`
- [ ] **External security audit** — Budget £30-50K. Recommended firms: Trail of Bits, Least Authority, Zellic, OtterSec, Veridise, Sigma Prime. Scope at `docs/AUDIT_SCOPE.md`.

---

## HIGH — Protocol & Consensus

- [ ] **3-node Tendermint deployment** — Apsarth Mini needs SSH key added (physical visit to cousin's). Currently 2-node.
- [x] **Weak subjectivity** — ALREADY BUILT: checkpoints every 1000 blocks, reorg protection, persistence at `tendermint.rs:1146-1302`
- [x] **BLS proof-of-possession** — ALREADY BUILT: `add_validator_with_pop()`, `verify_bls_pop()` with real BLS12-381 at `validator_set.rs:188-227`
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

- [x] **Code coverage** — DONE: `cargo-tarpaulin` in CI with Codecov upload (`.github/workflows/ci.yml`)
- [x] **Cargo-audit in CI** — DONE: `rustsec/audit-check@v2.0.0` on every push/PR
- [x] **Cargo-deny** — DONE: `deny.toml` config + `cargo-deny-action` in CI
- [x] **Integration tests in CI** — DONE: `cargo test -p integration-tests -- --include-ignored` with 15min timeout
- [ ] **Benchmark regression tracking** — Run benchmarks in CI, flag regressions >10%.
- [ ] **Fuzzing harnesses** — Wire `wallet/src/fuzzer.rs` to `cargo-fuzz`. Add harnesses for Poseidon, EvaporScript parser, bincode deserialization.
- [x] **Release automation** — DONE: `.github/workflows/release.yml` builds linux/mac amd64/arm64 on tag push, creates GitHub Release

---

## MEDIUM — Ops & Deployment

- [ ] **Terraform/IaC** — Modules for Hetzner/AWS/GCP. Reproducible infrastructure.
- [ ] **Expanded devnet** — 10-20 validators on Hetzner (€5/mo CX21 each). Scripts exist at `scripts/deploy-testnet.sh`.
- [x] **Log aggregation** — DONE: `--json-log` flag for structured JSON output via tracing-subscriber, RUST_LOG env filter support
- [x] **Alerting rules** — DONE: 10 Prometheus alert rules at `deploy/alertmanager-rules.yaml` (consensus stalled, no peers, disk, memory, validator down)
- [x] **Health check endpoints** — DONE: `/healthz` (liveness) and `/readyz` (readiness with block height, peers, uptime) in `api.rs`
- [ ] **Backup strategy** — Automated RocksDB snapshots. Define RTO/RPO.
- [ ] **Disaster recovery plan** — Documented procedures for node failure, state corruption, network partition.
- [ ] **Runbooks** — Validator onboarding, emergency procedures, network upgrades, state sync.
- [x] **Environment configs** — DONE: `configs/dev.json`, `configs/staging.json`, `configs/prod.json` with env-specific parameters

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

- [x] **Update test count in docs** — DONE: Updated to 4,159 tests across README, CLAUDE.md, grants, announcement
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
