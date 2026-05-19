# EvaporChain Mainnet Sprint — Consolidated Plan (2026-05-11)

End-to-end audit + sprint plan produced 2026-05-11 evening. Replaces the
ad-hoc "what's next" thread that lived across `MAINNET_READINESS.md`,
`docs/MAINNET_PUNCHLIST.md`, `DOCTRINE_PUNCH_LIST.md`, and
`REMAINING_WORK.md`. After this lands, the cluttering predecessors are
archived (see §3 below).

The sprint window is **May–Oct 2026** (6 months, £100K budget per
`project_evaporchain_2026_push.md`). All other personal projects are
maintenance-mode for the duration.

---

## 1. Useless files / dead weight

### 1.1 Doc clutter to archive (11 files)

These docs are either superseded by current authoritative ones,
explicitly marked DEPRECATED, or describe completed work that has
already shipped. Moving them under `docs/archive/` keeps the history
without forcing every new session to read them.

**`docs/archive/obsolete-audits/`** — superseded audit snapshots:
- `FULL_AUDIT_2026_04_24.md` (24KB) — 13 CRITICAL + 23 HIGH + 30 MEDIUM; all closed in `AUDIT_2026_05_06.md`
- `AUDIT_2026_05_06.md` (62KB) — comprehensive end-to-end; superseded by `AUDIT_2026_05_11.md`
- `AUDIT_2026_05_08_DECAY_LOOP.md` (17KB) — specialist interrogation; folded into May-11
- `AUDIT_RECONCILIATION_2026-05-09.md` (10KB) — meta-summary of older audits
- `CHAIN_FINDINGS_2026_05_08.md` (13KB) — wallet-driver empirical snapshot

**`docs/archive/completed-plans/`** — plan docs whose ✅/✅/✅ checklists are done:
- `CROOKS_MEV_INTEGRATION_PLAN.md` (33KB, 35/35 ✅ — last commit `36cda88` 2026-05-07)
- `LAMBDA_FOLD_NOVA_PLAN.md` (27KB, 36/37 ✅ — `df3bb34` 2026-05-07)
- `LIGHT_CONE_FULL_DAG_PLAN.md` (32KB, 39/39 ✅ — `8dc05bd` 2026-05-07)
- `MCC_FULL_MULTI_PARENT_PLAN.md` (54KB, 28/28 ✅ — `fd5a3b8` 2026-05-08)

**`docs/archive/deprecated/`** — explicitly marked DEPRECATED in their own headers:
- `REMAINING_WORK.md` (14KB) — header reads "DEPRECATED 2026-05-07 — DO NOT USE AS A LIVE PUNCH LIST"
- `docs/MAINNET_PUNCHLIST.md` (21KB) — dated 2026-04-27, role taken over by `MAINNET_READINESS.md`

**Result:** root .md count drops 22 → 11. A new session reads 5 docs to be productive (§3).

### 1.2 Crate clutter — 25 candidate crates for `research/dead-weight/` or feature-gating

Reverse-dep audit of the 155 workspace members (18 core + ~137 substrate)
shows 25 crates with no consumers in the core hot path AND no
consumers in any other substrate crate. They're test harnesses,
scaffolds, or research-marked artefacts. Zero mainnet-correctness
risk to move them; ~0.3M LOC of `cargo check` noise removed.

| Tier | Crates | Why dead |
|---|---|---|
| 1A — Pure stubs (<30 lines) | `causal-chsh-cli`, `finality-attestation-cli`, `eth-bridge` (Phase 0 stub, replaced by `ethereum-bridge/` standalone workspace) | Scaffolds with `fn main() { unimplemented!() }`-grade content |
| 1B — Near-stubs (<50 lines) | `finality-attestation-renderer`, `causal-chsh-renderer`, `energy-coverage`, `compute-market`, `network-simplex-v2`, `scdi` | Test-only or empty modules |
| 1C — Research-marked | `ollivier-ricci`, `sgb`, `dfri` | Carry "research only" lib.rs markers |
| 2 — Isolated chains | `fold-debugger`, `scl`, `ssm`, `ra-did`, `sbav`, `epa-mmr-sumcheck`, `total-evaporscript-fuel`, `embd` | <100 lines, no upstream users |
| 3 — Test harnesses | `causal-chsh-sweep`, `causal-chsh-realdata`, `light-client-example-balance-monitor` | Example / one-shot empirical runners |

**Keep (explicitly NOT dead):**
- Doctrine primitives `total-evaporscript`, `cap-decay-vm`, `dp-native-vm`,
  `thermal-stm`, `epa-mmr`, `ew-twap`, `plc`, `memento` — real
  implementations of recent Tier-2/Tier-3 substrate; not yet wired
  to consensus but design is shipped.
- All 9 app-templates crates (`app-templates-{deploy,materialise,engine,bind,fees,receipt,eventlog}` + `app-glue` + `app-templates`) — dApp deploy pipeline, interconnected.
- `evaporchain-mera` — retained per `CLAUDE.md` as research artefact;
  empirical gate failed 2026-05-03 (R²=0.66 < 0.85), VERKLE verdict
  locked. Keep with a `#[deprecated]` marker post-launch.

**Procedure:** move per-crate (not bulk delete) so commits are reversible.

---

## 2. Information that messes up creativity

The audits surfaced four classes:

1. **Three docs that all claim "single source of truth" for what's left.**
   `MAINNET_READINESS.md` (current), `docs/MAINNET_PUNCHLIST.md`
   (2026-04-27), and `DOCTRINE_PUNCH_LIST.md` overlap on lane status.
   Resolution: keep `MAINNET_READINESS.md` as the lane board (open vs
   claimed vs done), keep `DOCTRINE_PUNCH_LIST.md` as the doctrine-layer
   completion tracker, archive `MAINNET_PUNCHLIST.md` outright.

2. **Five overlapping audit docs.** A reader has to load 100+ KB to know
   what the current open findings are. `AUDIT_2026_05_11.md` is the
   current state; the others become history.

3. **Four ✅-shipped plan docs still at the repo root.** They describe
   work that landed and now bias new sessions into thinking they need
   to ship something that's already shipped. Archive them.

4. **The two giant journal files** (`SESSION_PROGRESS.md` 139KB,
   `CHANGELOG.md` 128KB) are correctly authoritative and append-only,
   but only the top 1-2 entries actually matter for a new session.
   Leave both at root; document in CLAUDE.md "read top 1-2 entries of
   SESSION_PROGRESS only."

---

## 3. The canonical read-order

After §1 archival, a new session reads exactly these to be productive:

1. `SESSION_PROGRESS.md` — newest entry first; stop after 2-3 entries.
2. `MAINNET_READINESS.md` — pick a lane.
3. `DOCTRINE_PUNCH_LIST.md` — what's already done at layer level.
4. `AUDIT_2026_05_11.md` — current open findings.
5. `CHANGELOG.md` — grep, do not read top-to-bottom.

Plus the existing `CLAUDE.md` / `README.md` / `SECURITY.md` for
onboarding, and the active decision docs `POST_EXEC_STATE_VERIFICATION_PLAN.md` /
`ETHEREUM_BRIDGE_PLAN.md` / `IMPOSSIBLE_RESEARCH_STACK.md` for
in-flight research lanes.

I'll add a short "BEFORE STARTING WORK" header to `CLAUDE.md` pointing
at exactly this list once the archival is committed.

---

## 4. Mainnet sprint plan — what's actually left

Tier 2 lanes (sharding live, Encrypted Mempool full activation, MetaCoq
extraction, Energy-Verkle compression, LLSA Layer 7) are V1.5 — NOT
mainnet-blocking. Listed only so nobody picks one by accident.

### 4.1 Critical path (Tier 0 + Tier 1 + Tier 3 — mainnet-blocking)

**Verified ✅ DONE — keep as reference, do not re-plan:** T0.3 (POST_EXEC
Phase 4), T0.4 (block-hash inclusion), T0.11 (replay protection), T0.11b
(MembershipAttester gate), T1.14 (Phase 2 round-trip), T1.16 (audit
reconciliation), T1.X1 (EVR badges — audit-miss false positive).

**🟡 OPEN lanes I can drive now (parallel-safe):**

| Lane | Surface | Effort | Notes |
|---|---|---|---|
| T0.1 | CONSENSUS | 2-3 weeks | Layer 4 hot-path consensus surgery C.1–C.6. Highest-value remaining code work. |
| T0.5 (finish) | PRIVACY | 3-5 days | Only sub-task 5 (adversarial test) + 2 operator steps remain. |
| T0.6 | EXECUTION + STATE-DB | 1-2 weeks | Slashing-at-scale empirical tests. |
| T0.7 (finish) | NETWORK + EXECUTION | 1-2 weeks | 4 of 5 DoS vectors shipped; needs comprehensive suite + runbook. **AUDIT-2026-05-11-1/2 already landed today closes part of this.** |
| T0.8 (finish) | NETWORK | 1-2 weeks | Sub-tasks 2 & 4 + structural-validation shipped 2026-05-11; remaining edge cases. |
| T0.9 D-finish | BRIDGE-RUST | 3-5 days | Halo2 IPA prove/verify; blocked on `Params<C>` curve-param binding. Two hypothesis paths in lane spec. |
| T1.15 | PAYMASTER | 2-3 days | Per-key in-flight locking. Coordinate with parallel Sonnet session. |
| T1.20 | STATE-DB | 1-2 weeks | Coverage push (90% target). `execution::parallel.rs` 63.6% is the biggest hole. |

**🔴 BLOCKED on operator (chokepoints):**

| Blocker | Lanes gated | Action needed |
|---|---|---|
| Hetzner SSH auth (T3.1) | T0.2, T0.6, T1.17, T1.18, T1.19, T1.21, T1.22, T1.23, T3.2 | One operator message authorizing SSH to `root@evaporchain-hel-1` (100.66.208.20) and `root@evaporchain-hel-2` (100.91.235.22). |
| Auditor selection (T0.12) | T0.12 | Operator picks one of Trail of Bits / OtterSec / Spearbit / Halmos and signs the engagement. Recommend deciding by 2026-05-25 once T0.1–T0.10 are stable. |

**🔴 BLOCKED on prior lanes (cascade only):**
T0.2 → T0.1+T3.1 · T0.10 → T0.9 D-finish · T1.13 → T0.1.

### 4.2 Sprint sequence (assuming T3.1 SSH auth lands by ~2026-05-13)

**Week 1 — 2026-05-12 → 05-18 (code parallel):**
- T0.1 (Layer 4 consensus C.1–C.6) — primary
- T0.9 D-finish (Halo2 IPA hypothesis-test) — parallel
- T1.15 (paymaster locking) — parallel, coordinate w/ Sonnet
- T0.5 sub-task 5 adversarial test — Mini-only, parallel
- T3.1 cluster deploy + 24hr soak — operational, parallel

**Week 2 — 2026-05-19 → 05-25:**
- Ship T0.1
- T0.10 (VerkleProofVerifier.sol Groth16) — gated on T0.9 D-finish
- T0.7 / T0.8 finalize edge cases + DoS suite docs
- T1.20 coverage push on `execution::parallel.rs`

**Weeks 3-4 — 2026-05-26 → 06-07:**
- T0.2 Layer 4 D-track (72-hr cluster soak)
- T0.6 slashing-at-scale (live cluster)
- T1.13 conservation gating flip
- T0.12 audit kickoff

**Weeks 5+ — 2026-06-08 onward (V1 gate):**
- T3.2 5-node genesis switch
- T1.21/22/23 ops runbooks (monitoring, upgrade rehearsal, genesis dry-run)
- T0.12 audit remediation (parallel with V1.1 planning)

### 4.3 Audit-miss candidates to flip in `MAINNET_READINESS.md`

These lanes are marked 🟡 OPEN but the work is already shipped:

- **T1.X1** — false positive; EVR docs already have implementation-status badges. Flip to ✅ DONE.
- **T0.7 vector V5** (DAG fork-spam) — commit `0e976f4` (2026-05-11) lands it. Update lane to "4 of 5 vectors ✅, V5 ✅; comprehensive suite + docs remain."
- **T0.8 sub-task 4** (partial withhold) — commit `8abd388` (2026-05-11). Flip to ✅.
- **T0.8 structural-validation** — commit `dee358b` (2026-05-11). Flip to ✅.

### 4.4 What I'm NOT going to propose

Per `feedback_no_papers_in_building_mode.md` and `feedback_building_mode_2026_05_02.md`:
- No paper/whitepaper work as "what's next".
- No grant pursuit ("Innovate UK" was for ZovoNotes, already submitted).
- No external-audit prep deck rewrites until code is frozen.
- No Hetzner expansion suggestions (parked per `feedback_no_hetzner_until_conclusion.md`).

Per `feedback_satya1_domain.md`:
- SATYA-1 domain remains separate from `infonovasolutions.com`.

---

## 5. Concrete next actions (operator picks one)

The cluster reached lockstep 2026-05-11 evening. The code surface is
~95% mainnet-ready. The decision now is which lane gets driven first.

1. **Doc + crate hygiene pass** (≤1 hr; reversible). `git mv` the 11
   obsolete docs into `docs/archive/{obsolete-audits,completed-plans,deprecated}/`
   and run a single batch `cargo workspace member` removal for 25
   dead-weight crates → `research/dead-weight/`. Open small follow-up
   PRs to add the "read these 5 files" preamble to `CLAUDE.md`. Net
   effect: every future session boots into a cleaner picture.

2. **T0.1 Layer 4 consensus surgery (C.1–C.6).** Highest-value code
   work; can run on Mini 1 standalone, doesn't need T3.1 SSH.

3. **T0.9 D-finish hypothesis spike.** 3-5 day focused stretch on the
   Halo2 `Params<C>` curve-param binding; unblocks T0.10 which is the
   last bridge-side gate.

4. **T0.5 PNT finalize.** Smallest item; sub-task 5 adversarial test
   plus prep notes for the cluster operational steps. Can land in a
   day or two.

Recommend doing (1) first regardless — it permanently lowers the
cognitive overhead of every subsequent session. Then (2) or (3) on
deep-work days, (4) for shorter sessions.

---

## 6. Bookkeeping

- This file replaces ad-hoc planning that scattered across the 11
  archive candidates in §1.1.
- `MAINNET_READINESS.md` remains the lane-claim board. This file is
  the *narrative* of the sprint; `MAINNET_READINESS.md` is the
  *coordination protocol* a session uses to claim work.
- Append a `SESSION_PROGRESS.md` entry per session in the existing
  template — that requirement does not change.
- Next audit follow-up: re-spot-check the agent-reported audit-miss
  candidates (§4.3) before flipping `MAINNET_READINESS.md` status
  lines.
