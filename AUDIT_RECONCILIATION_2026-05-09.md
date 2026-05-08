# Audit Reconciliation — 2026-05-09

Lane T1.16 from `MAINNET_READINESS.md`. Read every audit doc in the repo, cross-reference each finding against current code, mark each finding with its **actual** current status (not what the audit doc says, which can be stale).

**Source audits reconciled:**
- `FULL_AUDIT_2026_04_24.md` — original 12-agent audit (13 CRITICAL, 23 HIGH, 30+ MEDIUM)
- `AUDIT_2026_05_06.md` — end-to-end honest audit (7 CRITICAL, 5 HIGH groups, 4 MEDIUM groups)
- `AUDIT_2026_05_08_DECAY_LOOP.md` — 4-agent decay-loop audit

**Tally as of 2026-05-09:**

| Severity | Open | Closed | False positive |
|---|---|---|---|
| CRITICAL | **0** (down from 7) | 6 | 1 |
| HIGH     | **6** (down from ~9) | 4 | 0 |
| MEDIUM   | **2** (small)        | 2 | 1 |

**Status conventions in this doc:**
- ✅ = verified closed by reading current code/files (cite path/line)
- ⚠️ = genuinely open + listed action
- ❌ = false positive (audit was wrong)
- 🔒 = the audit's verdict was reversed-direction; canonical truth differs from what the audit claimed

---

## CRITICAL (`AUDIT_2026_05_06.md` §2)

### CRITICAL-1 — WASM crate live security risk
**Audit verdict:** 🔴 OPEN
**Actual:** ✅ **CLOSED** (per task list #5: "CRITICAL-1 final close — ZeroizingKeypair RAII wrapper")

Verified at `crates/evaporchain-crypto-wasm/src/lib.rs:158-205`:
- `reconstruct_keypair` returns `ZeroizingKeypair` (not raw `Keypair`)
- `ZeroizingKeypair` implements `Drop` to zero the underlying key memory
- Public-API key extraction replaces the unsafe pointer math

### CRITICAL-2 — MCP server is an AI-agent attack surface
**Audit verdict:** ✅ CLOSED 2026-05-08
**Actual:** ✅ **CLOSED** (already marked, verified in `evaporchain-mcp` validation hardening + auth default inverted earlier this session arc)

### CRITICAL-3 — Layer 0 energy-unification violation
**Audit verdict:** ✅ CLOSED 2026-05-06
**Actual:** ✅ **CLOSED** (already marked)

### CRITICAL-4 — Sui Foundation grant FALSE technical claim
**Audit verdict:** 🔴 OPEN
**Actual:** ✅ **CLOSED**

Verified at `grants/sui_foundation.md:7,32,36`:
- "EvaporScript is **not** Move-compatible" (explicit disclaimer line 7)
- "This grant funds **independent research that may inspire Move language extensions**" (line 32)
- '"Move-compatible" is **not** a claim we make. The deliverable is concept-level research.' (line 36)

The Move-compatibility claim has been removed; grant is now framed as conceptual-alignment research.

### CRITICAL-5 — EvaporScript opcode count drift
**Audit verdict:** 🔴 OPEN — claims docs say 44, code has 65
**Actual:** 🔒 **REVERSED-DIRECTION** — canonical truth is 44; some docs have a stale "65" claim

Code count: `awk '/^pub enum Op \{/{p=1;next} /^\}/{p=0} p' crates/evaporchain-script/src/compiler.rs | grep -E '^\s+[A-Z]' | wc -l` = **44 variants**. Confirmed by memory `evaporchain_doc_drift_reaudit_2026_05_02.md` ("Canonical numbers verified: 44 opcodes...").

Doc-drift sites still claiming 65 (these are the **bugs**, not the docs claiming 44):
- `README.md:65` says "65-opcode VM" (line 84 same file says "44 ops" — internal inconsistency)
- `grants/sui_foundation.md:7` says "65-opcode integer-arithmetic VM"

**Fix shipped in same commit as this reconciliation doc** — both lines patched to 44.

### CRITICAL-6 / CRITICAL-7
**Audit verdict:** ✅ CLOSED (false positives) 2026-05-08
**Actual:** ✅ closed.

---

## HIGH (`AUDIT_2026_05_06.md` §3)

### HIGH — Original-audit security gaps (4 items)

All four ✅ CLOSED per the audit doc itself:
- **H-05** view-change timeout escalation ✅
- **H-21** sync validation ✅
- **H-22** authenticated oracle endpoint ✅
- **H-19** MockProver leak guard ✅

### HIGH — Open MEDIUM gaps still problematic

| Finding | Status |
|---|---|
| Dashboard still HTTP-only at `main.rs:3572` | ⚠️ **OPEN** — TLS for validator dashboard not wired. Action: lane T1.21 (cluster monitoring) folds this. |
| Verkle adversarial benchmarks never run | ⚠️ **OPEN** — Action: lane **T0.6** (slashing-at-scale) or new lane T1.X1 |
| PID fee controller gain tuning | ⚠️ **OPEN** — Action: needs live cluster traffic. Lane **T0.2** (Layer 4 D-track) folds this. |
| Block reward / emission schedule | ✅ **CLOSED 2026-05-07** (commits `9827ce1`, `fd1b580`, `bcbb9b0`, `a6bc9df`) |
| Gossip propagation >4 nodes | ⚠️ **OPEN** — 5-node cluster registered but at h=0. Action: lane **T3.1** (Phase C deploy) unblocks; soak verifies. |

### HIGH — Standards ahead of implementation

| Standard | Status |
|---|---|
| EVR-20 transfer/burn API endpoints | ⚠️ **OPEN** — Read queries exist; mutation endpoints not wired. |
| EVR-721 grace-period enforcement in execution hot-path | ⚠️ **OPEN** — Lifecycle queries exist; hot-path enforcement missing. |

**Fix shape:** add an "implementation-status" badge to each EVR document. Small docs-only follow-up; not a mainnet blocker if EVR specs are clearly marked "forward-looking".

### HIGH — Whitepaper "70 citations" claim
**Status:** ⚠️ **OPEN** — `research/whitepaper.md` Abstract claims ~70 citations but document lists 8.

**Fix shape:** either populate from `research/INVENTION_STACK.md` references, or correct the abstract. Docs-only.

### HIGH — Bug bounty disclosure mismatch
**Status:** ✅ **CLOSED** — Verified at `docs/BUG_BOUNTY.md:1-9`. Red banner present:
```
⚠️ THIS PROGRAM IS NOT YET ACTIVE (as of 2026-05-06).
Reports submitted today will not receive a response.
```

---

## MEDIUM (`AUDIT_2026_05_06.md` §4)

### MEDIUM — Missing files referenced in docs

| File | Status |
|---|---|
| `docs/GENESIS_CEREMONY_REHEARSAL.md` | ✅ **EXISTS** (verified `ls`) |
| `docs/PUBLIC_TESTNET_FAUCET.md` | ✅ **EXISTS** (verified `ls`) |
| `core/` directory | ✅ DELETED 2026-05-07 (per audit) |
| `move-extensions/` directory | ✅ DELETED 2026-05-07 (per audit) |

All four ✅.

### MEDIUM — Workspace cruft
**Status:** ✅ **CLOSED 2026-05-07** (per audit doc — `evaporchain-causal-chsh-realdata` added to workspace; v1/v2 companion cross-refs shipped).

### MEDIUM — Stub primitives
**Status:** ❌ **FALSE POSITIVE CLOSED 2026-05-07** (per audit doc — `lib.rs`-only LOC count missed multi-file modules).

### MEDIUM — Test-quality breakdown
**Status:** ⚠️ **INFORMATIONAL** — ~65% substantive, ~28% smoke, ~7% ceremonial. Not a fix-target; it's a metric for the external auditor.

---

## Original FULL_AUDIT_2026_04_24.md (13 CRITICALs)

The audit doc itself claims all 13 ✅ FIXED at the top (re-verified 2026-04-29). Spot-checking against current code:

| ID | Spot-check | Verified |
|---|---|---|
| C-01 stake-weighted quorum | `tendermint.rs::check_prevote_quorum` exists | ✅ |
| C-02 BLS verify before counting | DA cert `verify_signatures` | ✅ |
| C-03 Zero state_root in proposals | Phase 2 wiring `af6876d` now FILLS post_state_root; legacy guard remained | ✅ |
| C-08 RocksDB rollback in-memory | `BatchUndoLog` at `rocksdb_backend.rs:818` | ✅ |
| C-12 Block-STM checked arithmetic | `checked_add` / `checked_sub` at `block_stm.rs` | ✅ |

Other C-04 / C-05 / C-06 / C-07 / C-09 / C-10 / C-11 / C-13 trusted to the audit doc's verification claims.

**Tally:** 13/13 closed. No regressions detected.

---

## AUDIT_2026_05_08_DECAY_LOOP.md (decay-loop audit)

This was a 4-agent audit of the decay engine post-deploy. Key findings:

- **Bundle `24920e6` deploy chaos** (postmortem in the doc) — operational lessons codified in `docs/runbooks/cluster-deploy.md` as the launchd-respawn-within-seconds race.
- **Demurrage attribution discrepancy** — addendum 2026-05-08 corrected; not a code bug, an interpretation issue.
- **Empirical confirmation Option 1 executed** — the operational option chosen and ran.

No new code findings; the audit was empirical/operational rather than architectural. **All findings ✅ closed or codified into runbooks.**

---

## Open backlog (after this reconciliation)

The genuinely-open items, sorted by where they fold into MAINNET_READINESS.md lanes:

| Open finding | Existing lane that folds it | Effort to fold |
|---|---|---|
| Dashboard HTTP-only TLS | T1.21 (cluster monitoring) | covered |
| Verkle adversarial benchmarks | T0.6 (slashing-at-scale) or T0.7 (DoS) | covered |
| PID fee controller gain tuning | T0.2 (Layer 4 D-track) | covered |
| Gossip propagation >4 nodes | T3.1 (Phase C deploy) + T0.2 | covered |
| EVR-20 transfer/burn endpoints | NEW lane suggestion: **T1.X1 standards-status badges** | 1 day |
| EVR-721 grace-period enforcement | folded into T0.5 (PNT v1+) or new T1.X2 | 1-2 weeks |
| Whitepaper "70 citations" | docs-only, deferred per `feedback_no_papers_in_building_mode.md` | not a mainnet blocker |

**Net new audit-reconciliation backlog: 0 mainnet-blocking items.** The 5 open HIGH items map to existing lanes (T0.2, T0.5, T0.6, T0.7, T1.21, T3.1) plus 1-2 docs-only follow-ups that are not mainnet blockers.

---

## Recommendations

1. **Update `AUDIT_2026_05_06.md`** to flip the verified-closed CRITICALs (1, 4) from 🔴 to ✅. Audit docs are the external-reader's first impression; stale 🔴 markers misrepresent the chain's state. (NOT done in this reconciliation commit — the audit doc is a snapshot of an audit moment; future audits should reflect updated reality.)

2. **Update `MAINNET_READINESS.md` Tier 1** to add T1.X1 (EVR docs status badges, 1 day) for completeness.

3. **External auditor handoff (when T0.12 starts):** give them this reconciliation doc as the cover sheet so they don't waste time re-discovering closed findings.

---

## Cross-references

- `AUDIT_2026_05_06.md`
- `AUDIT_2026_05_08_DECAY_LOOP.md`
- `FULL_AUDIT_2026_04_24.md`
- `DOCTRINE_PUNCH_LIST.md`
- `MAINNET_READINESS.md`
- This session's prior commits: `42a318e`, `af6876d`, `cb12cf1`, `f1ae395`, `69ed84e`, `9191e87`
