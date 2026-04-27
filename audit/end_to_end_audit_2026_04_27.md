# EvaporChain — End-to-End Audit & Mainnet Readiness

**Date:** 2026-04-27
**Method:** 6 parallel domain audits (consensus / crypto+ZK / execution+VM / DA+network+sharding / economics+governance+oracle+operational / privacy+state+storage), each given the prior audit baseline as "already known — don't re-flag." Followed by head-to-head verification of the most explosive new claims to filter agent over-flags.

**Companions:**
- `cross_verification_2026_04_27.md` — 6 contested findings (4 confirmed, 1 false positive, 1 unverified)
- `audit_readiness_pack_2026_04_27.md` — trust model, scope, invariants
- `external_audit_rfp_2026_04_27.md` — engagement brief, firm shortlist
- `FULL_AUDIT_2026_04_24.md` — internal multi-agent baseline

This doc is the synthesis. It separates what's real from what's noise.

---

## Headline verdict

| Frame | Score | Reasoning |
|---|---|---|
| Code as code (does it compile, run, hold its invariants under benign use) | ~85% | 4,486 tests pass, 3-Mini testnet stable, primitives mostly correct |
| Code as code under adversarial use | ~60% | Multiple real CRITICAL gaps in oracle/governance/DA wiring/upgrade-contract |
| Operational readiness for mainnet launch | ~30% | No genesis ceremony, placeholder genesis values, no monitoring runbook, no formal upgrade path, no public testnet |
| External validation | 0% | No external audit started; ~5-6 month timeline once RFP issues |
| **Honest weighted "to mainnet"** | **~50%** | **8-14 months of focused work, contingent on funding for audit + ceremony + ramp** |

The README's "audit-ready" line is half-true: the code is *audit-rfp-ready*, not audit-passing.

---

## Method note — what to trust in this doc

Every CRITICAL or HIGH below is either:
- **Verified head-to-head** by reading the actual file (✅), or
- **Read by an agent only, plausible, deferred** (⚠ — needs your other session to confirm before acting), or
- **Already covered in `cross_verification_2026_04_27.md`** (cross-ref only).

I'm calling out **2 confirmed false positives** from the agent reads at the end so they don't leak into your fix list.

---

## 1. Consensus + slashing + view-change

**Verified-real new findings:**
- ✅ **HIGH — Finality monotonicity removed** (already in cross-verification §1).
- ⚠ **MEDIUM — DA attestation may lack canonical domain separation tag.** Worth a 5-min check on `da_attestation.rs:create_attestation()`.
- ⚠ **LOW — Proposer randomness beacon falls back to `hash(epoch)` if uninitialized.** Bootstrap-only risk; first-block proposer predictable.

**Agent over-flag (verified false):**
- ❌ "Vote equivocation counted in quorum tally" — I read `tendermint.rs:1364-1378` directly. The second equivocating vote triggers slash and **early-returns at line 1375 BEFORE** the insert at line 1378. First vote stays, second is rejected. No double-count. **NOT a real bug.**

**Already-confirmed solid:**
- Stake-weighted 2/3 quorum (`bridge.rs:79`)
- BLS aggregate sig verification (`bridge.rs:82-103`)
- Locked-block / locked-round mechanism (Tendermint correctness)
- Equivocation slashing 10% triggers correctly (separate from the rejected over-flag)
- View-change exponential timeout (no infinite proposer-skip)
- MAX_ROUNDS_PER_HEIGHT = 10 with reset (no quorum bypass)
- Trusted-checkpoint long-range defence

**Domain readiness: 75%.** Top gap = finality monotonicity (already tracked).

## 2. Cryptography + ZK proving

**Verified-real new findings:**
- ⚠ **MEDIUM — Poseidon constants are documented as unaudited** (already-known H-15, but the parameter generation method, MDS matrix, and 56-partial-rounds margin have NOT been reviewed by a ZK cryptographer). External cryptographer review is non-negotiable before mainnet.
- ⚠ **MEDIUM — BLS proof-of-possession is implemented but NOT enforced at validator registration.** Rogue-key attack surface. Find: `signatures.rs:435-443` has `proof_of_possession()`; consensus path doesn't require it.
- ⚠ **LOW-MEDIUM — Evaporation proof Fiat-Shamir transcript not bound to block state root.** Possible batch-mixing across independent proof statements.
- ⚠ **LOW — Verkle proof reconstruction may accept omitted siblings under specific peak-bagging shapes.** Property test would catch.

**Agent over-flag (verified false):**
- ❌ "HybridVerifier OR-logic is unsafe" — I read `signatures.rs:344-352`. Mismatched hybrid-pk/non-hybrid-sig combinations fall into `verify_hybrid` → `split_sig` returns `None` → returns `false`. Fails closed. No downgrade attack.

**Already-confirmed solid:**
- ML-DSA Drop zeroize (`signatures.rs:39-49`)
- BLS DST + PoP DST domain separation
- OsRng across crypto crate (no `thread_rng` outside tests)
- MMR append correctness, Verkle inclusion logic
- Hybrid keypair generation independence

**Domain readiness: 70%.** Top gap = external Poseidon parameter audit.

## 3. Execution + STM + EvaporScript VM + contracts

**Verified-real new findings:**
- ✅ **CRITICAL — UpgradeContract is a no-op** (cross-verification §3, also seen by execution agent).
- ⚠ **HIGH — Address type mismatch in template-contract privilege check.** `contracts/lib.rs:1659` and similar lines compare `caller_hex` (String) with `ts.owner` (String), then `caller` (AccountAddress) with `creator` (AccountAddress). Mixing string-encoded and bytes-encoded addresses is an access-control footgun. Verify hex casing/padding canonicalization.
- ⚠ **HIGH — UserOp paymaster balance check non-atomic with deduction across Block-STM replays.** No paymaster nonce. Replay-drain possible if MVCC retries.
- ⚠ **MEDIUM — Storage rent feature is stubbed.** `storage_deposit` and `storage_bytes` fields are initialized to 0 and **never written** by execution paths. Sender can claim arbitrary storage with no prepayment.
- ⚠ **MEDIUM — No bytecode verification at deploy time** (only runtime). Pathological scripts can be deployed; runtime gas/step caps mitigate but don't prevent ongoing DoS.
- ⚠ **MEDIUM — Block-STM `MAX_ABORTS_BEFORE_SERIAL = 3` × `max_waves = num_txs + 2`** allows O(N²) re-execution worst case on pathological dependency cycles.

**Already-confirmed solid:**
- Reentrancy bounded by MAX_CALL_DEPTH (`lib.rs:141,643`)
- Block-STM checked arithmetic (`block_stm.rs:540`, etc.)
- Failed-tx revert keeps fee burn (`lib.rs:1334-1348`)
- All 44 opcodes have explicit gas costs (verified by execution agent; opcode count corrected from earlier "91" — actual `compiler.rs:11 enum Op` has 44 variants)
- Stack/memory bounds enforced (1024 / 4MB)
- 6 contract templates have caller==owner / caller==creator gates
- Multisig dedup, MMR bounds, script jump validation
- Paymaster gas calculation correct (`lib.rs:1015` — checked `+ GAS_USER_OP`)

**Domain readiness: 65%.** Top gap = UpgradeContract handler + storage rent enforcement.

## 4. Data availability + network + mempool + sharding

**Verified-real new findings:**
- ✅ **CRITICAL for mainnet — DA 2D erasure NOT wired into block production** (already in cross-verification, but worth re-emphasising). Library exists (`da/src/block_da_2d.rs`); call site missing in `consensus/lib.rs:238 produce_block()`. Until this is wired, DA certificates run over a sentinel `data_root` and the system has no actual DA enforcement.
- ⚠ **HIGH — NMT namespace 0 may not be enforced at tree construction.** Cross-check `da/src/namespace.rs:from_blobs` for explicit reject.
- ⚠ **MEDIUM — RS reconstruction allows selective cell disclosure.** Prover can choose which k cells to reveal; doesn't prove the omitted cells are sound. Light-client sampling soundness depends on this.
- ⚠ **MEDIUM — Empty-block `data_root` ambiguity.** `None` (sentinel) vs `Some([0u8; 32])` (a non-empty block hashing to all-zeros) are not protocol-distinguished.
- ⚠ **MEDIUM — No global mempool BYTE cap.** Per-account count limit is enforced; total bytes only recalculated, not capped. N attackers × 64 txs × 128 KiB = ~80 MiB OOM surface.
- ⚠ **MEDIUM — Gossip mesh fanout = 3** in libp2p config. Tight for permissionless deployment; eclipse risk if peer count is small.
- ⚠ **LOW — Cross-shard receipt dedup uses message_id only.** No timestamp/ordering tie-breaker.

**Already-confirmed solid:**
- DA cert supermajority (`da/poha.rs:131`)
- Network DA sampling wired (`network/service.rs:62`)
- Mempool sig-verify before admit (`mempool.rs:140-152`)
- Per-account mempool limit (64 txs)
- Stake-weighted shard assignment (round-robin, no concentration risk)
- Sampling seed unmanipulable (block_number + validator_id + sample_index)

**Domain readiness: 55%.** Top gap = DA encoder not wired into block production.

## 5. Economics + governance + oracle + operational

**Verified-real new findings:**
- ✅ **CRITICAL — Governance applies arbitrary parameter values with no bounds check.** I read `execution/lib.rs:951` directly. `db.put_governance_param(param_key, param_value)` is called with no range or type validation on any parameter. Combined with:
- ✅ **CRITICAL — Whale can pass any proposal solo.** I read `execution/lib.rs:942-947, 949`. Vote weight = `voter_balance` (balance, not stake). Pass condition is `votes_for > votes_against * 2` with **no quorum requirement**. From `genesis-mainnet.json`: Foundation Treasury holds 350M of 1B total supply (35%). A single Foundation vote passes any proposal alone.
- ✅ **HIGH — Bootstrap peers in genesis are placeholders** (`/ip4/0.0.0.0/tcp/9000-9003`). MUST be replaced for real mainnet.
- ✅ **MEDIUM — `genesis_time: 2026-10-01T00:00:00Z`.** End of your sprint window. If mainnet launches earlier, epoch arithmetic breaks.
- ⚠ **HIGH — TWAP can be single-block manipulated.** No `len() ≥ 3` minimum history before computing.
- ⚠ **HIGH — Oracle does not check `validator_id` is in the actual validator set.** Combined with the broken sig check (cross-verification §2), oracle has effectively no authentication.
- ⚠ **HIGH — 271-298 `unwrap()`/`expect()` calls in consensus crate**, including on attacker-controllable bytes (`bridge.rs:542 BlsVerifier::aggregate_signatures(&sigs).unwrap()`, NaN compare in oracle votes). Crash DoS surface.
- ⚠ **MEDIUM — Min stake for proposal creation: none.** Spam proposals possible.
- ⚠ **MEDIUM — No emergency pause / timelock between proposal pass and parameter application.** Bad proposal applies immediately.
- ⚠ **MEDIUM — Inflation parameters governance-tunable with no cap.** `block_reward = u64::MAX` is settable.

**Already-confirmed solid:**
- Reward accumulator wired (block rewards + staker distribution)
- Validator unbonding period
- Vote liveness slashing
- Governance double-vote guard (HashSet voter tracking)
- Distinct chain_id mainnet vs testnet
- Prometheus metrics + alert rules exist (`deploy/prometheus.yml`)

**Economic / governance / oracle readiness: 50%.** Top gaps = governance whale-pass + unbounded params, oracle no-auth, TWAP manipulation.
**Operational readiness: 40%.** Top gaps = genesis placeholders, 298 consensus unwraps, no emergency pause, no documented upgrade path, no genesis ceremony.

## 6. Privacy + state + storage

**Verified-real new findings:**
- ⚠ **HIGH — Persistence panics on RocksDB write failure.** `rocksdb_backend.rs:338, 388` use `.expect("write object to RocksDB")`. Disk-full / permission failure → node panic mid-block. Should propagate `Result` and halt the chain gracefully.
- ⚠ **MEDIUM — Snapshot finality not enforced.** `SnapshotBuilder::create()` doesn't check the height is past reorg-safety window. Sync nodes could load a non-canonical snapshot.
- ⚠ **MEDIUM — Snapshot pruning race.** State root computed without lock; concurrent prune/apply could yield inconsistent snapshot.
- ⚠ **MEDIUM — Nullifier check happens before verification, then again at spend.** In serial execution this is safe; in parallel STM, two privacy txs with the same nullifier could both pass the early check before either spends. Verify which path is taken for privacy txs.
- ⚠ **LOW — No view-key separation, no dummy notes for set-anonymity.** Privacy is correct but anonymity-set is small.

**Already-confirmed solid:**
- Hash-prefixed key encoding (`state/db.rs:10-36`)
- RocksDB rollback (`rocksdb_backend.rs:277-309`)
- WAL replay determinism (no clock/random in payloads)
- Pedersen commitment binding + balance-binding sum-zero check
- Nullifier derivation deterministic + unique
- Merkle membership tree depth fixed at 20
- Snapshot integrity hash + tamper detection
- Ghost-bridge cross-chain replay protection

**Domain readiness: 75%.** Top gap = persistence panic on write failure (one-day fix).

## 7. Tests + dependencies + code quality

- ✅ 4,486+ tests; 19+ adversarial scenarios; proptest harnesses; `fuzz/` directory at repo root.
- ⚠ Code coverage report not yet generated. Auditors will ask. Run `cargo-llvm-cov --workspace` on a Mini before RFP issue.
- ⚠ `pqc_dilithium` upstream unaudited (already-known H-13). Pin commit, document.
- ⚠ Robustness debt: 271-298 unwraps in consensus, 250 in execution, 193 in node. Many on attacker-controllable bytes. Not bugs per se but DoS surface and audit-finding generators.
- ✅ `deny.toml` exists for `cargo-deny`. Run before audit kickoff to catch yanked/advisory deps.

---

## Confirmed false positives from agent reads (don't action these)

1. **Consensus equivocation in tally** — agent claimed both equivocating votes are counted in quorum. **VERIFIED FALSE** — `tendermint.rs:1375` returns early before the insert. First vote stays, second is rejected and slashed.
2. **HybridVerifier OR-logic** — agent claimed mismatched hybrid-pk/non-hybrid-sig is exploitable. **VERIFIED FALSE** — fails closed via `split_sig` returning `None`.
3. **Paymaster gas underchanged** — earlier audit-pass false positive (cross-verification §6).

Do not let these into the fix list.

---

## What "real distance to mainnet" means after this audit

There are three overlapping gaps:

### Gap A — Code (4-12 weeks of focused work)

In must-fix order:
1. Oracle vote real signature verification + validator-set membership check (`oracle/consensus.rs:183-203`).
2. UpgradeContract — either implement properly with `governance_approved` check, or remove the tx variant.
3. DA encoder wired into `produce_block()` so blocks ship with real `data_root` rather than sentinel.
4. Governance — add parameter range bounds, quorum requirement, vote-weight cap, optional timelock.
5. Finality monotonicity — non-blocking variant that allows gap-fill.
6. Validator BLS key encryption (`bls_key.bin`) before mainnet.
7. Persistence panic → graceful halt.
8. Reduce critical-path unwraps; prioritise BLS sig aggregation, attestation deserialization, NaN-compare paths.
9. NMT namespace 0 enforcement at construction.
10. Storage rent enforcement (or remove fields).

### Gap B — Operational (8-16 weeks calendar)

1. Genesis ceremony — replace placeholders, validator key collection, real bootstrap peers, real `genesis_time`.
2. Public testnet (current 3-Mini Tailscale doesn't exercise Sybil/eclipse/network-partition).
3. Monitoring + alerting + incident-response runbook.
4. Upgrade path — runtime upgrades vs hard fork, backward compatibility strategy.
5. Snapshot + restore tested, documented.
6. Architecture diagrams (auditors will ask).
7. Tabulated parameters (`audit_readiness_pack §10` TODO).
8. Bug bounty program.
9. Validator recruitment + community.

### Gap C — External validation (5-6 months calendar, capital-dependent)

1. RFP to Trail of Bits / Sigma Prime / Zellic (per `external_audit_rfp_2026_04_27.md`).
2. Engagement, audit, fix iteration, re-audit.
3. Optional: Runtime Verification or NCC Group for primitive review.
4. **Cost: £150-400K depending on scope and firm.**

These gaps run in parallel after Gap A is mostly closed. RFPs CAN be issued while operational work continues, but firms will downgrade their estimate if cross-verification CRITICALs are still open at kickoff.

---

## Honest mainnet timeline

| Scenario | ETA from today |
|---|---|
| Best case — all 3 gaps run in parallel, no audit cascade, capital available | 8 months |
| Realistic — typical execution drift + some audit findings deepen scope | 11-13 months |
| Conservative — solo, no audit-grade capital ready, sprint discipline slips | 14-18 months |

Your 6-month sprint (May-Oct 2026) is enough to close Gap A and start Gap B. It is **NOT** enough to launch mainnet by October 2026 — that timeline doesn't include the external audit (5-6 months by itself).

If you set `genesis_time` to "2026-10-01" because that was the sprint endpoint, the genesis time is aspirational, not real. Remove it from `genesis-mainnet.json` or move it to a separate `mainnet-target.json` so the mainnet config doesn't carry a deadline you can't hit.

---

## Final answer to "is it good to work on?"

The codebase is **genuinely strong technology**. ~85% of "code as code" works. ~60% under adversarial scrutiny. Most of the gap is fixable engineering, not architectural. The novel primitive (energy-based decay) is real and well-implemented.

But the path from here to a real mainnet you can defend in production is **8-14 months** and **£200K-500K** of audit + ceremony + ramp spend, plus continued solo discipline. As technology research / portfolio / capability-proof, this is exceptional work and worth continuing. As a commercial L1 launch under solo unfunded conditions, the ROI vs FINGAURD or CardioSafe over the same 6 months is unfavourable.

The mandate in your `CLAUDE.md` already names this: *"Target is 'human history's best blockchain,' not a shipped product."* This audit confirms that framing is the honest one.

If you want to keep building: focus the next 6 months on Gap A (the 10 code fixes above) + start Gap B (genesis, testnet, monitoring). Issue the RFP after Gap A is closed. Defer "mainnet launch" decisions until Gap C produces a clean external audit and you have capital to ramp validators. That's the realistic path.
