# EvaporChain — External Security Audit RFP

**Document version:** 2026-04-27
**Project:** EvaporChain (Rust L1 with Tendermint BFT, Nova IVC, ZK privacy, EvaporScript VM, Energy-Verkle Trie)
**Contact:** Satyawan Singh — `satyawansinghinuk@gmail.com` · `ss1738@student.le.ac.uk`
**Repository:** `github.com/ss1738/EvaporChain` (private; access by NDA)

This document is the engagement brief for prospective external auditors. It does **not** include detailed threat modelling or code-level invariants — those live in `audit_readiness_pack_2026_04_27.md`. Recent verified findings live in `cross_verification_2026_04_27.md`. Both will be supplied to the engaged firm under NDA.

---

## 1. Project summary

EvaporChain is a novel Layer 1 blockchain whose differentiator is **energy-based state decay** ("evaporation"): objects decay along configurable curves, with cryptographic decay proofs. Other primitives:

- **Consensus:** Tendermint BFT with stake-weighted 2/3 quorum + BLS12-381 aggregate signatures (production path); MockConsensus for single-node dev (default in current binary).
- **Cryptography:** ML-DSA (Dilithium3) for transaction signatures, BLS12-381 for consensus, BLAKE3 for hashing, Poseidon for ZK circuits, Verkle trees, MMR.
- **Proving:** Nova IVC for incremental proving; ZK evaporation proofs.
- **Smart contracts:** EvaporScript VM with 44 opcodes (`crates/evaporchain-script/src/compiler.rs:11 enum Op`); template contracts + rule engine in `evaporchain-contracts` (template count to be confirmed against `lib.rs` impl blocks — pitch claims 7, prior audit memo said 6).
- **Data availability:** 2D erasure coding library (Reed-Solomon over BLS12-381) with PoHA (Proof of Honest Availability), Namespace Merkle Trees, light-client DA sampling. Library complete, integration into block production is a known gap.
- **Sharding & oracle:** stake-weighted shard assignment, cross-shard message routing, on-chain oracle with TWAP and outlier rejection.
- **Privacy:** ZK note tree with nullifiers, balance-binding commitments, real Poseidon hashing.

**Codebase scale:** ~66K LOC Rust across 16 workspace crates; ~4,486 tests including ~19 byzantine adversarial scenarios, proptest, and fuzzing harnesses.

**Project status:** all 7 internal development phases complete. Internal multi-agent audit completed 2026-04-24, with a follow-up cross-verification on 2026-04-27 that surfaced new findings from the intervening commits. Currently running 3-node testnet across Mac Minis over Tailscale (~9.5 blocks/sec, 224+ tx executed, state roots equal across nodes). Mainnet not yet launched.

## 2. Audit objectives

In priority order:

1. **Consensus safety and liveness** under byzantine validators (up to 1/3 malicious by stake).
2. **Cryptographic correctness** — ML-DSA / BLS12-381 / Poseidon / Nova usage, key lifecycle, RNG sources, side-channel exposure.
3. **Smart contract sandbox** — EvaporScript VM correctness, gas metering, reentrancy, call-depth bounds, access control.
4. **State integrity** — RocksDB rollback, Verkle/MMR proof verification, block-STM determinism (parallel vs serial fallback).
5. **Economic safety** — fee accounting, slashing conditions, reward distribution, integer overflow on stake/balance arithmetic.
6. **Data availability assumptions** — DA certificate verification, sampling correctness, certificate forgery resistance.
7. **Cross-shard messaging** — receipt root deduplication, replay protection across shards.
8. **Network layer** — gossip integrity, peer authentication, DoS resistance, mempool admission.

## 3. Scope

### In-scope crates (all under `crates/`)

| Crate | Approx LOC | Why |
|-------|-----------|-----|
| `evaporchain-consensus` | 13,900 | Tendermint, validator sets, finality, DA attestation |
| `evaporchain-execution` | 10,500 | STM, parallel exec, fees, rewards, privacy enforcement |
| `evaporchain-node` | 9,500 | API, persistence, key load path, oracle/shard bridges |
| `evaporchain-proving` | 5,600 | Nova IVC, ZK evaporation proofs |
| `evaporchain-crypto` | 4,746 | ML-DSA, BLS, BLAKE3, Verkle, MMR, EnergyVerkleTrie |
| `evaporchain-script` | 4,452 | EvaporScript VM, opcodes, gas metering |
| `evaporchain-da` | 3,316 | 2D erasure coding, sampling, certs, PoHA, NMT |
| `evaporchain-state` | 3,400 | RocksDB backend, evaporation, ghost bridge |
| `evaporchain-contracts` | 2,897 | Template contracts, rule engine, access control |
| `evaporchain-types` | 1,600 | Tx variants, block fields, decay curves |
| `evaporchain-oracle` | 1,400 | Vote consensus, TWAP, outlier rejection |
| `evaporchain-network` | 1,017 | P2P, mempool, TLS validator keys |
| `evaporchain-sharding` | 700 | Assignment, cross-shard routing |

### Out-of-scope (not audited unless explicitly added)

- `evaporchain-cli` — operator UX
- `evaporchain-mcp` — stub (771 LOC, 0 tests)
- `evaporchain-crypto-wasm` — WASM bindings (in scope only if browser exposure of keys is in threat model)
- `mobile-wallet/`, `dapps/`, `extension/`, `wallet/`, `wallet-sdk/` — application layer
- `sdk/` (TypeScript) — only the protocol-level invariants are in scope, not SDK ergonomics
- Frontend explorer
- Genesis / tokenomics economic-modelling assumptions (auditors are welcome to flag, but parameter calibration is the project's responsibility)

### Already-known issues (please review for completeness, not novelty)

Detailed in `audit_readiness_pack_2026_04_27.md`. Headline:
- `pqc_dilithium` Rust crate is itself unaudited upstream (H-13 in prior audit).
- Poseidon constants are non-standard pending RFC alignment (H-15).
- MockConsensus is the binary default; Tendermint is `--tendermint-mode` opt-in.
- Validator BLS key file is plaintext at rest (mode 0600 only) — not yet encrypted.
- Six findings from internal cross-verification (2026-04-27) are tracked in `cross_verification_2026_04_27.md`. Most will be resolved before audit kickoff.

## 4. Engagement deliverables expected

1. **Pre-audit threat model review** — confirm or amend the model in `audit_readiness_pack_2026_04_27.md`.
2. **Manual code review** of all in-scope crates with severity-graded findings (CRITICAL / HIGH / MEDIUM / LOW / INFO).
3. **Adversarial scenario testing** — at minimum: nothing-at-stake, long-range, equivocation, censorship, eclipse, mempool DoS, slashable double-vote replay, finality reversion.
4. **Cryptographic primitive review** — RNG sources, key lifecycle, signature scheme implementations, Poseidon parameter audit.
5. **Optional: formal verification** of selected EvaporScript opcodes and consensus monotonicity invariants (priced separately).
6. **Optional: DA layer dedicated review** — 2D erasure correctness, light-client sampling soundness, certificate forgery resistance (priced separately).
7. **Re-audit pass** after fix iteration (one round included, additional rounds on T&M).
8. **Final public report** with executive summary suitable for marketing, redacted only for active critical issues.

## 5. Auditor shortlist

Filtered for: Rust + L1 / Cosmos-SDK / Substrate experience; novel-protocol depth; willing to engage UK/sole-trader entities. **Pricing ranges and lead times below are typical industry signals from public engagements; treat them as planning estimates, not quotes — final pricing depends on scope, calendar, and firm utilization.**

### Tier 1 — first-choice for this project

**Trail of Bits** (US, NYC)
- Rust depth: deep. Substantial Cosmos-SDK and Rust L1 work. Slither / Echidna / Manticore lineage; strong on fuzzing infrastructure.
- Indicative engagement: USD $200K-$400K range for a 4-6 week core-protocol review on ~60K LOC. Lead time 8-16 weeks from contract signature.
- Why pick: most rigorous on cryptographic engineering and unsafe-Rust review.

**Sigma Prime** (Sydney, AU)
- Rust depth: native. Authors of Lighthouse (Ethereum consensus client). Substantial novel-consensus audit experience.
- Indicative engagement: USD $150K-$300K for a 4-6 week consensus-focused engagement. Lead times historically 6-12 weeks.
- Why pick: best in class for Tendermint / BFT consensus correctness; strong on slashing condition review.

**Zellic** (US)
- Rust depth: strong. Several recent novel-L1 audits (Aptos, Sui, Movement) and a clear preference for engineering-driven rather than checkbox audits.
- Indicative engagement: USD $150K-$300K for ~4 week core review. Lead times typically 4-10 weeks.
- Why pick: fast turnaround, strong adversarial-thinking culture, good fit for novel primitives like evaporation/energy decay.

### Tier 2 — credible alternatives

**ChainSecurity** (Zurich, CH)
- Strong on EVM but increasingly on Substrate / Polkadot. Formal-methods inclined.
- Useful if a Switzerland/EU base is preferred for paperwork.

**Halborn** (Miami, US)
- Larger team, broader coverage (offensive + crypto + smart contract). Growing Rust practice.
- Useful if combined penetration testing is desired.

**Quantstamp** (Toronto, CA)
- Broad coverage, established methodology. Generally Solidity-leaning but has handled Rust.
- Useful as a price-competitive option for in-scope expansions.

### Tier 3 — specialised supplements (optional, alongside one Tier 1)

**Runtime Verification** (US/Greece)
- Formal verification specialists (K framework). Best-in-class for proving VM opcode semantics.
- Indicative engagement: USD $100K-$250K for a focused VM / consensus proof engagement.
- Why pick: complement to a Tier 1 firm; defense-in-depth for EvaporScript opcodes and finality monotonicity invariants.

**NCC Group — Cryptography Services** (US/UK)
- Pure cryptographic primitive review (key lifecycle, side-channel, RNG, BLS / Poseidon parameter check).
- UK office is a logistical advantage.

### Considered and excluded

- **OpenZeppelin** — top-tier but Solidity-centric; Rust engagements are less frequent and the firm's strongest reviewers are typically allocated to EVM work.
- **CertiK** — checkbox-heavy, less aligned with novel-primitive depth needs.

## 6. What auditors will ask before quoting

Anticipate and prepare answers for these. Most are addressable from `audit_readiness_pack_2026_04_27.md`.

1. **LOC by language and crate** — provided.
2. **Test count, coverage report, fuzz corpora locations** — provided; coverage report not yet generated, plan to run before kickoff.
3. **Threat model document** — provided in audit-readiness pack.
4. **Trust assumptions** — who is honest, what attackers can do.
5. **Architecture diagrams** — to be added (see action list, §8).
6. **Existing audit reports** — `FULL_AUDIT_2026_04_24.md` (internal multi-agent), `cross_verification_2026_04_27.md`. Both supplied under NDA.
7. **Known-issue list** — in audit-readiness pack.
8. **Build / test reproducibility** — `cargo build --workspace`, `cargo test --workspace`. Documented in CLAUDE.md.
9. **Critical dependencies** — `pqc_dilithium`, `blstrs`, `arkworks`, `nova-snark`, `rocksdb`, `libp2p`. Versions in `Cargo.lock`.
10. **Network / consensus parameters** — block time, gas limit, block reward, slashing %, unbonding period. To be tabulated in audit-readiness pack v2.
11. **Mainnet timeline** — drives audit slot urgency.
12. **Engagement model** — fixed price vs T&M; whether re-audit is in-scope.

## 7. Timeline expectations

| Stage | Duration |
|-------|----------|
| RFP issued to 3-5 firms | week 0 |
| First-round NDA + scoping calls | week 1-3 |
| Proposals received | week 3-5 |
| Selection + contract negotiation | week 5-8 |
| Audit kickoff | week 8-12 |
| Active audit | 4-6 weeks (Tier 1 firms) |
| Findings delivered + remediation | 2-4 weeks |
| Re-audit pass | 1-2 weeks |
| Final report | week 18-26 from RFP issue |

Plan for **roughly 5-6 months** from sending this RFP to a clean final report. Compress only by paying premium-slot fees, which firms occasionally offer.

## 8. Open work before issuing the RFP

In approximate priority order:

1. **Resolve the cross-verification findings** in `cross_verification_2026_04_27.md`. Auditors will down-weight the engagement if these are still open at kickoff.
2. **Wire `BlockDA2D::encode_block()` into `MockConsensus::produce_block()`** so DA layer is actually exercised (currently sentinel for empty blocks).
3. **Generate code-coverage report** (`cargo tarpaulin` or `cargo-llvm-cov`) on the Minis, not on MacBook.
4. **Produce architecture diagrams** — at minimum: tx lifecycle, consensus state machine, DA flow, key lifecycle. Auditors universally ask.
5. **Tabulate consensus / economic parameters** — block time, target finality latency, max gas, slashing percentages, unbonding period, supply schedule, fee burn ratio. Add to audit-readiness pack v2.
6. **Pin `pqc_dilithium` to a specific commit** and note upstream-audit status to the auditor (they will ask).
7. **Decide budget envelope** — informs which tier of firms to invite.
8. **NDA template** — UK-favourable mutual NDA, ready to send before scoping calls.

## 9. Recommended next action

Issue the RFP simultaneously to **Trail of Bits, Sigma Prime, and Zellic**. Optionally add NCC Group Cryptography Services for a parallel primitive-review quote. Set proposal deadline at 4 weeks from issue. Choose on the basis of (a) reviewer CV match to Rust + Tendermint + Nova, (b) timeline fit, (c) re-audit terms, not just headline price.

Do not engage a single firm without competitive proposals — slot scarcity and pricing variance among the top three is meaningful.
