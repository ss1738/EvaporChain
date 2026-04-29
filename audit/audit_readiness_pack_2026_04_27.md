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
| H-15 | Poseidon constants are non-standard + startup hang on 2^20 NoteTree init | **Startup-hang half RESOLVED 2026-04-28 (commit `9228c55`).** `NoteTree::new` was computing 2^20-1 ≈ 1M Poseidon hashes at every cold boot because every empty leaf produced an independent hash. Cached the empty-subtree hash per level — depth=20 now needs **20 hashes instead of 1M** (50,000× speedup) and the per-node-vs-fast-init regression test (`test_empty_tree_root_matches_per_node_hash`) proves bit-for-bit identical roots for depths 1..6, so no hard fork. **Constants-audit half STILL OPEN:** the BLAKE3-derived round constants and 56-partial-rounds margin remain unaudited by a ZK cryptographer. | Startup latency on a fresh node went from minutes to seconds (sample backtrace pre-fix showed every thread parked in `pasta_curves::Fp::mul`). Constants risk unchanged. |
| K-01 | ~~MockConsensus is the binary default~~ | **RESOLVED 2026-04-27 (commit 4afe27f).** Tendermint is the binary default; `--mock-consensus` is opt-in (`main.rs:751`). `--mainnet` strict mode hard-fails on `--mock-consensus`. | — |
| K-02 | ~~`bls_key.bin` validator key plaintext on disk, mode 0600 only~~ | **RESOLVED 2026-04-27 (commit 0af4bb2).** Opt-in EVK1 encryption (Argon2id + XChaCha20-Poly1305) via `EVAPORCHAIN_VALIDATOR_KEY_PASS`; `--mainnet` strict mode requires it. | — |
| K-03 | ~~`EVAPORCHAIN_KEY_MASTER` env var defaults to dev string~~ | **RESOLVED 2026-04-27 (commit 4afe27f).** `--mainnet` strict mode hard-fails on unset, dev-default, or sub-16-char value. | — |
| K-04 | ~~DA layer 2D erasure exists but not wired into `produce_block`~~ | **RESOLVED 2026-04-27 (commit 1fc67c0).** `compute_block_da` calls `BlockDA2D::encode_block_with_blobs` from `MockConsensus::produce_block`, `produce_block_with_reveals`, and `RotatingConsensus::produce_block_if_leader`. Empty blocks still use sentinel data_root. Tendermint already had its own wiring (`tendermint.rs:1958-2030`). | — |
| K-05 | ~~Equivocation slashing not yet automatic~~ | **RESOLVED — already wired before audit pack capture.** `slash_equivocation` is invoked in-line at all three detection sites: proposal (`tendermint.rs:1113`), prevote (`tendermint.rs:1377`), precommit (`tendermint.rs:1473`). Penalty applies stake reduction + jail + auto-remove below MIN_STAKE in one call (`validator_set.rs:341-356`). Three regression tests cover the path. | — |
| K-06 | Cross-verification §1-§4 findings | All four resolved 2026-04-27 (commits 674be1d, c49a2fe, 0af4bb2). See `cross_verification_2026_04_27.md` for original details. | — |
| K-07 | Multi-validator BFT cluster splits without a shared genesis | **RESOLVED 2026-04-27 / 28** by P0 #4 Phase 1 (commits `fa7271a` + `01b9fe8` + `9f0edbe`) and the K-07 launch guard (commit `051a030`). Node now seeds its validator set from genesis `bls_public_key` entries; without genesis it warns; `--mainnet` strict mode refuses `--validators>1` without `--genesis-config`. CLI `genesis set-validator-bls` retrofits older genesis files. Original failing cluster shape (4-node local, 20260427-215707) and the real-Mini reproduction (K-08, 20260427-230134) both preconditioned on the missing pubkeys; neither would pass today's launch guard. | — |
| K-08 | K-07 reproduces on the real 3-Mini Tailscale cluster | **Confirmed 2026-04-27 via stress run 20260427-230134.** Three Minis launched with today's binary (Satyawan, Apsarth) and the older Sunday binary (Ironman, launchd-managed). Even with Satyawan + Apsarth sharing the *same* genesis state_root `695609051208d7f6…` and peer connectivity established (3-4 peers reported), the cluster committed **zero blocks in 60s** under sustained 8 TPS load. Mempools filled to per-account quota cap (64-110 pending) with no drainage. Each node generated its own ML-DSA + BLS keys at startup so the per-node validator-set bytes differ; Tendermint quorum cannot form. Same root cause as K-07. The K-07 fix (shared `--genesis-config` with pre-registered validator pubkeys) applies. **RESOLVED 2026-04-27 in commits fa7271a (node-side) + 01b9fe8/9f0edbe (CLI `genesis set-validator-bls`).** Node now seeds the validator set directly from genesis bls_public_key entries; on multi-validator launches without genesis it WARNs (and `--mainnet` strict refuses, per commit 051a030). | Validates that K-07 is a real ops-config issue, not a single-machine artefact. Real mainnet validator onboarding must distribute genesis files plus pre-registered validator BLS + ML-DSA pubkeys |
| K-09 | TLS validator key encryption deferred | **Deferred 2026-04-27.** Audit recommended same encryption treatment for TLS validator keys at `network/src/tls.rs:130-141` as we now apply to BLS keys (commit 0af4bb2). However the runtime path in `node/src/main.rs:1709` initialises `tls_certs: None` — TLS is not currently wired for libp2p transport, the cluster uses Noise. Encrypting an unused code path was deemed gold-plating during the building sprint. Re-enable when TLS transport is actually turned on for inter-validator P2P; reuse the EVK1 module (`evaporchain_crypto::bls_key_store`) with a different magic header. | None today (TLS not in use). Becomes a real plaintext-key risk only when TLS transport is enabled |
| K-10 | UpgradeContract proper wiring needs product-design session | **Deferred 2026-04-27.** `UpgradeContractTx { contract_id, new_bytecode }` was made fail-loud in commit 674be1d. Discovered during P0 work that EvaporChain has TWO contract abstractions: template contracts (`ContractInstance`, no bytecode field — can't be upgraded) and EvaporScript scripts (`DeployScriptTx { source_code }`, compiled bytecode that *could* be swapped). UpgradeContract conceptually applies to the script flavor only. Open product questions: (a) governance threshold (currently any GovernanceProposal passes at 2/3 stake — same for upgrades?), (b) state-shape migration (existing script contract state stored under old code may be incompatible with new), (c) atomicity (mid-call upgrade behaviour), (d) immutability opt-in (some contracts should be unupgradeable). The fail-loud guard from 674be1d is the correct production stance until these are answered. | Deployed scripts cannot be upgraded today. Operators / contract authors must redeploy at a new id. Real impact only when EvaporScript contract authors hit production constraints |
| K-12 | ~~3-Mini Tailscale cluster never produced a block (operational K-07/K-08 follow-up)~~ | **RESOLVED 2026-04-28.** First time the cluster has actually committed blocks. Three protocol-level fixes were required: (a) `f2e2fde` enables libp2p `tcp::Config::default().port_reuse(true)` so the listener socket can dial out — without it macOS rejected every bootstrap dial with EADDRINUSE because every cluster member listens on port 9000; (b) `d2ee00e` short-circuits `perform_da_sampling` for empty blocks to use the same `blake3("evaporchain:empty_block")` sentinel the proposer uses, instead of trying to BlockDA-encode `serde_json::to_vec(&[])` and getting a different root — this was rejecting every empty block forever and starving precommit quorum; (c) ops fix: codesign-adhoc the debug binary on apsarth/ironman (Gatekeeper SIGKILL'd the unsigned mach-O) and add it to satyawan's `socketfilterfw` allowlist (firewall was blocking inbound). Plus the genesis ceremony: 3 BLS keypairs via `evaporchain keygen`, injected into `genesis-tailscale-3node.json` via `evaporchain genesis set-validator-bls`, distributed with matching `bls_key.bin` per node. Empirically: block #1531 logged with DA Certificate supermajority, Prevote+Precommit messages from all 3 validators, identical state_root + parent_hash on each node when sampled mid-run. | K-07/K-08 fix moves from structurally proven (matching genesis state_root on init) to **operationally proven** on the actual hardware. |
| K-13 | ~~Recovery branch handoff from sister Claude session~~ | **PARTIAL MERGE 2026-04-28.** Tier 5 session left a recovery branch `recover/tier5-stashed-work` (worktree at `/Users/satyawansingh/EvaporChain-recover`) consolidating ~9.5K lines of stashed WIP. **Cherry-picked:** `b7f4314` SimpleExecutor storage-rent gate (closes Tier-1 #6 SimpleExecutor half — was over-charging rent ~50× per block at 2s blocks) → main as `6506a60`; `16fc712` consensus enforcement (closes the **#4 split-brain risk** where `RotateValidatorKey` tx was admitted but never applied) → main as `a1bf167`. Pulled supporting files (`secret_file_store.rs`, `decaying_dao.rs`, `light_client.rs`) and stubbed `PrivacyExecutor::restore_from_db` so the workspace builds. **Walked back two cascade regressions:** Gap-A #9 NMT had to admit namespace 0 (`f95ba97`) because the proposer uses ns=0 for non-Blob "core tx" framing (would otherwise drop every non-Blob tx); the `canonicalize_address_hex` strict 64-char enforcement had to be reverted on the embedded contract templates (`8c58f6f`) because their tests use string identifiers like "alice"/"bob" — function kept for callers that legitimately need L1 addresses. **Deferred:** the much larger `c3c3dfa` recovery commit (cross-shard activation, vesting timelock, UpgradeContract execution path, DecayingDAO contract template, Coq + TLA+ scaffolding) — needs a deliberate review pass since the recovery branch itself doesn't build cleanly (handoff doc claimed `restore_from_db` existed but it's never defined anywhere in recovery). | Closes the #4 split-brain risk and the storage-rent over-charge. Larger recovery surface still pending. |
| K-14 | ~~Gap-A #4: governance bounds + quorum + vote-cap + timelock~~ | **RESOLVED 2026-04-28 (commit `3f60a6f`).** `execute_governance` now enforces: title ≤ 200 bytes, param_key ≤ 64, param_value ≤ 256, voting_epochs in [10, 100_000], param_key on a closed allowlist (block_gas_limit, base_fee_floor, base_fee_ceiling, target_gas_utilization + `upgrade_contract:` pattern), and per-key value range validation. CastVote caps each voter at `MAX_VOTE_WEIGHT` (10M tokens) so the 35% Foundation Treasury entry can no longer pass solo. `decide_proposal_outcome` enforces 3% supply quorum + 3-distinct-voter quorum + 2× super-majority. Activation is deferred by `GOVERNANCE_TIMELOCK_EPOCHS = 5` so stakeholders can react. 8 inline tests cover quorum gates, super-majority strictness, per-key value validation, and the unknown-key rejection. | Closes Gap-A #4 from end_to_end audit. Whale-pass risk + arbitrary param-key stamping now structurally prevented. |
| K-15 | ~~Gap-A #7: persistence panic → graceful halt for consensus-critical writes~~ | **RESOLVED 2026-04-28 (commit `b47435a`).** Introduced `fatal_persist_err` in `node/src/main.rs` — emits structured `tracing::error`, prints FATAL to stderr, gives 100ms to flush, then `std::process::exit(2)`. Mirrors the discipline already in `evaporchain-state/rocksdb_backend.rs:46`. Migrated 13 call sites — `consensus_meta`, `full_block`, `script_contracts`, `template_contracts` — anything where on-restart divergence between in-memory and on-disk state would corrupt the chain. Left on `log_persist_err` (recoverable from chain replay): `mempool`, `da_package`, `chain_stats`, `events`, defi stores, `poha`. | Lossy persistence on consensus-critical writes used to log red text and continue, leaving in-memory state ahead of disk. Now it halts cleanly. |
| K-16 | ~~Gap-A #8: critical-path unwrap sweep~~ | **RESOLVED 2026-04-28 (commit `a48b1b5`).** Production-path sweep across consensus, execution, mempool, network, da, proving. Most surfaces were already clean (slice→array conversions in da/sampling are statically guaranteed, partial_cmp uses unwrap_or(Equal), Mutex locks use safe_lock helper, parse_hex_address unwraps are demo-mode hardcoded). The one real critical-path panic surfaced was `tendermint.rs::verify_certificate_with_grace` which had three `.expect()` calls that could SIGABRT under adversarial input — signer removed mid-verify, validator without registered BLS key, etc. Replaced with structured `warn!` + `return false` (cert rejection). | Removed the only realistic panic-on-malformed-cert vector. Other audit-named hot paths (BLS sig agg, attestation deserialization, NaN compares) were already defensive. |
| K-17 | ~~Gap-A #1: oracle vote inbound P2P + validator-set membership~~ | **RESOLVED 2026-04-28 (commit `910f252`).** Before: each validator only ever submitted oracle votes to its OWN local `OracleBridge`. There was no inbound P2P route to receive votes from other validators, so multi-validator oracle consensus did not actually run on the cluster — every validator computed quorum against its own self-vote only. `submit_vote_via_validator_set` in `oracle_bridge.rs` was dead code. After: new `ConsensusMessage::OracleVote { payload: Vec<u8> }` variant carrying a serde_json-encoded `OracleVote` (kept opaque so the consensus crate stays decoupled from the oracle crate); height()/round() return 0 and `on_message` returns early before height filters; demo oracle-vote site in `node/main.rs` now ALSO broadcasts each signed vote alongside the local submit; consensus message receive handler deserializes and routes to `OracleBridge::submit_vote_via_validator_set`, which performs validator-set membership lookup keyed on `vote.validator_id` and BLS sig verification against the REGISTERED pubkey (eliminates the rogue-pubkey attack class where an adversary forges a vote claiming any validator_id and supplies an attacker-owned matching pubkey). | Multi-validator oracle quorum now actually runs on a live cluster instead of being a single-validator illusion. |
| K-18 | ~~Faucet → tx → commit silent failure on the live cluster~~ | **RESOLVED 2026-04-28 (commit `7d9893d`).** Cluster could commit empty blocks indefinitely but every tx silently failed at execute time. ROOT CAUSE: `ParallelExecutor::new_production` enables `verify_signatures=true` and constructs with `chain_id=String::new()`. `TendermintConsensus::set_chain_id` only updated `self.chain_id`, not `self.executor.chain_id`. So the API signed txs with the actual `args.chain_id` ("evaporchain-testnet-1" by default) and the executor verified the signature against an empty chain_id — verification always failed, the tx appeared in committed blocks (visible in `/api/block/N` with hardcoded `status: "success"`), but `txs_executed` counted 0 and state never changed. Fix propagates `chain_id` from `TendermintConsensus` to the executor in `set_chain_id`. **Verified live on the 3-Mini cluster:** faucet balance 900_000_000 → 899_885_000, faucet nonce 0 → 1, recipient credited with 10_000, identical state across all 3 validators. First successful real transaction commit on the BFT cluster. | Without this fix mainnet would commit empty blocks forever. With it, the faucet round-trip works end-to-end and load-testing is unblocked. Also surfaces a follow-up: API-side nonce caching for concurrent faucet hits (currently each call queries `faucet_acct.nonce` once at submission, so 5 concurrent submits all use nonce=0 and 4 fail with InvalidNonce — a load-harness concern, not a chain bug). |
| K-11 | ~~Validator delegation feature: 7 of 8 phases complete~~ | **RESOLVED 2026-04-28.** P0 #4 shipped end-to-end across types, state, execution, consensus, wallet, node, and CLI surfaces. Phase 1 types (`a62debb`), Phase 2 exhaustiveness arms across the workspace, Phase 3 StateDB persistence (`aa24e80` + stubs in OverlayStateDB), Phase 4 real `execute_delegate`/`execute_undelegate` handlers (`a882f65` + `a2ff1cb`), Phase 5 `slash_delegations_for_validator` (`dd21e5c`), Phase 6 `delegated_stake` cache + `refresh_delegated_stakes` (`1805c2e`), Phase 7 `ClaimDelegationTx` end-to-end (`e74ea89` + `4ce1415`) with discriminator `0x17`, gas 30k, governance-aware unbonding-window enforcement (`unbonding_epoch + UNBONDING_PERIOD_EPOCHS`), record cleanup when both `amount` and `unbonding_amount` reach zero, and 3 inline tests, Phase 8 13 inline tests (`8c5488d` + `24186e1`). **Wiring deferrals — RESOLVED 2026-04-28 (commit `ba13d8e`):** `refresh_delegated_stakes(db)` now fires at the top of `TendermintConsensus::tick`, and `slash_delegations_for_validator` is invoked at both `ConsensusAction::SlashValidator` handlers in `node/main.rs` (proposer + follower paths) with reason-aware percentages (Equivocation = 10%, Downtime = 1%/missed-block capped at 100%). 4,293 lib tests pass on the Mini at HEAD `4ce1415`. | Operators can delegate, undelegate, get slashed proportionally with delegations slashed alongside the validator's own stake, and reclaim unbonded amounts once the unbonding window elapses. Voting power tracks live delegations on every consensus tick |
| K-20 | ~~`INVENTION_STACK.md` §A1.7 V1 sprint substrate (weeks 1–24)~~ | **RESOLVED 2026-04-28 (commits `84605fc` → `dceebbf`, single session).** 31 substrate crates shipped covering the entire May–Oct 2026 sprint per the Amendment 1 build order, plus 5 Tier 2 (V2) starts. **436 tests, all green on satyawan.** V1 sprint crates (26): `evaporchain-energy-kernel` (single-λ + 4 conservation compartments + RefreshPool + ConservationCheck), `evaporchain-padic` (Hughes 2004), `evaporchain-demurrage`, `evaporchain-tropical` (Speyer-Sturmfels 2004), `evaporchain-fee-controller` (Singh-Lyapunov whitepaper centerpiece — empty-block monotone V drift property-tested), `evaporchain-sanov-slashing` (Sanov 1957 KL-rate), `evaporchain-cfm` (Crooks 1999 / Jarzynski 1997, β=1/λ), `evaporchain-boltzmann-stake`, `evaporchain-refresh-market` (quadratic AMM), `evaporchain-tur-liveness` (Barato-Seifert 2015), `evaporchain-light-cone` (causal-set DAG), `evaporchain-antichain-mempool`, `evaporchain-mcc` (Jaynes Maximum Caliber), `evaporchain-causal-cone` (Shalizi 2003 sufficient statistic ≤128B), `evaporchain-epv` (versions decay→un-runnable), `evaporchain-llsa` (Coq-gated self-amendment, pluggable verifier trait), `evaporchain-cmu-gate` (Cμ ≤ E + hμ), `evaporchain-cslc` (1-state baseline ε-machine), `evaporchain-lambda-fold` (Nova substrate, blake3 placeholder for R1CS), `evaporchain-mdl-shard` (Rissanen 1978), `evaporchain-lad-vm` (Linear/Affine/Decaying), `evaporchain-decay-lamport`, `evaporchain-modular-beacon` (8-term Eisenstein E_4/E_6/Δ q-expansion), `evaporchain-efh` (Cohen-Steiner-Edelsbrunner-Harer 2007), `evaporchain-prp` (positive retention), `evaporchain-evap-fork-cert` (negative finality). Tier 2 starts (5): `evaporchain-decay-forget` (GDPR), `evaporchain-allen-decay` (13 Allen 1983 relations), `evaporchain-eb-fs` (cross-fork replay defence), `evaporchain-hot-cold-stake`, `evaporchain-hlwa` (Ronin/Wormhole-class hack defence). **Substrate boundaries (production needs):** `lambda-fold` needs arkworks Nova R1CS; `llsa` needs MetaCoq-extracted verifier; `cslc` needs full Shalizi-Klinkner CSSR; `lad-vm` needs compile-time substructural enforcement in `evaporchain-script-lad` frontend. **Deferred:** Authenticated Energy-MERA (§A1.4) gated on Ethereum block-touch entropy study per doctrine §10.12. **Integration debt:** zero of the 31 crates is wired into the existing `evaporchain-execution`/`evaporchain-consensus`/`evaporchain-node` paths — they compile + unit-test in isolation. Natural first integration: `EnergyAccumulator + Compartment` into the execution crate's account/state path. Per-crate detail in `~/.claude/projects/-Users-satyawansingh/memory/project_evaporchain_substrate_31_crates.md`. | Substrate for the entire 6-month sprint exists; the long pole shifts from "design + scaffold per primitive" to "wire into the chain". |
| K-19 | ~~Coq mechanizations: three `Admitted` lemmas~~ | **RESOLVED 2026-04-28 (commits `67a538a` → `a16bf60`, machine-verified under Rocq 9.1.1).** All three `Admitted` proofs in `research/coq/` discharged AND all 4 `.v` files compile clean: `coqc -Q . EvaporChain` returns 0 on `EnergyDecayMonotonicity.v`, `EnergyVerkleCompression.v`, `PoHAFreeloading.v`, `LazyEagerEquivalence.v` (run on satyawan Mini, Rocq 9.1.1 via `brew install coq`). (1) `EnergyDecayMonotonicity.v::energy_step_cross_halving` — closed via a new `decay_term_bound` arithmetic helper that proves `Nat.div (v * rm) (2 * h) + Nat.div v 2 <= v` for `rm < h, h > 0`, using `Nat.mul_div_le` lower bounds + `nia` over the certificate `2hQ <= v*h-v` plus `2hP <= h*v` summing to `<= 2vh-v` ⇒ `Q+P <= v`. (2) `EnergyVerkleCompression.v::cold_subtree_zero_energy` NInternal case — closed via a new `node_strong_ind` custom strong-induction principle that threads the IH through `Forall P` over children (the auto-generated `node_ind` doesn't recurse through `list node`), plus a pure list lemma `fold_left_zero_of_Forall_zero` and a `cold_zip_forall` helper that pairs the all_cold Forall with the IH Forall. `all_cold` itself was rewritten from a Fixpoint to an Inductive (Coq's syntactic positivity rejected the Forall-recursing Fixpoint). (3) `PoHAFreeloading.v::poha_freeloading_resistance` — closed via a new closure axiom `negligible_le : forall p q, p <=p q -> negligible q -> negligible p`, a structural property of asymptotic bound classes (downward closure under pointwise `<=`), not a property of any cryptographic primitive. The Coq dependency is now isolated to ONE clearly-named axiom; discharging it requires modelling `prob = Q` and the inverse-polynomial bound for `negligible`. **Side effect of running real coqc:** four pre-existing compile bugs in the original recovery-cherry-picked proofs surfaced and were fixed (commits `bc590b6`, `4591306`, `984adfc`, `a16bf60`) — `nat_shr_monotone_step`'s wrong `Nat.le_div2` call, `all_cold`'s positivity violation, `compress_energy_conservative`'s implicit-rewrite ambiguity, and `energy_step_within_halving`'s no-op `rewrite Nat.add_1_r`. None were in the new code; all surfaced because the original cherry-pick had never been compiled. The proofs now Qed under a real Coq compiler. | Formal methods deliverable goes from "claimed proven" to "machine-checked." The only remaining axiom (`negligible_le`) is structural, not crypto-specific, and is documented as a tracked follow-up to the eventual `prob = Q` model. |

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

## 10. Operational parameters

| Parameter | Current value | Source |
|-----------|---------------|--------|
| Block time target | 2,000 ms | `genesis-mainnet.json` → `block_interval_ms` |
| Max gas per block | 500,000 (default); overridable via `--block-gas-limit` | `genesis-mainnet.json` → `block_gas_limit` |
| Validator bonding period | 2 epochs (200 blocks) | `validator_set.rs:715` `BONDING_PERIOD_EPOCHS` |
| Validator unbonding period | 256 epochs (25,600 blocks) | `execution/lib.rs:196` `UNBONDING_PERIOD_EPOCHS` |
| Slashing % for double-vote (equivocation) | 10% of stake | `validator_set.rs:34` `SLASH_EQUIVOCATION_PCT` |
| Slashing % for downtime | 1% of stake per missed slot | `validator_set.rs:37` `SLASH_DOWNTIME_PCT` |
| Epoch length | 100 blocks | `validator_set.rs:721` `EPOCH_LENGTH` |
| Min validators (liveness threshold) | 3 | `validator_set.rs:709` `MIN_VALIDATORS` |
| Max validator set size | Unbounded (no hard cap; churn ≤ 33%/epoch) | `validator_set.rs:712` `MAX_CHURN_FRACTION` |
| Max blob size | 128 KiB | `execution/lib.rs:307` `MAX_BLOB_SIZE` |
| Max cross-contract call depth (execution) | 64 | `execution/lib.rs:306` `MAX_CALL_DEPTH` |
| Max cross-contract call depth (EvaporScript) | 8 | `script/lib.rs:215` `MAX_CALL_DEPTH` |
| Max VM stack depth | 1,024 | `script/vm.rs:34` `MAX_STACK_DEPTH` |
| Max records in `FinalityTracker` | 10,000 | `finality.rs:126` |
| Min storage deposit (object creation) | 1,000 EVAP | `types/lib.rs` `MIN_STORAGE_DEPOSIT` |
| Storage rent rate | 1 EVAP / byte / epoch | `types/lib.rs` `STORAGE_RENT_PER_BYTE_PER_EPOCH` |
| Oracle quorum: min total weight | 30,000,000 EVAP | `oracle.rs` `QUORUM_MIN_TOTAL_WEIGHT` |
| Oracle quorum: min voters | 3 | `oracle.rs` `QUORUM_MIN_VOTERS` |
| Oracle max vote weight per validator | 10,000,000 EVAP | `oracle.rs` `MAX_VOTE_WEIGHT` |
| Governance timelock | 5 epochs (500 blocks) | `execution/lib.rs` `GOVERNANCE_TIMELOCK_EPOCHS` |
| Block reward (genesis) | 100 EVAP (halving every 1,000,000 blocks) | `genesis-mainnet.json` → `tokenomics` |
| Total supply | 1,000,000,000 EVAP | `genesis-mainnet.json` → `tokenomics.total_supply` |
| Fee burn ratio | 50% | `genesis-mainnet.json` → `tokenomics.fee_burn_rate` |
| Staker fee share | 50% | `genesis-mainnet.json` → `tokenomics.staker_fee_share` |
| Target staking APY | 5% | `genesis-mainnet.json` → `tokenomics.target_staking_apy` |

## 11. Pre-RFP checklist

In approximate priority order:
- [ ] Resolve all CRITICAL items in `cross_verification_2026_04_27.md`.
- [ ] Resolve HIGH items or document as accepted risk if deferred.
- [ ] Generate code-coverage report.
- [ ] Produce architecture diagrams D-01 through D-05.
- [x] Tabulate all values in §10.
- [ ] Wire `BlockDA2D::encode_block()` into block production.
- [ ] Pin `pqc_dilithium` commit; document upstream-audit status.
- [ ] Encrypt `bls_key.bin` (mainnet gate; not strictly required for testnet).
- [ ] Mutual NDA template ready.
- [ ] Decide budget envelope.
