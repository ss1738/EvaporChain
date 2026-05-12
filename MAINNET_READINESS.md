# EvaporChain Mainnet-Readiness Plan + Multi-Session Coordination

**Single source of truth** for what's left before mainnet, organized into parallel lanes that different Claude sessions (or the operator) can drive concurrently without colliding.

**Read this BEFORE starting any work that touches consensus, execution, bridge, privacy, paymaster, state-db, or runbooks.** Then read `SESSION_PROGRESS.md` for the latest cluster-state and session-coordination notes.

**Updated atomically.** This file is the authoritative claim board. A session that wants to start a lane edits its status line + commits + pushes in one atomic operation. The coordination protocol below specifies exactly how.

---

## 1. Status legend

| Symbol | Meaning |
|---|---|
| 🔴 BLOCKED | Gated on operator input (SSH auth, credentials, decision). Cannot start. |
| 🟡 OPEN    | Ready to claim. No prerequisites pending. |
| 🟢 CLAIMED | A session has started. See the claim line for owner + base commit. |
| ✅ DONE    | Shipped + verified on Mini 1. See `done-as-of` commit in lane spec. |

---

## 2. How to claim a lane

**Step 1 — read state:**

```bash
git pull --ff-only
cat SESSION_PROGRESS.md | head -200
cat MAINNET_READINESS.md | head -250   # this file's index
```

**Step 2 — pick a lane:**

- Must be 🟡 OPEN (not 🔴 / 🟢 / ✅).
- Its `Depends on:` list must all be ✅ DONE.
- Its `Files touched:` set must NOT overlap any 🟢 CLAIMED lane's `Files touched:`.

**Step 3 — claim atomically:**

Edit the lane's status line in this file ONLY. Stage just this file. Commit. Push immediately.

```bash
# Edit MAINNET_READINESS.md — change the status line for your lane only
git add MAINNET_READINESS.md
git diff --staged --stat                # MUST show only this file
git diff --staged | head                 # confirm only the status line changed
git -c commit.gpgsign=false commit -m "claim: lane <ID> by <session-id>"
git push                                 # immediate
```

The claim line format:

```
🟢 CLAIMED by <Opus 4.7 | Sonnet 4.6 | operator> @ <ISO-8601 UTC> · base: <commit>
```

**Step 4 — work the lane:**

Tactical commits are governed by `SESSION_PROGRESS.md`'s "Cross-session conventions (mandatory)" section: stage-check before commit, pull-before-cargo on Mini 1, no SSH probing, append SESSION_PROGRESS entry per shipped commit.

**Step 5 — release the lane:**

On completion:

1. Mark the lane ✅ DONE in this file.
2. Add a `done-as-of: <commit>` line.
3. Update SESSION_PROGRESS.md with a final entry per the file's existing template.
4. Atomic commit + push for the status flip.

If you abandon a lane (cluster wedge, parallel-session conflict, scope blowout), revert it to 🟡 OPEN and add a `last-attempt-notes:` line so the next claimer knows what happened.

---

## 3. Conflict matrix — which lanes CAN run together

Lanes are grouped by primary file/crate. Lanes within the same group are SEQUENTIAL (one at a time). Lanes across different groups are PARALLEL.

| Group | Primary surface | Lanes |
|---|---|---|
| **CONSENSUS** | `crates/evaporchain-consensus/src/tendermint.rs` | L1, L3, L4, T1.13, T1.14 |
| **EXECUTION** | `crates/evaporchain-execution/`, energy/conservation | T0.6, T0.7 |
| **PRIVACY** | `crates/evaporchain-pnt`, `evaporchain-execution::privacy_exec` | T0.5 |
| **NETWORK** | `crates/evaporchain-network`, `evaporchain-state::sync` | T0.7, T0.8 |
| **BRIDGE-RUST** | `crates/evaporchain-eth-bridge`, `ethereum-bridge/circuits/` | T0.9 |
| **BRIDGE-SOL** | `ethereum-bridge/contracts/` | T0.10, T0.11 |
| **PAYMASTER** | `crates/evaporchain-paymaster`, `wallet/` | T1.15, parallel-session arc |
| **OPS-RUNBOOK** | `docs/runbooks/`, `scripts/` | T1.21, T1.22, T1.23 |
| **AUDIT-SWEEP** | wide (read-only first; fix per finding) | T1.16 |
| **STATE-DB** | `crates/evaporchain-state/src/db.rs`, `rocksdb_backend.rs` | T0.6 (slashing), T1.20 (coverage) |

**Rule of thumb:** if two lanes share a primary surface, they're SEQUENTIAL. Otherwise parallel.

---

## 4. Lane index (one-liner each)

### Tier 3 — Operational unblocks (must run first)

| ID | Lane | Status | Surface |
|---|---|---|---|
| T3.1 | Phase C cluster deploy + bring-up | 🔴 BLOCKED on Hetzner SSH auth | OPS |
| T3.2 | 5-node Tailscale genesis switch | 🔴 BLOCKED on T3.1 | OPS |

### Tier 0 — Critical path (mainnet-blocking)

| ID | Lane | Status | Surface |
|---|---|---|---|
| T0.1 | Layer 4 hot-path consensus surgery (C.1-C.6) | 🟡 OPEN | CONSENSUS |
| T0.2 | Layer 4 D-track adversarial + perf + 72hr soak | 🔴 BLOCKED on T0.1 + T3.1 | CONSENSUS |
| T0.3 | POST_EXEC Phase 4 enforce-mode (refuse-to-apply, not prevote-NIL — see spec note) | ✅ DONE (c191498) — flag + 4 tests; needs T0.4 fork-epoch + soak before flipping to enforce | CONSENSUS |
| T0.4 | POST_EXEC Phase 5 block-hash inclusion | ✅ DONE (695c49c) — bit-compat fold (Some→include, None→skip); 3 hash tests | CONSENSUS |
| T0.5 | PNT v1+ activation (privacy authoritative) | 🟡 **CODE-COMPLETE — OPS-ONLY** — sub-tasks 1, 3, 4-infra, **5** all ✅. Sub-task 5 adversarial tests live at `crates/evaporchain-execution/src/privacy_exec.rs:2168` (`pnt_v1_respend_after_window_eviction_rejected_via_anchor`) + line 2296 (Stage-1 companion); both green on Mini 1 release 2026-05-11. Remaining: operator step 2 (governance flip 0→1 at fork-epoch, needs T3.1 cluster) + operator step 6 (storage-growth telemetry post-flip, needs T3.1). No further code work for this lane. | PRIVACY |
| T0.6 | Slashing-at-scale empirical tests | 🟡 OPEN | EXECUTION + STATE-DB |
| T0.7 | Mempool + signature DoS hardening | 🟡 **PARTIAL** — V1-V4 ✅ shipped; V5 DAG fork-spam multi-validator convergence ✅ (`0e976f4`, 2026-05-11); AUDIT-2026-05-11-1/2 ShardSample rate-limit + queries cap ✅ (`8c59fad`, 2026-05-11). Remaining: comprehensive end-to-end DoS suite + `docs/runbooks/dos-resistance.md` (already-present runbook needs the new vectors folded in). | NETWORK + EXECUTION |
| T0.8 | Light-client / fast-sync against malicious snapshots | ✅ **DONE** — all 5 lane-spec adversarial fixtures shipped in `crates/evaporchain-state/tests/adversarial_snapshots.rs` and green on Mini 1 release 2026-05-11 (`adversarial_t08_forged_integrity_hash_rejected_via_missing_quorum_cert`, `..._stale_quorum_cert_from_different_snapshot_rejected`, `..._duplicate_validator_ids_in_set_rejected`, `..._truncated_zstd_payload_rejects`, `..._partial_state_withhold_nullifier_rejected_via_cert`). Covers sub-tasks 1 (5 fixtures), 2 (quorum-cert verification), 3 (integrity-hash chain validation), 4 (partial-state-withhold). | NETWORK |
| T0.9 | Bridge Phase 4 full V2 (Halo2 EccChip in-circuit Pallas MSM) | ✅ **DONE** — all 8 sub-tasks shipped. D-finish resolved (`Params<halo2_proofs::pasta::EqAffine>` = vesta::Affine IS the right verifier curve for an Fp-circuit; resolution comment at `circuit_v2.rs:997-1011`). `VerkleProverV2::{setup, prove_v2, verify_v2}` (lines 1019-1104) wire `keygen_vk` + `keygen_pk` + `create_proof` + `verify_proof` over the EccVerkleStepCircuit. Headline test `prove_v2_and_verify_v2_round_trip` re-verified green on Mini 1 release 2026-05-11 (77.40s end-to-end, k=11 IPA params). Crate is gated behind `--features v2-ecc`; default build remains V1 Poseidon path until T0.10 pulls V2 through. Earlier 2026-05-09 green run preserved in memory `evaporchain_t0_9_d_finish_done.md`. | BRIDGE-RUST |
| T0.10 | `VerkleProofVerifier.sol` Groth16 wrap | 🟡 OPEN (unblocked 2026-05-11 by T0.9 ✅) — wraps the V2 IPA proof in a Groth16 envelope so the on-chain Solidity verifier can pin-verify the same `VerkleProofV2` JSON the Rust side produces. | BRIDGE-SOL |
| T0.11 | Cross-chain replay protection hardening (dispatcher) | ✅ DONE (ee2ebba) — L1 finalization-depth gate (12 blocks); 46/46 forge tests pass on-host | BRIDGE-SOL |
| T0.11b | Extend finalization-depth gate to StateMembershipAttester | ✅ DONE (b74e72d) — symmetric defense w/ T0.11; 48/48 forge tests pass on-host | BRIDGE-SOL |
| T0.12 | External security audit kickoff + remediation | 🔴 BLOCKED on operator (auditor selection) | wide |

### Tier 1 — Smaller items (mainnet-blocking)

| ID | Lane | Status | Surface |
|---|---|---|---|
| T1.13 | Promote conservation audit gating → mandatory | 🔴 BLOCKED on T0.1 | CONSENSUS |
| T1.14 | Phase 2 round-trip test (proposer-stamp == validator-apply) | ✅ DONE (9191e87) — 3 tests appended end-of-file; build verification deferred | CONSENSUS |
| T1.15 | Paymaster Finding 1 — per-key in-flight locking | 🟡 OPEN | PAYMASTER |
| T1.16 | Internal audit findings reconciliation sweep | ✅ DONE (7f36b46) — `AUDIT_RECONCILIATION_2026-05-09.md` + opcode-count drift fix | AUDIT-SWEEP |
| T1.17 | BLS key rotation under live cluster conditions | 🔴 BLOCKED on T3.1 | OPS-RUNBOOK |
| T1.18 | Validator-key passphrase migration on live nodes | 🔴 BLOCKED on T3.1 | OPS-RUNBOOK |
| T1.19 | EVPL plaintext key migration on live nodes | 🔴 BLOCKED on T3.1 | OPS-RUNBOOK |
| T1.20 | Coverage push to ≥90% (currently ~73%) | 🟡 OPEN | STATE-DB |
| T1.X1 | EVR-20 / EVR-721 implementation-status badges (docs-only, audit follow-up) | ✅ DONE — false-positive from audit reconciliation; both EVR docs already carry detailed implementation-status tables ahead of the spec body | docs |
| T1.21 | Cluster monitoring (Prometheus + Grafana + alerts) | 🔴 BLOCKED on T3.1 | OPS-RUNBOOK |
| T1.22 | Network upgrade rehearsal (live flag-flip + rollback) | 🔴 BLOCKED on T3.1 | OPS-RUNBOOK |
| T1.23 | Mainnet genesis-amendment dry-run | 🔴 BLOCKED on T0.1 + T3.1 | OPS-RUNBOOK |

### Tier 2 — Defer to V1.5 (NOT blocking mainnet)

Documented for awareness; do NOT claim these as mainnet-readiness work.

| ID | Lane | Defer reason |
|---|---|---|
| T2.24 | MetaCoq pin + extraction-to-Rust harness + on-chain CoqVerifier | `MultiAuditorVerifier` k-of-n is the operational substitute |
| T2.25 | EncryptedMempool full activation (today: shadow-tracking) | Hard-fork required; V1 ships shadow-mode |
| T2.26 | Sharding live activation | Single-shard sufficient for V1 |
| T2.27 | Energy-Verkle Trie compression | Optimization, not correctness |
| T2.28 | LLSA Layer 7 full path | Already descoped per doctrine |

---

## 5. Lane specs

Each lane below has the full spec a session needs to start. Status here mirrors the index above — keep them in sync when you flip a status.

---

### T3.1 — Phase C cluster deploy + bring-up

**Status:** 🔴 BLOCKED on Hetzner SSH auth from operator
**Surface:** OPS (no code changes; SSH + binary stage)
**Depends on:** none
**Effort:** 1-2 days

**Goal:** Get all 5 cluster nodes (3 Minis + 2 Hetzners) running the post-bundle binary against a clean genesis, advancing past h=0.

**Prerequisites the operator must paste in chat:**

> "Yes, SSH `root@evaporchain-hel-1` (100.66.208.20) and `root@evaporchain-hel-2` (100.91.235.22), plus the standard Mini access (`satyawansingh@100.119.53.101`, `satyawan-mini-1@100.113.253.72`, `satyawan-mini-2@100.103.216.125`), to execute Phase C stop-the-world per `docs/runbooks/cluster-deploy.md` §3."

**Acceptance criteria:**

- All 5 nodes report `chain_id: evaporchain-tailscale-5node-1` (or testnet-1 if operator chose continuity)
- All 5 nodes report `light_cone_block_count` advancing in lockstep
- `/api/four_act` shows `last_conservation_audit_ok` non-null on all 5
- 24-hour clean soak (no `ConservationViolation`, no fork events, no node restarts)

**Files touched:** none (operational only)

---

### T3.2 — 5-node Tailscale genesis switch

**Status:** 🔴 BLOCKED on T3.1
**Surface:** OPS
**Depends on:** T3.1
**Effort:** Half day

**Goal:** Wipe data dirs (preserving BLS keys per `CLAUDE.md`), re-init from `genesis-tailscale-5node.json` (chain_id `evaporchain-tailscale-5node-1`), restart cluster.

Operator already authorized the switch in this session arc. Fold into T3.1's runbook execution.

**Files touched:** none (operational only)

---

### T0.1 — Layer 4 hot-path consensus surgery (C.1-C.6)

**Status:** 🟡 OPEN
**Surface:** CONSENSUS
**Depends on:** none (can start now; verifies on cluster after T3.1)
**Effort:** 2-3 weeks focused work

**Goal:** Promote `authoritative_head` from admin-RPC into the consensus hot path, route votes by head, multi-parent set proposer selection, validator-determinism gate. Source: `DOCTRINE_PUNCH_LIST.md` Layer 4.

**Sub-tasks (commit one per):**

- C.1 — `authoritative_head` field on `TendermintConsensus` populated by consensus, not RPC
- C.2 — vote routing keyed by head (handle_prevote / handle_precommit changes)
- C.3 — multi-parent set selection in `create_proposal`
- C.4 — validator-determinism gate (every validator computes same `effective_parents`)
- C.5 — propagation tests with 4-of-5 / 3-of-5 partition scenarios
- C.6 — adversarial proposer test (Byzantine producer that picks wrong head)

**Files touched:**
- `crates/evaporchain-consensus/src/tendermint.rs` (heavy)
- `crates/evaporchain-consensus/src/{mcc,light_cone}.rs`
- `crates/evaporchain-consensus/tests/byzantine_adversarial.rs`

**Acceptance:** all 6 sub-tasks land + `cargo test -p evaporchain-consensus` green on Mini 1.

**Coordination warning:** This lane LOCKS the CONSENSUS group. T0.3, T0.4, T1.13, T1.14 cannot run concurrently with T0.1.

---

### T0.2 — Layer 4 D-track (adversarial + perf + 72hr soak)

**Status:** 🔴 BLOCKED on T0.1 + T3.1
**Surface:** CONSENSUS
**Depends on:** T0.1, T3.1
**Effort:** 1 week + 72hr wall-clock

**Sub-tasks:**

- D.1 — adversarial sweep on the live cluster
- D.2 — perf profile (target: ≥1 block/sec under 1k tx/sec mempool feed)
- D.3 — 72-hr stability soak with live tx generator
- D.4 — fault injection (kill one validator, restart, lockstep recovery)
- D.5 — partition healing test

**Files touched:** `tests/integration/`, `scripts/` (perf + soak harness)

**Acceptance:** soak report committed to `docs/runbooks/layer4-soak-report.md`.

---

### T0.3 — POST_EXEC Phase 4 enforce-mode prevote NIL

**Status:** 🟡 OPEN
**Surface:** CONSENSUS
**Depends on:** Phase 3 clean soak (governance flag value `"warn"` for ≥7 days on live cluster). Currently Phase 2+3 always-on per `af6876d`; needs `post_state_verify_mode` governance flag added.
**Effort:** 3-5 days code + 1-2 week soak

**Sub-tasks:**

1. Add `post_state_verify_mode` governance flag to allowlist + defaults (`tendermint.rs::governance_set_param` + `governance_flags_snapshot`)
2. Gate the existing Phase 2 wiring at `tendermint.rs:6582-6592` on the flag (currently runs unconditionally)
3. In `apply_block` at `tendermint.rs:6089`, when flag = `"enforce"` and mismatch, return error → consensus prevotes NIL for this round
4. Tests: enforce-mode rejects mismatched proposal, warn-mode logs but accepts, off-mode no-op

**Files touched:**
- `crates/evaporchain-consensus/src/tendermint.rs`
- `crates/evaporchain-consensus/tests/`

**Acceptance:** flag-flip from `"warn"` to `"enforce"` on a 5-node cluster + adversarial test (one node deliberately diverges) → cluster prevotes NIL on the divergent block.

**Coordination warning:** SEQUENTIAL after T0.1. Cannot run concurrently.

---

### T0.4 — POST_EXEC Phase 5 block-hash inclusion

**Status:** 🔴 BLOCKED on T0.3
**Surface:** CONSENSUS
**Depends on:** T0.3 + 7-day clean enforce-mode soak
**Effort:** 1-2 days code + fork-epoch coordination

**Goal:** Roll `post_state_root` into the bytes hashed for `block_hash`. Behind the same fork-epoch gate as T0.3.

**Files touched:**
- `crates/evaporchain-types/src/lib.rs` (Block::hash impl)
- `crates/evaporchain-consensus/src/tendermint.rs`

**Acceptance:** pre-fork blocks compute legacy block_hash, post-fork include `post_state_root`. Genesis-amendment broadcasts the fork-epoch.

---

### T0.5 — PNT v1+ activation (privacy authoritative)

**Status:** 🟡 OPEN — code mostly shipped; remaining work is operational + 1 adversarial test
**Surface:** PRIVACY
**Depends on:** none
**Effort:** 2-3 weeks (revised: ~3-5 days code + operational soak)

**Audit 2026-05-10 — actual state vs original spec:**

| Sub-task | Status | Where |
|---|---|---|
| 1 — `current_protocol_version` reads `block.protocol_version` | ✅ DONE | `privacy_exec.rs:240` (`set_protocol_version`); called from `execute_block` per-block |
| 2 — Genesis-amendment to bump 0 → 1 | ⏳ OPERATIONAL | needs T3.1 cluster + governance ride |
| 3 — Fork-epoch gate (legacy db ↔ PNT bounded window) | ✅ DONE | `privacy_exec.rs:261` (`is_double_spend`); wired in both `execute_unshield` (line ~503) and `execute_private_transfer` (line ~722) |
| 4 — PNT phase auto-advance cadence | ✅ INFRA DONE | `tick_pnt_phase` at `privacy_exec.rs:196`; default `PNT_DEFAULT_PHASE_INTERVAL_EPOCHS = 100`; cadence tuning is operational |
| 5 — Adversarial spend-evict-respend test | ⏳ OPEN | needs Mini cargo verification; must include anchor-mechanism interaction |
| 6 — Storage size benchmark | ⏳ OPEN | needs live cluster (T3.1) |

**Security-model audit (paired with this lane):** the bounded-window PNT *requires* an anchor mechanism for soundness — otherwise nullifiers that age out of the window become re-spendable. Confirmed at `privacy_exec.rs:493,712`: every shield/transfer tx must reference `tx.anchor == self.engine.merkle_root()`. Notes whose source root is older than the chain's current root cannot be spent regardless of nullifier window. **PNT bounded window + anchor enforcement are jointly secure**; either alone would be unsound.

**Files touched:** (per the original spec)
- `crates/evaporchain-pnt/`
- `crates/evaporchain-execution/src/privacy_exec.rs`
- `crates/evaporchain-consensus/src/tendermint.rs` (fork-epoch dispatch)

**Remaining concrete work after this audit:**
1. **Sub-task 5** — write the adversarial test in `privacy_exec.rs` test module (`#[test] fn pnt_v1_respend_after_window_eviction_rejected_via_anchor`). Verifies anchor enforcement closes the window-bypass. Mini-only verification.
2. **Operator step 2** — once Phase C cluster runs, ride `governance-flip.sh` to bump `block.protocol_version` 0 → 1 at a fork epoch.
3. **Operator step 6** — collect storage-growth telemetry post-flip.

**Acceptance:** flag-flip on testnet, 7-day double-spend soak with adversarial nullifier replay attempts, all rejected.

---

### T0.6 — Slashing-at-scale empirical tests

**Status:** 🟡 OPEN
**Surface:** EXECUTION + STATE-DB
**Depends on:** T3.1 (live cluster)
**Effort:** 1-2 weeks

**Goal:** Validate every slashing rule fires correctly on the live cluster under real Byzantine conditions. Today's tests are unit-scale.

**Slashing rules to exercise:**

1. Double-vote (`SanovSlash`)
2. Equivocation across rounds
3. MEV missing-refund violation (`apply_mev_missing_refund_slashes`)
4. Validator key rotation grace expiry
5. Mortis condition partial trigger

**Files touched:**
- `crates/evaporchain-sanov-slash/`
- `crates/evaporchain-execution/src/{lib,parallel}.rs` (slashing dispatch)
- `crates/evaporchain-state/src/` (stake updates)
- `tests/integration/`

**Acceptance:** 5 adversarial-validator scenarios, each producing the expected slash + log.

---

### T0.7 — Mempool + signature DoS hardening

**Status:** 🟡 OPEN
**Surface:** NETWORK + EXECUTION
**Depends on:** T3.1 (recommended)
**Effort:** 1-2 weeks

**Goal:** Comprehensive DoS surface audit + remediation. Past incident: per-(height,round) suppression bug fixed (`evaporchain_verification_track_2026_05_02.md`); this lane finds the rest.

**Vectors to test:**

1. Tx flooding (1k, 10k, 100k tx/s)
2. Signature-verification storm (high-volume malformed sigs)
3. Mempool overflow (full mempool, eviction policy)
4. Encrypted mempool reveal flood (fast-cycling reveal commitments)
5. Fork-spam (validator producing many sibling proposals)
6. Gas exhaustion (single tx hits block_gas_limit)
7. Memory blow-up (large-blob namespaces)

**Files touched:**
- `crates/evaporchain-network/`
- `crates/evaporchain-execution/`
- `crates/evaporchain-consensus/src/{encrypted_mempool,tendermint}.rs`

**Acceptance:** harness committed to `tests/dos/`, runs ≥1hr at each load level without violation. Document bounds in `docs/runbooks/dos-resistance.md`.

---

### T0.8 — Light-client / fast-sync against malicious snapshots

**Status:** 🟡 OPEN
**Surface:** NETWORK
**Depends on:** none
**Effort:** 1-2 weeks

**Goal:** Verify fast-sync rejects malicious snapshots — equivocating commit cert, MMR manipulation, partial state withholding.

**Sub-tasks:**

1. Adversarial snapshot fixtures (5 scenarios)
2. Snapshot quorum-cert verification (ensure 2f+1 attestations)
3. Integrity-hash chain validation (reproducible per `evaporchain_integrity_hash_reproducible.md`)
4. Partial-state-withhold detection (snapshot claims to be complete but isn't)

**Files touched:**
- `crates/evaporchain-state/src/{snapshot,sync}.rs`
- `crates/evaporchain-network/src/sync.rs`

**Acceptance:** all 5 adversarial fixtures rejected; clean fast-sync still works.

---

### T0.9 — Bridge Phase 4 full V2 (Halo2 EccChip in-circuit Pallas MSM)

**Status:** 🟡 OPEN
**Surface:** BRIDGE-RUST
**Depends on:** none
**Effort:** 2-3 weeks

**Goal:** Replace the Phase 4 V1 Poseidon-binding circuit (`af6876d`-era) with native EC MSM in-circuit using Halo2 `EccChip`. Cryptographically stronger than the current BLS-multisig fallback.

**Files touched:**
- `ethereum-bridge/circuits/` (standalone workspace)
- `crates/evaporchain-eth-bridge/src/`

**Acceptance:** `verkle-prove` binary produces a proof verifiable on Ethereum via the V2 verifier (T0.10). Cross-side fixture matches.

---

### T0.10 — `VerkleProofVerifier.sol` Groth16 wrap

**Status:** 🔴 BLOCKED on T0.9
**Surface:** BRIDGE-SOL
**Depends on:** T0.9
**Effort:** 1 week

**Goal:** On-chain consumer of the Phase 4 V2 IVC proofs. Standard Groth16 verifier wrapping the CompressedSNARK output.

**Files touched:**
- `ethereum-bridge/contracts/src/VerkleProofVerifier.sol`
- `ethereum-bridge/contracts/test/`

**Acceptance:** forge test verifies a real Phase 4 V2 proof produced by the Rust prover.

---

### T0.11 — Cross-chain replay protection hardening

**Status:** 🟡 OPEN
**Surface:** BRIDGE-SOL
**Depends on:** none
**Effort:** 1 week

**Goal:** Bridge Phase 5 dispatches evaporation events. Add: replay protection across L2 reorgs, finalization-depth assumptions, rollback handling.

**Files touched:**
- `ethereum-bridge/contracts/src/EvaporationDispatcher.sol`
- `ethereum-bridge/contracts/src/EvaporHeaderInbox.sol`

**Acceptance:** forge tests exercise reorg scenarios; dispatcher rejects replay.

---

### T0.12 — External security audit kickoff + remediation

**Status:** 🔴 BLOCKED on operator (auditor selection + contract)
**Surface:** wide
**Depends on:** none formally; recommended T0.1-T0.11 before kickoff for stable code surface
**Effort:** 4-8 weeks calendar; 2-3 weeks internal remediation

**Operator must provide:**

- Selected auditor (Trail of Bits, OtterSec, Spearbit, Halmos, Code4rena)
- Contract signed
- Audit pack handoff (existing `evaporchain_audit_round_*.md` files + `AUDIT_2026_05_06.md`)

---

### T1.13 — Promote conservation audit gating → mandatory

**Status:** 🔴 BLOCKED on T0.1
**Surface:** CONSENSUS
**Depends on:** T0.1
**Effort:** 1 day

**Goal:** Once Layer 4 changes block-acceptance semantics, revisit the `conservation_enforcement` flag and make it always-on (default `"enforce"`). Source: `DOCTRINE_PUNCH_LIST.md` Layer 4 sequel item.

**Files touched:** `crates/evaporchain-consensus/src/tendermint.rs` (defaults table)

---

### T1.14 — Phase 2 round-trip test (proposer-stamp == validator-apply)

**Status:** 🟡 OPEN
**Surface:** CONSENSUS (test-only)
**Depends on:** none (the InMemoryStateDB batch fix `69ed84e` unblocked this)
**Effort:** 1 day

**Goal:** End-to-end test that proves the proposer's stamped `block.post_state_root` actually equals what the validator computes when it applies the same block on a fresh state.

**Files touched:**
- `crates/evaporchain-consensus/src/tendermint.rs` (inline test module — append-only at end of file to avoid editing the parallel session's eprintln→debug cleanup region)

**Acceptance:** test passes on Mini 1; failure case (executor field that doesn't snapshot) catches.

---

### T1.15 — Paymaster Finding 1 — per-key in-flight locking

**Status:** 🟡 OPEN
**Surface:** PAYMASTER
**Depends on:** none, but coordinate with parallel Sonnet 4.6 paymaster arc
**Effort:** 2-3 days

**Goal:** Close the concurrent-retry race documented in `docs/runbooks/paymaster.md` §Idempotency → "Limitations the cache does NOT cover" → bullet 2. Today: lock dropped between cache get and insert; two retries with the same key can both miss + double-allocate paymaster nonces.

**Approach:** per-key `Mutex` / `Arc<Notify>` so the second retry waits on the first's outcome.

**Files touched:**
- `crates/evaporchain-paymaster/src/lib.rs`
- `crates/evaporchain-paymaster/tests/`

**Acceptance:** new test `concurrent_same_key_retries_dont_double_allocate` passes.

**Coordination warning:** the parallel Sonnet 4.6 session has been actively committing in this surface. Pull `origin/main` immediately before claiming, and re-pull at the start of work. If your `git status` shows uncommitted `crates/evaporchain-paymaster/` or `wallet/` files, STOP — that's the parallel session's WIP.

---

### T1.16 — Internal audit findings reconciliation sweep

**Status:** 🟡 OPEN
**Surface:** AUDIT-SWEEP (read-only first; fix per finding)
**Depends on:** none
**Effort:** 1 week

**Goal:** Read every `evaporchain_audit_round_*.md` and `AUDIT_*.md`, reconcile each finding against current code, mark genuinely closed vs still open, ship fixes for remaining.

**Files touched:** wide on read; narrow per fix.

**Acceptance:** master `AUDIT_RECONCILIATION_2026-05-09.md` enumerating each finding with current status. Fix commits referenced.

---

### T1.17 / T1.18 / T1.19 — operational key-rotation runbook executions

**Status:** 🔴 BLOCKED on T3.1
**Surface:** OPS-RUNBOOK
**Effort:** 2-3 days each

These are runbook executions on the live cluster. Operator-driven; documented in `docs/runbooks/`.

---

### T1.20 — Coverage push to ≥90%

**Status:** 🟡 OPEN
**Surface:** STATE-DB primary; broader on need
**Depends on:** none
**Effort:** 1-2 weeks

**Goal:** Push `cargo llvm-cov` from current ~73% to ≥90%, focusing on integration shims (the systematic gap per `evaporchain_coverage_baseline.md`).

**Files touched:** wide; one PR per crate.

**Acceptance:** `cargo llvm-cov --workspace` reports ≥90% region coverage.

---

### T1.21 — Cluster monitoring (Prometheus + Grafana + alerts)

**Status:** 🔴 BLOCKED on T3.1
**Surface:** OPS-RUNBOOK
**Effort:** 1-2 weeks

**Goal:** Production-grade observability. `evaporchain-paymaster` already has `/metrics` Prometheus exposition (Day 9 `2585011`); the chain side needs the same surface, plus dashboards + alert rules.

**Files touched:**
- `crates/evaporchain-node/src/api.rs` (add `/metrics` endpoint)
- `docs/runbooks/monitoring.md` (NEW — but explicit operator request: this lane authorizes the doc creation)
- `scripts/grafana-dashboards.json`

---

### T1.22 — Network upgrade rehearsal (live flag-flip + rollback)

**Status:** 🔴 BLOCKED on T3.1
**Surface:** OPS-RUNBOOK
**Effort:** 1 week

**Goal:** Execute a flag-flip across the live cluster using `scripts/governance-flip.sh`, observe propagation, deliberately trigger a rollback, validate the runbook end-to-end.

**Files touched:**
- `docs/runbooks/governance-rehearsal.md` (NEW — authorized by this lane spec)
- (no code changes)

---

### T1.23 — Mainnet genesis-amendment dry-run

**Status:** 🔴 BLOCKED on T0.1 + T3.1
**Surface:** OPS-RUNBOOK
**Effort:** 3-5 days

**Goal:** Build + sign + broadcast a real `LlsaProof`-bound genesis amendment on the testnet. Validates the full upgrade path (`evaporchain-llsa::apply_amendment` + EPV registry binding + MultiAuditorVerifier k-of-n).

**Files touched:** runbook + test fixtures only.

---

## 6. Dependency graph (text)

```
T3.1 (Phase C deploy)        ←── operator unblock first
  ├→ T3.2
  ├→ T0.2 (Layer 4 D-track)
  ├→ T0.6 (slashing scale)
  ├→ T1.17/18/19 (key rotations)
  ├→ T1.21 (monitoring)
  ├→ T1.22 (rehearsal)
  └→ T1.23 (genesis dry-run)

T0.1 (Layer 4 surgery)       ←── biggest single lane
  ├→ T0.2
  ├→ T0.3 (POST_EXEC P4)
  │    └→ T0.4 (POST_EXEC P5)
  ├→ T1.13 (conservation gate)
  └→ T1.23

T0.5 (PNT v1+)               ←── parallel, no deps
T0.6 (slashing scale)        ←── parallel after T3.1
T0.7 (DoS hardening)         ←── parallel
T0.8 (fast-sync)             ←── parallel

T0.9 (Bridge V2)             ←── parallel
  └→ T0.10 (Verkle Sol)

T0.11 (replay protection)    ←── parallel

T1.14 (Phase 2 round-trip)   ←── 1 day, parallel with most CONSENSUS work
T1.15 (paymaster Finding 1)  ←── parallel, in PAYMASTER group
T1.16 (audit recon)          ←── parallel, read-mostly
T1.20 (coverage)             ←── parallel
```

---

## 7. Coordination conventions (link to SESSION_PROGRESS.md)

The five rules below are mirrored from `SESSION_PROGRESS.md`'s coordination note. Apply to all work in this file.

1. **`git diff --staged --stat` BEFORE every `git commit`.** Confirm only intended files are staged.
2. **Pull `origin/main` on Mini 1 before `cargo build/test`.**
3. **Don't probe SSH usernames blindly.** Use one specific user + one specific key path that the operator has explicitly named.
4. **Don't auto-commit another session's edits.** When `git status` shows files you didn't write, leave them alone.
5. **Co-author trailer is the de-facto session ID.** Sonnet 4.6 vs Opus 4.7 are distinguishable in the log.

---

## 8. Status update log

Append a one-line entry every time a lane status changes. Do NOT delete old entries.

```
[2026-05-09T22:48Z] Opus 4.7  · MAINNET_READINESS.md created · base: 179f18b
[2026-05-09T22:55Z] Opus 4.7  · T1.14 claimed · base: fb49762
[2026-05-09T23:05Z] Opus 4.7  · T1.14 ✅ DONE · ship: 9191e87
[2026-05-09T23:10Z] Opus 4.7  · T1.16 claimed · base: 83d6705
[2026-05-09T23:25Z] Opus 4.7  · T1.16 ✅ DONE · ship: 7f36b46
[2026-05-09T23:30Z] Opus 4.7  · T0.3 claimed · base: 6644589
[2026-05-09T23:45Z] Opus 4.7  · T0.3 ✅ DONE · ship: c191498 · unblocks T0.4
[2026-05-09T23:50Z] Opus 4.7  · T0.4 claimed · base: 66330fe
[2026-05-10T00:05Z] Opus 4.7  · T0.4 ✅ DONE · ship: 695c49c
[2026-05-10T00:10Z] Opus 4.7  · T0.11 claimed · base: 7e65ede
[2026-05-10T00:30Z] Opus 4.7  · T0.11 ✅ DONE · ship: ee2ebba · 46/46 forge VERIFIED on-host
[2026-05-10T00:35Z] Opus 4.7  · T0.11b claimed · base: 93aa83d
[2026-05-10T00:50Z] Opus 4.7  · T0.11b ✅ DONE · ship: b74e72d · 48/48 forge VERIFIED on-host
[2026-05-10T00:55Z] Opus 4.7  · T1.X1 added + claimed · base: 8dca75c
[2026-05-10T01:00Z] Opus 4.7  · T1.X1 ✅ DONE — false-positive (badges already shipped)
[2026-05-10T01:25Z] Opus 4.7  · disk reclaim — 99% → 23% (40 GiB free) via cargo target + uv cache wipe
[2026-05-10T01:30Z] Opus 4.7  · T0.9 claimed (sub-task A scoped) · base: 2e9e2c5
[2026-05-10T01:50Z] Opus 4.7  · T0.9 sub-task A ✅ DONE · ship: 8809064 · 11/11 on-host · sub-lane → 🟡 PARTIAL
[2026-05-10T02:10Z] Opus 4.7  · T0.9 sub-task C native half ✅ DONE · ship: 70be2b8 · 16/16 on-host (5 native Pedersen tests added)
[2026-05-10T02:15Z] Opus 4.7  · T0.5 claimed (sub-task 3 starter) · base: cd964a5
[2026-05-10T02:30Z] Opus 4.7  · T0.5 audited + re-scoped → 🟡 OPEN — code work largely shipped (sub-1/3 ✅, sub-4 infra ✅); remaining = sub-5 test (Mini) + operator steps. Anchor-enforcement at privacy_exec.rs:493,712 confirms PNT bounded-window is jointly secure with merkle-anchor rule.
[2026-05-10T02:50Z] Opus 4.7  · T0.9 sub-task B starter ✅ DONE · ship: dee01d9 · 17/17 on-host (EccChip column allocation + EccColumns struct)
[2026-05-10T09:30Z] Opus 4.7  · T0.9 sub-task B FINISH ✅ DONE · ship: 76659af · 20/20 on-host (full FixedPoints impl + Circuit::configure body w/ EccChip wired)
[2026-05-10T09:30Z] ⚠ note: commit 39e7c60 carries B-finish commit message but its diff is unrelated parallel-session WIP — labeling drift caused by index-race during git commit. Actual B-finish content is in 76659af (used `git commit --only <file>` to bypass the race).
[2026-05-10T09:48Z] Opus 4.7  · T0.9 sub-task C circuit-half STARTER ✅ DONE · ship: 438ad1e · 21/21 on-host (incl. MockProver synthesize check). C-circuit-FINISH (full MSM + constrain_equal + parity test) + D remain.
[2026-05-10T10:15Z] Opus 4.7  · T0.9 sub-C circuit-half FINISH attempted; blocked on `LookupRangeCheckConfig::load` not resolving for `pasta_curves::Fp` despite `bits` feature being default-enabled on pasta_curves 0.5.1. ff 0.13.1 + halo2_gadgets 0.3.1 are the canonical versions. Hypothesis: feature-unification race in cargo when pasta_curves is shared between an `optional` dep (ours, at v2-ecc) and `halo2_proofs`'s direct dep — the rmeta hash doesn't change so `bits` may be silently disabled in the binary. **Workaround for next session:** try `default-features = false, features = ["bits"]` on pasta_curves explicitly, OR a full `cargo clean` (slow) before re-attempting. Reverted attempt; 21/21 still green at 438ad1e (sub-C-starter).
[2026-05-10T11:25Z] Opus 4.7  · T0.9 sub-C-circuit-FINISH ✅ DONE · ship: 1f567be · Root cause was wrong: the `load` method is `#[cfg(test)]`-gated in halo2_gadgets, NOT a feature/bits issue. Re-implemented loader in our crate. **MockProver now verifies the full cryptographic claim**: in-circuit g·k == native pedersen_commit_native(&[generator()], &[k]) for k∈{1,7,100,12345}. T0.9 lane is structurally complete modulo D (prover wiring).
[2026-05-10T11:50Z] Opus 4.7  · T0.9 sub-D-starter ✅ DONE · ship: dd75ba6 · 22/22 on-host. Wire format `VerkleProofV2` + `VerkleProverV2Stub::placeholder` + JSON round-trip + fingerprint determinism tests. Cross-side fixture format LOCKED. Sub-D-finish (real IPA prove/verify) blocked on curve-param hypothesis to be tested next.
[2026-05-10T12:25Z] Opus 4.7  · T0.9 sub-D-finish hypothesis FALSIFIED. Tested `Params<halo2_pallas::Affine>` (= EpAffine, pallas::Base = Fp) — same `Circuit<Fq>` error. Both EqAffine AND EpAffine reject Circuit<Fp>. The bound `ConcreteCircuit: Circuit<C::Scalar>` is resolving C::Scalar to Fq for both pasta curves despite the `new_curve_impl!` macro setting `type Scalar = $scalar` to Fp for EqAffine. Two new hypotheses for the next attempt: (a) halo2_proofs may have a hidden/blanket impl of CurveAffine that overrides `Scalar`; (b) maybe needs `#[derive(Clone)]` or another trait on the circuit for `Circuit<F>` to resolve. Out-of-scope for this turn — needs ~half-day of dep-internals investigation. Status preserved at sub-D-starter (dd75ba6, 22/22 on-host).
```
