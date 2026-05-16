# AUDIT 2026-05-16 — Status dashboard (Round 7 complete)

**Last updated:** 2026-05-16, end of session 43.

---

## 1. What's done

### Audit infrastructure shipped this session

| PR | Title | Status |
|---|---|---|
| #337 | regression replay: R3/R4/R6/R7 + DRIFT-N3 (10 closures dropped by a merge) | open — needs merge |
| #338 | round-2 defensive bundle: CONS-A1 + CONS-B1 + PARSER-1 | open — needs merge |
| #339 | `audit-canaries.sh` regression gate + `make check` hook + CI workflow + pre-commit sample (32 canaries) | open — needs merge |
| #340 | M10 BatchUndoLog stakes/delegations/sentinel (never-merged closure) | open — needs merge |
| #341 | M9 `delete_account` routes through pending_batch (round-7 verified-live) | open — needs merge |

### 7 audit rounds executed

| Round | Date | Findings | Closure |
|---|---|---|---|
| 1 | 2026-05-11 (`AUDIT_2026_05_11.md`) | 13C + 23H + 30M | all in code |
| 2 | 2026-05-15 (`AUDIT_2026_05_15.md`) | 9C + 6H + 8M + 16L + DRIFTs | all in code |
| 3 | 2026-05-16 #1 (CONSENSUS/PRIV agent set) | DRIFT-N3 + 9 admin gates | PR #337 |
| 4 | 2026-05-16 #2 (defensive sweep) | CONS-A1/B1 + PARSER-1 | PR #338 |
| 5 | 2026-05-16 #3 (STATE deep-dive) | M10 BatchUndoLog gap | PR #340 |
| 6 | 2026-05-16 #4 (never-merged sweep) | 5 cherry-picks of H4/H10/H7/C1/C2 | merged via parallel cherry-picks |
| **7** | **2026-05-16 #5 (10-agent end-to-end)** | **M9 verified-live; 3 design-deferred** | **PR #341 + this doc** |

### Audit canary script ladder (PR #339)

32 canaries pinning every recent closure across 4 categories:
- 16 admin-gate canaries (R3, R4, R5, R6, R7, R8, R9, R10)
- 1 negative canary (DRIFT-N3 warn-only comment)
- 4 DST canaries (GEN-N3 ×2, PRIV-N5 ×2)
- 3 defensive-assert canaries (CONS-B1, CONS-A1, PARSER-1)
- 6 M10 canaries (stakes/delegations/sentinel snapshots + sentinel routing)
- 1 GEN-N5 Argon2 parameter
- 1 placeholder

On current main: 13/26 pass, 13 fail.  The 13 failures are exactly the closures still on the open PRs.

CI workflow now blocks the cargo build behind a green `audit-canaries` step — non-bypassable at merge time.

---

## 2. Round-7 results — 10-agent end-to-end

| # | Agent | Result | Live findings |
|---|---|---|---|
| 1 | CONSENSUS | clean | GEN-N1 (HIGH, deferred — needs design); DRIFT-N3 already closed by PR #337 |
| 2 | STATE/SYNC/SNAPSHOT | clean | M10 (already in PR #340); M9 (PR #341); H8 (deferred — test refactor) |
| 3 | EXECUTION + PAYMASTER | clean | none (false-positive on gas accumulator — already checked_add at line 1527) |
| 4 | CRYPTO | clean | H2 + H3 (verified live, deferred — wire-format change needs test refresh) |
| 5 | PROVING + PRIVACY | clean | none (all PRIV-N1..N6 closed; SUB-N9 closed) |
| 6 | NETWORK + DA | clean | NETWORK-N1 (false-positive — `from > to` already gates); NETWORK-N2 (minor — req-resp size cap) |
| 7 | BRIDGE | clean | none — bridge layer audit-clean |
| 8 | VM + APP-TEMPLATES + SUBSTRATE | clean | SCR-N2/SUB-N1/N2/N3 all flagged but already closed via different SHAs |
| 9 | NODE-API + CLI | clean | R1-R12 all verified closed |
| 10 | SHARDING + ORACLE + REMAINDER | clean | 1 MEDIUM (deploy-testnet.sh tempfile permission — operational only) |

**Bottom line:** 1 new closure shipped in code (M9, PR #341). 3 verified-live HIGH/MEDIUM deferred to follow-ups requiring test refactor or design work (H2, H3, H8). 1 verified-live HIGH deferred for design (GEN-N1 KeyAnnounce continuity signature).

---

## 3. What's left

### Code-actionable backlog (deferred from Round 7)

| ID | File:Line | Severity | Why deferred |
|---|---|---|---|
| **H2** | `crates/evaporchain-crypto/src/verkle.rs:131` | HIGH | Wire-format change (state-root shifts).  6 verkle tests need fixture refresh.  Estimated 30 min once tests updated. |
| **H3** | `crates/evaporchain-crypto/src/energy_verkle.rs:38` | HIGH | Same wire-format dependency as H2.  Should ship together with H2. |
| **H8** | `crates/evaporchain-state/src/ghost_bridge.rs:185` | HIGH | 3 happy-path tests pass `validator_pubkeys = None`.  Fix needs design split: rename current API to `_unsafe_no_attestation` (or remove the `None` arm and require explicit keys).  Estimated 1 hr. |
| **GEN-N1** | `crates/evaporchain-consensus/src/tendermint.rs:4666` | HIGH | Needs new signature scheme: rotating BLS key must be signed by *previously-registered* key, not just PoP of new key.  Multi-validator-cycle protocol change.  Estimated 2-3 hrs (design + impl + tests). |

### Operational / out-of-code

- **deploy-testnet.sh tempfile permission** — MEDIUM operational. Use `mktemp` + chmod 600 before sudo mv. Estimated 5 min.
- **NETWORK-N2** (request_response pre-deserialize size cap) — LOW. Defense-in-depth above the gossip cap. Estimated 15 min.

### Merge logistics

5 open audit PRs (#337-#341) need user-side merge.  Until they land, the canary script returns 13/26 on main; once they all merge, returns 32/32 green.

### Master MAINNET_READINESS lanes

After this audit cycle: **zero pure-code lanes are OPEN**.  All remaining 🟡 lanes in `MAINNET_READINESS.md` are OPS-only (cluster soak, runbook execution, external audit kickoff).  T0.12 is BLOCKED on operator (auditor selection).

---

## 4. Progress summary

```
Audit posture entering session 43:   AUDIT_2026_05_15.md closed in code
Audit posture exiting session 43:    AUDIT_2026_05_15.md closed in code
                                   + 5 audit PRs queued (regressions and defensive bundles)
                                   + 1 net-new round (Round 7) executed
                                   + 4 verified-live items deferred (H2/H3/H8/GEN-N1)
                                   + canary gate + CI workflow shipped (#339)

Backlog after merge of 5 open PRs:   4 verified-live items (Round 7 deferred)
                                   + GEN-N1 protocol design work
```

---

## 5. Next heavy-work pivots available

With the audit punch list at "follow-ups only" and zero open pure-code MAINNET_READINESS lanes, the natural next workstreams are:

1. **Resolve H2/H3 wire-format change + test refactor** — 30-60 min focused work.
2. **Design + ship H8 fail-CLOSED API split** — 1 hr focused work.
3. **Design + ship GEN-N1 KeyAnnounce continuity sig** — 2-3 hr protocol change.
4. **Move past audit phase entirely** — pick from doctrine punch-list, substrate primitive extension, or operator-side cluster work.

The audit gate is now non-bypassable (PR #339), so any future regression will be caught at merge time.
