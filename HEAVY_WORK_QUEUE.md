# Heavy work queue — 2026-05-16 (post-audit-cycle)

The entire 2026-05-15 + 2026-05-16 audit cycle is closed in code (PRs #337–#345 awaiting merge).  Every code-actionable finding from rounds 1–7 has a fix landed or queued.  This file enumerates the five candidate heavy workstreams that come next, each in a different mental space so two or three can run in parallel without context-collision.

---

## Status legend

- 🟢 ready — kick off now, no deps
- 🟡 ready w/ setup — kick off after ~30-60 min ops setup
- 🔴 blocked — depends on operator decision or unfinished prerequisite

---

## 1.  Cluster soak — T0.2 + T0.5 + T0.6   🟡

**Surface:** OPS + EXECUTION + CONSENSUS · `MAINNET_READINESS.md` Tier-0 lanes

**Scope (already-built, just needs to run on a live cluster):**
- T0.2 D-track adversarial + perf + 72hr soak — `scripts/d-track-*.sh` ready (`adversarial`, `fault-injection`, `partition`, `soak`).
- T0.5 PNT v1 governance flip 0→1 at fork-epoch + storage-growth telemetry — code-complete; needs the operator step on a live cluster.
- T0.6 Slashing-at-scale empirical — 5 adversarial scenarios in `crates/evaporchain-consensus/tests/slashing_at_scale.rs` green on Mini 1 release (2026-05-14); cluster-side soak ready.

**Wall time:** 24–72 hours (mostly background).  Active work ~2 hours for kickoff + monitoring.

**Mainnet leverage:** highest of the five.  These are the last 🟡 OPEN lanes in `MAINNET_READINESS.md` Tier-0; flipping them to ✅ DONE is the gate before external-audit kickoff (T0.12).

**Dependencies:** T3.1 cluster bring-up status.  The 3 Minis are accessible; the 2 Hetzners need the operator's SSH-auth step (see `docs/runbooks/cluster-deploy.md` §3).  Mini-only soak is sufficient for D-track validation; the Hetzner pair is for the 5-node BFT minimum and only matters for the final 72hr soak.

**Why parallel-friendly:** runs in background; the foreground brain stays free.

---

## 2.  Greenfield substrate primitive  🟢

**Surface:** substrate (new crate, no chain-core conflict)

**Scope (pick one):**
- Energy-decay credentialing — a credential whose validity decays with the holder's on-chain energy (issued by a Bell-Beacon-authenticated issuer; revoked by energy-floor breach).
- Decay-bound auction primitive — extends `sealed_bid_auction.es` reference contract into a substrate crate; reserve-price + decay-deadline + commit-reveal binding.
- Skill-bounty primitive on top of SHLM — extends the existing skill half-life market with a bounty-pool / settlement primitive.

**Wall time:** 1-2 weeks for a useful first cut.  Single-iteration scaffolding: ~2 hours (crate skeleton + state machine + 15 unit tests).

**Mainnet leverage:** medium.  Doesn't unblock mainnet, but produces the "what the chain is for" demo that the application-universe doc has been promising.

**Dependencies:** none — purely additive substrate.

**Why parallel-friendly:** doesn't touch any existing crate; reviewer surface is the new crate only.

---

## 3.  Leaderless block production  🟢 *(but deep)*

**Surface:** consensus core · doctrine punch list Layer 6 ⏳ items

**Scope:**
- Block proposers emit parent sets without leader rotation (post-V1 doctrine).
- Sorkin BD-action / interval-cardinality invariant enforced at insert.
- Network-level causal delivery.

**Wall time:** 2-4 weeks.  Multi-phase: BFT theorem → validator-determinism gate → anti-censorship analysis → impl → adversarial test harness.

**Mainnet leverage:** zero for V1 (doctrine V1 ships with leader-rotation Tendermint; leaderless is V1.5+).  High for long-term doctrine purity.

**Dependencies:** the chain's current Tendermint hot-path stays in place (this is a parallel `ConsensusEngine` trait impl behind `--cfg doctrine_v1.5`).

**Why parallel-friendly:** behind a cfg flag; doesn't touch the production hot path.

---

## 4.  Coverage push 87% → 95%  🟢

**Surface:** wide (test additions per crate)

**Scope:**
- T1.20 reached 87.22% line / 88.54% function workspace-wide; the original ≥90% goal stalled at the substrate-crate tail.
- Iteration shape: `cargo llvm-cov --workspace --summary-only` → identify lowest-cov crate → add ~10 targeted tests → repeat.
- Scan is **running in background** on Mini 1 as of this commit (job `bcrwe1a9o`); result feeds into the next-loop decision.

**Wall time:** 1-2 weeks sustained.  Boring but high-yield — every test you add increases the audit-canary's effective coverage in a way the script can't grep for.

**Mainnet leverage:** medium.  External auditor (T0.12) will ask about coverage; getting to 95% is the easiest way to deflect that line of questioning.

**Dependencies:** none.

**Why parallel-friendly:** per-crate work, no cross-crate dependency.  One coverage gap closed at a time.

---

## 5.  Cross-project switch — ZovoNotes Phase 2  🟢

**Surface:** zero EvaporChain · different repo entirely · `~/Documents/zovonotes-backend/DIARIZATION_UPGRADE_PLAN.md`

**Scope:**
- 6-phase diarization upgrade plan; Phase 1 running, Phase 2 ready.
- NHS GP scribe — paying-pilot-bound; Innovate UK grant submitted (2026-04-29 application 10201984, decision by 2026-06-03).
- Beats every published diarization model in the relevant benchmark per `project_zovonotes_asr_roadmap.md`.

**Wall time:** 2 weeks per phase.

**Mainnet leverage:** zero (different project).  Active-Maintain tier per `~/CLAUDE.md`.

**Dependencies:** none — totally separate repo + stack (Python + PyTorch, no Rust).

**Why parallel-friendly:** true parallelism.  Zero context-collision with EvaporChain.  Use it as the third brain space when both #1 + #2 are active.

---

## Recommended kick-off combo

**`#1` (cluster soak, background) + `#2` (greenfield substrate, foreground) + `#5` (cross-project, async / weekend brain)**

- `#1` unblocks Tier-0 OPS lanes in `MAINNET_READINESS.md` with minimal active work.
- `#2` produces visible code shipping in the next 1-2 weeks while `#1` runs.
- `#5` gives a totally separate brain space for evenings / when chain-side is blocked on something.

Skip `#3` until the chain has paying users (it's a V1.5 doctrine cleanup, not a V1 blocker).  Skip `#4` unless the external auditor explicitly asks for ≥95% — the marginal value below 95% is too low.

---

## Audit-cycle close ledger

Open audit PRs awaiting merge (no further code action needed from me):
- #337 — R-series + DRIFT-N3 regression replay
- #338 — CONS-A1 + CONS-B1 + PARSER-1 defensive
- #339 — audit-canaries.sh + CI gate + pre-commit hook
- #340 — M10 BatchUndoLog
- #341 — M9 delete_account
- #342 — verkle verify-side DST + Energy-Verkle alias test
- #343 — ghost_bridge unsafe-no-attestation API
- #344 — DEPLOY-1 mktemp + chmod 600
- #345 — GEN-N1 KeyAnnounce continuity signature

Once these 9 merge, the canary script returns 32/32 green and the regression class is non-bypassable at CI.
