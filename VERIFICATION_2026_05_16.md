# EvaporChain — Independent Mainnet-Readiness Verification (2026-05-16, late evening)

**Type:** Point-in-time verification report (like `AUDIT_*.md`; NOT the live journal).
**Why standalone:** `SESSION_PROGRESS.md` was under active concurrent-session writes during this audit; per the repo's cross-session rules this report is recorded here to avoid clobbering. A future session should fold a one-line pointer into `SESSION_PROGRESS.md` when that file is quiescent.

**Scope:** Operator-requested independent verification of "how close to mainnet" (distrust of an optimistic estimate). 4 parallel read-only streams: audit-finding reconciliation, readiness-vs-git drift, live build/test on Mini 1, code-reality spot-check. **No EvaporChain code changed; 0 commits.**

## Empirical results — Mini 1, HEAD `c5dbf2c1` (pulled clean)
- `make test-compile`: **PASS, 0 errors** (full workspace links; 39 non-fatal warnings)
- `cargo test -p evaporchain-consensus`: **1019 passed / 0 failed** (9 ignored)
- `cargo test -p evaporchain-execution`: **559 / 0**
- `cargo test -p evaporchain-state`: **258 / 0**
- `make lint`: **FAILS (exit 2)** — see new defect

## Verified facts
- **AUDIT_2026_05_15.md** (3 Crit / 9 High / 8 Med / 16 Low): all closed in `main`, **0 open Crit/High** (commit-mapped). Closure tracked in git/SESSION_PROGRESS, not the audit doc body.
- **High-risk DONE claims spot-checked in code: 5/6 VERIFIED**, none contradicted:
  - PNT anchor enforcement — `privacy_exec.rs:580` (unshield), `:825` (private_transfer); adversarial tests `:1847`, `:2507`
  - `conservation_enforcement` default = `"enforce"` — `tendermint.rs:1994`
  - H2/C1 DST-tagged node_hash — `verkle.rs:145`, `energy_verkle.rs:771/814`
  - C2 NMT zero-sibling reject — `namespace.rs:439`, test `:925`
  - GEN-N1 KeyAnnounce continuity sig — `tendermint.rs:185,4774,3895`
  - Coq: **zero bare `Admitted.`** across `research/coq` + `research/proofs`
- No hidden un-built V1 engineering in DOCTRINE_PUNCH_LIST / HEAVY_WORK_QUEUE; Layer-6 ⏳ items + decay-bound-auction are explicitly post-V1 / V1.5.

## 🔴 NEW DEFECT FOUND
- **`make lint` hard-fails**: `clippy::absurd_extreme_comparisons` at **`crates/evaporchain-consensus/src/tendermint.rs:5177`** —
  `let below_min = block.protocol_version < MIN_SUPPORTED_PROTOCOL_VERSION;` is **always false**
  → the minimum-protocol-version downgrade guard (`below_min` reject path) is **dead / unreachable code in the consensus version gate**. Introduced by recent rotation-continuity work. Tests unaffected; lint gate broken; genuine correctness smell. **Fix before treating the surface as frozen.**

## Doc-drift flags (reconcile later — not new engineering)
- `CLAUDE.md` governance table: `conservation_enforcement` default listed `"observe"`; code is `"enforce"` (`tendermint.rs:1994`). Doc stale.
- `DOCTRINE_PUNCH_LIST.md` Layer-4 still `[ ]` "promote conservation audit" — shipped `76d95590`. Stale checkbox.
- `MAINNET_READINESS.md` §8 status log last entry `2026-05-14`; ~147 commits since (bookkeeping drift, no hidden scope). **T3.1 self-contradicts**: index says 5-node live (PR#209); lane spec says 🔴 BLOCKED on Hetzner SSH. Real unknown — the 72-hr BFT soak depends on T3.1.
- "25,435 tests" headline unverifiable from source (static `#[test]` count ≈6,929; rest runtime/parameterised — aggregate, not a hard fact).
- SUB-N3 (HBCT): closed via reject-before-mutate, not the audit's literal `checked_add`; conservation defended but verify canary on a Mini.

## Honest recalibration
- **Build ≈ 95% — empirically verified** (compiles + core hot-path crates green on Mini today). Prior doubt "were tests actually run" → resolved: yes, core crates pass.
- **"Close to finished mainnet" RETRACTED.** Distance to trustworthy mainnet is gated by:
  1. the new lint/consensus dead-branch (`tendermint.rs:5177`)
  2. security fixes that landed within hours of HEAD today (unsoaked, wet ink)
  3. a 72-hr BFT soak that has **never run** (history shows soaks surface real bugs here)
  4. ambiguous T3.1 5-node cluster status
  5. **external audit not started** — per T0.12: 4–8 wk calendar + 2–3 wk remediation *assuming no Criticals*; the true critical path.
- Construction ≈ done; **independent inspection has not begun.** "Built but unsoaked and unaudited" ≠ "nearly finished."

## What's next (priority)
1. Fix `tendermint.rs:5177` dead `below_min` branch + restore lint gate (small, today).
2. Resolve T3.1 — is the 3-Mini + 2-Hetzner 5-node cluster actually live, or Mini-only?
3. Run the 72-hr soak (T0.2 `scripts/d-track-soak.sh`).
4. **Operator: select + contract the external auditor (T0.12)** — the actual mainnet gate.

**Cross-references:** `tendermint.rs:5177`, `tendermint.rs:1994`, `privacy_exec.rs:580,825`, `AUDIT_2026_05_15.md`, `MAINNET_READINESS.md` §4/§8. Verification HEAD `c5dbf2c1`. No commits this session.

---

# ADDENDUM — SFSV reference-dApp reconciliation (same session, UNVERIFIED)

Operator authorised (explicit, eyes-open) building the SFSV reference dApp in parallel with the mainnet sprint, accepting cluster-cycle / mainnet-slip risk. The following was done **in code, NOT compiled** (Minis-only rule; commit-at-end chosen). Recorded here so the eventual commit/verify is mechanical, not a reconstruction.

### Decisions locked
- **First reference dApp = SFSV** (not mortal-credentials; per `evaporchain_application_universe` queue + `strategic_decision_2026_05_16_focus`).
- **Predicate model = (a)**: `EnergyDecaysBelow{threshold}` is a pure comparison over engine-supplied live `contract_energy`; decay/refresh owned by the engine (restores invariant #1; refresh-aware). The original crate's frozen-formula predicate was the divergence Move-1 found; `.es` was always model (a).
- **§5 decision = A**: the crate now mirrors the `.es` on-chain listing state machine (`list_for_sale`/`cancel_listing`/`record_sale` + expiry guard), so the `.es`-header claim "guarded by SFSV crate adversarial tests" is actually backed.

### Files changed (9, all uncommitted, model-(a) + `.es`-faithful, internally grep-verified coherent)
| Crate | Files |
|---|---|
| `evaporchain-sfsv` | `predicate.rs` (model a), `payout.rs` (3-arg `payout(v,epoch,contract_energy)`), `lib.rs` (doc), `vault.rs` (listing state machine + `Listing` + guards + unit tests), `market.rs` (SDDC clear routed through `record_sale`; tests `&mut`), `tests/adversarial.rs` (parallel session's §8 suite reconciled to model (a) + refresh + §8.6 listing adversaries) |
| `evaporchain-app-templates-engine` | `init_sfsv.rs` (InitConfig → `.es` set_terms shape: future_self/predicate_type/release_param/deposit_amount — `EnergyDecaysBelow` vaults now deployable), `dispatch.rs` (test updated) |
| `evaporchain-app-templates-bind` | `bind.rs` (SFSV invariant arm → new shape) |

### Parallel-session interplay (multi-session repo)
- `contracts/evaporscript/future_self_vault.es`, `research/SFSV_ARCHITECTURE.md`, `scripts/deploy-sfsv.sh` were authored by a concurrent session. `deploy-sfsv.sh` and the architecture doc **converged** with this reconciliation (same `.es`-faithful `set_terms` shape / model (a)) — no conflict. `tests/adversarial.rs` **collided** (was old-shape) and was reconciled here, preserving all test names/§8 mapping/intent.
- `SFSV_ARCHITECTURE.md` §10.1 status table ("25 tests / green") is now stale vs the reshaped crate — will self-correct on Mini verify. Did not edit (contended parallel-session file).

### EXACT commit/verify checklist (mechanical, for when commit-at-end fires)
1. Read the parallel session's latest `tests/adversarial.rs` + `future_self_vault.es` + `deploy-sfsv.sh` (they may have advanced) — reconcile any new old-shape refs to model (a).
2. Stage the 9 files above (scoped — `git add` by path, not `-A`).
3. Commit (no `--no-verify`; respect hooks).
4. Push; on Mini 1: `git pull --ff-only` then
   `cargo test -p evaporchain-sfsv -p evaporchain-app-templates-engine -p evaporchain-app-templates-bind`
5. Expected risk areas if it fails: `market.rs` borrow (`&mut Vault` threading), `record_sale` expiry boundary (`epoch_now > opened_at+duration`), serde on new `Vault.listing` field, `init_sfsv` consumers beyond bind/dispatch.
6. Also re-confirm `tendermint.rs:5177` lint defect is unrelated/separate (it is).

**Status: SFSV build substantively complete; 100% UNVERIFIED until step 4 runs.**

---

## FINAL STATE after attempted Mini verification (same session)

**Mini run result (HEAD `14de46b6`, pull FAILED — stale):** `evaporchain-app-templates-bind` 19 pass / **1 FAIL** `every_catalogue_default_binds` → `invalid SFSV init JSON: missing field 'future_self'`. `evaporchain-sfsv` + `app-templates-engine` **not reached** (run aborted at first crate). So those two crates remain entirely unverified.

**Bug found + fixed (10th file):** `crates/evaporchain-app-templates/src/catalogue.rs:93` SFSV `default_params` was still old-shape. Fixed to:
`json!({"future_self": "0x00", "predicate_type": 0, "release_param": 10000, "deposit_amount": 1000})`
This is the catalogue regression-test fix; it is the only remaining content gap from the Mini failure. Fix is in the MacBook working tree + local HEAD; **NOT on `origin/main`** as of last check (origin line 93 still old-shape).

**Two ENVIRONMENTAL blockers prevent a trustworthy green (not code issues):**
1. **Multi-session shared working tree/index.** Concurrent Claude sessions continuously `git reset`/`add`/`commit`. Controlled scoped commits are impossible from one session (verified: scoped `git add` → empty staged set 3×). The reconciliation reached `origin/main` only by being *swept* into a parallel session's commit; the catalogue fix sits in local HEAD, push not yet propagated.
2. **Mini 1 git divergence.** Upstream tracking points at deleted ref `refs/heads/chore/coverage-neptune-sponge`; `git pull --ff-only` fails; Mini tests stale local HEAD. Has `stash@{0}: pre-final-cleanup-checkout`.

**Bounded remediation (operator / single coordinated session — NOT loop-retryable from a racing session):**
- Land the catalogue.rs one-liner on `origin/main` via a single atomic session commit (or confirm a parallel sweep carried it).
- Mini 1: `git fetch origin --prune && git merge --ff-only origin/main` (non-destructive). If non-ff, decide on `git reset --hard origin/main` (DESTRUCTIVE — wipes the Mini stash; operator call).
- Then one Mini run: `cargo test -p evaporchain-sfsv -p evaporchain-app-templates-engine -p evaporchain-app-templates-bind`.

**Honest bottom line:** the SFSV model-(a) + listing + init reconciliation is *code-coherent and the one Mini-found bug is fixed in content*, but **no trustworthy green has been produced** and producing one is blocked on environment (shared-index chaos + Mini git divergence), not on the code. Do not claim "verified."

---

## ✅ VERIFIED GREEN — 2026-05-16 (supersedes the UNVERIFIED status above)

After the operator-authorised destructive Mini reset (fixed the git divergence permanently) + completeness pre-check of all 5 SFSV-init-shape sites on `origin/main`:

**Tested at exact clean `origin/main` HEAD `7d54e784`** ("fix(app-templates-deploy): update SFSV required_keys to new InitConfig field names"). Full `cargo test -p evaporchain-sfsv -p evaporchain-app-templates-engine -p evaporchain-app-templates-bind`:
- `evaporchain-sfsv` adversarial suite: **22 passed / 0 failed**
- `predicate_inlining_parity.rs`: **9 passed / 0 failed** — machine-verifies `.es` inlined logic ≡ Rust predicate under model (a) (the EvaporScript-first invariant, now proven).
- Run completed through all 3 crates' doc-tests, no abort.

Two verification-found bugs both fixed on origin/main: (1) catalogue `default_params` missing `future_self`; (2) `app-templates-deploy/required_keys.rs` SFSV still requiring old `deposit` key (fixed in `7d54e784`).

**The SFSV model-(a) predicate + on-chain listing reconciliation is VERIFIED: compiles + tests green across all 3 crates on a Mini at a known exact HEAD.** No stale-HEAD / env caveats. The reconciliation reached origin/main via the multi-session concurrent sweep (not a controlled scoped commit) but the *content* is verified coherent and passing.

---

# DEPLOY-RUNBOOK API SPEC — corrected `deploy-sfsv.sh` (the remaining v1.0 e2e build)

`scripts/deploy-sfsv.sh` is fundamentally mis-shaped vs the real node API (`crates/evaporchain-node/src/api.rs`). This is the precise corrected spec, derived from the source (verified parts) + flagged open questions (do NOT fabricate). Whoever implements (parallel session / coordinated single session) builds to this.

### Verified-correct request schemas (read directly from api.rs)

**Deploy** — `POST /api/tx/deploy-script`, handler `post_deploy_script`:
```
DeployScriptRequest { deployer: u8, source_code: String, energy: u64, half_life: u64 }
```
→ `deployer` is a **u8 devnet account index**, NOT a hex address. `source_code` = full `future_self_vault.es` text. Returns `TxResultResponse` (tx hash).

**Call** (`set_terms`, `try_payout`, read-only queries) — `POST /api/tx/call-script`, handler `post_call_script`:
```
CallScriptRequest { caller: u8, contract_id: u64, method: String, args: serde_json::Value, epoch: u64 }
```
→ `caller` is **u8**, `contract_id` is **u64** (not hex), and **`epoch: u64` is REQUIRED** (script currently omits it). `set_terms` args = `[future_self, predicate_type, release_param, deposit_amount]` (matches `.es` `set_terms` signature). `try_payout` args = `[]`.

**Contract read** — `GET /api/contract/:id` (`id: u64`), handler `get_contract`. Returns:
```
{ id, template, creator, energy, half_life, created_epoch, evaporated, state }
```
→ **No `/state` suffix route. No `/api/contract/by-deploy/:hash` route.** `released`/`payout_at`/`predicate_satisfied` are NOT top-level — logical vault state is the opaque `state` object (serialized EvaporScript contract state).

### Corrected flow
1. Deploy: POST deploy-script (deployer=u8 acct, source=.es text, energy, half_life) → tx hash.
2. Poll `GET /api/tx/<hash>` until `state ∈ {included,finalised}`.
3. Resolve `contract_id` (u64) — node has `resolve_deploy_contract_id` + `store.get_deployed_contract_id(tx_hash)`. **OPEN Q (confirm in api.rs before finalizing):** exact response field/endpoint exposing it — the `/api/tx/:hash` API-doc field list (api.rs:8695) does NOT include contract_id, so the resolution is via a different path; locate it.
4. `set_terms` via call-script (caller=u8, contract_id, method="set_terms", args=[future_self,predicate_type,release_param,deposit_amount], epoch=current). Poll tx.
5. Predicate wait: **no `predicate_satisfied` GET.** Either (a) call-script `predicate_satisfied` and read tx result, or (b) `GET /api/contract/<id>` and parse `.state`. **OPEN Q:** does call-script support read-only (no-mutation, no-gas) invocation for `predicate_satisfied`/`is_released` (api.rs ~9482 hints at "call-script … for reads")? Confirm; prefer read-only.
6. `try_payout` via call-script. Poll tx.
7. Verify released: call-script `is_released` and assert result == true, OR parse `GET /api/contract/<id>` `.state`. Confirm SFSV serialized-state shape for the released flag.

### Open questions — resolved from source (2026-05-16)
- **OQ1 RESOLVED:** deployed contract_id is returned by `GET /api/tx/<hash>` itself — TxStatus struct has `pub contract_id: Option<u64>` (api.rs:9485), populated for deploy txs via `resolve_deploy_contract_id` (api.rs:9545; persistent-index path + live-engine heuristic). Runbook step 3 = poll the deploy tx, read `.contract_id`. The `/api/contract/by-deploy/:hash` route does NOT exist; do not use it.
- **OQ2 CHARACTERIZED:** `post_call_script` is **tx-only** (returns `tx_hash`; poll via `/api/tx/:hash`). No read-only / dry-run / simulate endpoint exists. ⇒ runbook must NOT expect a `predicate_satisfied` GET; it `call-script`s the method as a tx, then observes state. Residual: how a call-script tx surfaces an EvaporScript method **return value** (1/0) is unclear — needs a `post_call_script`-result/events dig before steps 5/7 can read a return directly; until then, observe via OQ3 state.
- **OQ3 RESOLVED:** `ContractEngine::get_state(id) -> Option<&serde_json::Value>` returns the contract's `state` (`evaporchain-contracts/src/lib.rs:768`). EvaporScript VM state is `HashMap<String, Value>` (`evaporchain-script/src/vm.rs:120`) keyed by the `.es` `state {}` field names. So `GET /api/contract/:id` `.state` is a JSON object with `released`, `sealed`, `listed`, `payout_at`, `holder`, … directly readable. Caveat: map values commonly serialize as `Value::U64` (per EvaporScript grammar-gotcha) — treat `.state.released` as `0/1` (accept truthy/1, not strictly JSON `true`).

### Final corrected flow (fully source-grounded, zero guesses)
1. `POST /api/tx/deploy-script` `{deployer:<u8>, source_code:<.es text>, energy:<u64>, half_life:<u64>}` → tx hash.
2. Poll `GET /api/tx/<hash>` until `state∈{included,finalised}`; read `.contract_id` (u64) from that same response.
3. `POST /api/tx/call-script` `{caller:<u8>, contract_id, method:"set_terms", args:[future_self,predicate_type,release_param,deposit_amount], epoch:<cur>}`; poll its tx hash.
4. **No predicate-poll endpoint needed.** Retry `POST /api/tx/call-script` `{caller, contract_id, method:"try_payout", args:[], epoch:<cur>}` on a cadence — `.es try_payout` reverts cleanly until the predicate trips (idempotent/guarded), so loop until the tx reaches `included` (not `rejected`).
5. Verify: `GET /api/contract/<id>` → assert `.state.released` is truthy (`1`/true). Optionally read `.state.payout_at`, `.state.holder`.

## README audit (#358, parallel-session-owned) — 2026-05-16

**Forkability bar:** the "Fork recipe" section is adequate *as written* — concrete 6-step recipe, 30–80 LOC fork-distance, APPLICATION_UNIVERSE pointers, predicate-inlining + parity-test + adversarial-test porting guidance. Meets the *recipe* portion of the "demonstrably forkable" success criterion. A *demonstrated* fork (actually forking the pattern to another decay category) is still task-G / post-build success-criterion, NOT closed.

**README is code-WRONG (drifted pre-reconciliation — examples won't compile vs current origin/main):**
1. `Predicate::EnergyDecaysBelow { initial_energy, half_life, created_at_epoch, threshold }` + "Frozen-formula parameters" — the **removed** old shape; model-(a) is `{ threshold }` only, engine-supplied live energy.
2. `PredicateContext { epoch_now }` — missing required `contract_energy`.
3. `payout(&mut v, 100)` — old 2-arg; reconciled is `payout(v, epoch_now, contract_energy)`.
4. `use evaporchain_sfsv::market::{open_listing, settle_secondary}` — `open_listing` doesn't exist (`list_for_sale`).
5. Quick-start deploy flags `--deployer-addr 0xdeadbeef` / `--future-self 0xcafef00d` — the **old broken** script CLI; rewritten script uses `--deployer <u8>` etc.
6. Status table (adversarial 17→22; no `listing_parity.rs` row; market 6→7; predicate.rs counts) + roadmap "25 base + 17 adversarial + 9 parity" — stale.

**Action (do NOT edit the parallel-session README here):** a coordinated session must correct examples 1–5 (compile-breaking) before the README is trustworthy; 6 is cosmetic-stale (same class as the §10.1 drift below). Recorded for fold-in.

## SFSV_ARCHITECTURE.md §10.1 / §10.2 doc-drift (parallel-owned — recorded, not edited)

`§10.1 Shipped` table is **pre-reconciliation stale**. Corrected post-reconciliation snapshot (VERIFIED GREEN at `origin/main` HEAD `7d54e784`):

| Surface | §10.1 says (stale) | Actual now |
|---|---|---|
| `predicate.rs` | 169 LOC, 6 unit + 2 proptests | reworked to **model (a)** (`EnergyDecaysBelow{threshold}`, `PredicateContext{epoch_now,contract_energy}`); tests rewritten |
| `payout.rs` | 5 unit | 3-arg `payout(v,epoch,contract_energy)`; tests reconciled |
| `vault.rs` | 194 LOC, 8 unit | + on-chain **listing state machine** (`Listing`, `list_for_sale`/`cancel_listing`/`record_sale`, guards) + unit tests |
| `market.rs` | 6 unit | **7** (settle routes via `record_sale`; §5-A wiring test added) |
| `tests/adversarial.rs` | 17 | **22** (reconciled §8 + refresh-aware + §8.6 listing adversaries) |
| `tests/predicate_inlining_parity.rs` | 9 | 9 (green) |
| `tests/listing_parity.rs` | — (absent) | **NEW** (§5-A listing `.es`↔Rust parity, 8 tests) |
| status | "drafted / green on satyawan" | **VERIFIED GREEN Mini1 @ 7d54e784** |

`§10.2 Gaps for v1.0` reconciled: #1 adversarial suite ✅DONE+green · #2 predicate bit-parity ✅DONE+green (listing parity now ALSO closed via new `listing_parity.rs`) · #3 deploy script — **rewritten to source-verified spec, dry-run-clean, NOT live-verified (ENV-GATED, task #5)** · #4 TS view #359 exists, e2e-unverified (ENV-GATED, task #6) · #5 README #358 exists but has 6 compile-breaking drift errors (recorded above).

§10.3 frontier (§5.3–5.7) correctly v1.1+/out-of-scope. **Action:** a coordinated session folds the corrected §10.1/§10.2 numbers + the README fixes into the parallel-owned docs; recorded here so nothing is lost across the multi-session sweep.

### Honest status — SPEC COMPLETE
All request schemas **source-verified**; OQ1/OQ2/OQ3 all **resolved from source**; the released-observation design is settled (retry-idempotent-try_payout + `.state.released`, no extra endpoint). **This spec is implementation-ready with zero remaining guesses.** What remains is purely: (a) rewrite `deploy-sfsv.sh` to this spec, and (b) run it against a live `evaporchain-node` Mini devnet — both gated on single-session coordination + a clean Mini, not on any unknown. The reconciliation itself is VERIFIED GREEN; this completes the e2e-runbook design.

## ✅ LIVE E2E PASS — Task #5 — 2026-05-16

`deploy-sfsv.sh` (corrected, local-only) ran end-to-end against a **live isolated single-node devnet** on Mini 1 (`127.0.0.1:8099`; `--mock-consensus --mock-prove`, `/tmp` data-dir, `peer_count:0` — zero cluster contact; a parallel session's smoke node, reused not provisioned). Full lifecycle on a real chain:

| Step | Result |
|---|---|
| deploy-script (11,248 B `.es`) | finalised, `contract_id:4`, gas 150000 |
| set_terms (tagged-Value args) | **finalised** — all 5 `.es` `require()`s passed |
| try_payout | **7 predicate-gated rejections** (epochs 2585–2610 < release 2610) → **finalised at epoch 2614** (≥ release) |
| verify | `/api/contract/:id` → 404 (see CORRECTION below — devnet limitation, not retirement) |

> **CORRECTION (2026-05-16, from the #6 investigation):** the step-4 404 was **NOT** "instance retired post-payout per `.es` §lifecycle-4" as first written below. Root-caused during #6: this `--mock-consensus` devnet **never surfaces script contracts** via `GET /api/contract/:id` or `/api/contracts` — a *just-finalised* deploy is `contract not found` within 1 s, with `/api/contracts` empty, node uptime monotonic (no restart, PID stable). Script tx **execution is real** (the 7 predicate-gated rejections → finalise exactly at `release_epoch` cannot be faked — the `.es` genuinely ran), but the API-queried `contract_engine` is not populated on this devnet config. The #5 **PASS still stands** (deploy ✓, set_terms ✓, predicate-gated finalised try_payout ✓ — the lifecycle executed); only the *explanation of the 404* was wrong. `released==true` remains soundly inferential (below); it is simply unobservable here for a devnet reason, not a payout-retirement reason.

**Two real bugs the live e2e caught (both fixed in the local script; not in the spec above):**
1. **call-script args were bare positionals** (`["2",0,1606,1000]`). Node decodes `tx.args` as `Vec<evaporchain_script::Value>` — an **externally-tagged** serde enum. Correct form: `[{"Address":[b0..b31]},{"U64":n},…]` (32-byte addr = `addr_from_byte`: `[idx,0×31]`). Canonical form proven by `evaporchain-execution` tests (`args: r#"[{"U64": 42}]"#`). Without this, set_terms always `rejected`.
2. **verify step asserted bare `.state.released`** — state is `HashMap<String,Value>`, fields externally-tagged (`{"Bool":true}`). Fixed: tagged-aware read + a **non-vacuity guard** (must observe ≥1 predicate-`rejected` before the finalised try_payout, else fail — prevents a tautological green) + tolerate a post-payout `/api/contract/:id` 404. (NB: the 404 cause was later corrected — see CORRECTION above: it's a `--mock-consensus` devnet limitation, *not* payout-retirement. The tolerate-404 branch is still correct behavior; only its rationale changed.)

**Auth/funding facts (source-verified, needed to drive any auth-enabled node):** tx routes gate on `require_tx_auth` → session token; register **auto-verifies on testnet** (`auth.rs:323`), login doesn't gate on `verified` → register→login mints a 128-char bearer. Genesis pre-seeds the **all-zeros faucet** (`addr_from_byte(0)`, balance `u64::MAX/2`); `addr_from_byte(1..)` are NOT funded → use `--deployer 0` to skip the admin-key-gated faucet.

**`released==true` is proven *inferentially*, not by a direct state read:** `.es` `try_payout`'s only success path is `require(sealed) → require(!released) → require(predicate) → released=true; payout_at; emit`. A finalised try_payout that was *previously predicate-rejected* (non-vacuity guard enforces this) ⟹ released was set. No live `released` read is obtainable on this devnet (script contracts aren't surfaced via `/api/contract/:id` here — see CORRECTION) — so this is a sound proof from source + the predicate-gated tx transition, not an observed field. Honest, non-overclaimed PASS.

**CAVEAT (carry forward):** the corrected `deploy-sfsv.sh` lives **only on the MacBook working copy**. `origin/main` still has the old #357 mis-shaped version; the Mini working tree was `git checkout`-restored to HEAD after every run (no commit — multi-session shared tree, not this session's call to land it). The fix must be folded into the repo by a coordinated session along with the §10.1/§10.2/README drift.

## ✅ TS VIEW (#359) E2E — Task #6 — 2026-05-16

`crates/evaporchain-sfsv/ui/index.html` (533-line single-file dApp UI) was **written to the same mis-shaped API as the old #357 script, plus 3 nonexistent endpoints** — i.e. demo-only, could not complete a single op against a real node. Verified against source (`api.rs`) + the live Mini-1 node. Defects found & fixed (local-only edit; parallel-session-owned file — same no-commit discipline as the script):

| # | Defect (original UI) | Fix (source-verified) |
|---|---|---|
| 1 | `deployer:` hex string | `u8` index (Number) |
| 2 | `caller:` hex string | `u8` index (Number) |
| 3 | `contract:` key + addr | `contract_id: <u64>` |
| 4 | `args:[fs,n,n,n]` bare | tagged `[{Address:[…32]},{U64:…}]` |
| 5 | no `epoch` on call-script | `epoch:` from `/api/status` |
| 6 | `GET /api/contract/by-deploy/:hash` (no such route) | poll `/api/tx/:hash` → `.contract_id` |
| 7 | `GET /api/contract/:addr/state` (no such route) | `GET /api/contract/:id` |
| 8 | `s.released` etc. (untagged, no `.state`) | `untag(s.state.released)` (`{Bool:true}`→true; Address→0x…) |
| 9 | probe `GET /api/version` (no such route) | `GET /api/status` |

**Live-node verification of the corrected UI's exact JS-built request bodies** (curl mirror, `127.0.0.1:8099`):
- deploy (`deployer:0` as Number, unique energy) → **finalised, fresh `contract_id`** ✓
- `pollContractId` (poll `/api/tx/:hash` → `.contract_id`) → resolved ✓
- set_terms (`caller:0`, `contract_id`, **UI's exact tagged args** `[{Address:[0×32]},{U64:0},{U64:rp},{U64:1000}]`, `epoch`) → **FINALISED — UI tagged args accepted** ✓ (the central fix; identical encoding to the #5 PASS)
- try_payout body shape = the #5-verified shape (`caller` u8, `contract_id`, `args:[]`, `epoch`) — predicate-gate already proven in #5
- probe → `/api/status` reachable ✓

**Not positively verifiable on this devnet (node limitation, NOT a UI defect):** the UI's `refresh()` reads `GET /api/contract/:id` and `untag`s `.state.*`. The request shape is source-correct and the 404 branch is handled as terminal — but this `--mock-consensus` devnet **does not surface script contracts** via that endpoint (a just-finalised deploy 404s; `/api/contracts` empty; node uptime monotonic / no restart). So the positive state-render path is shape-correct + failure-path-handled, but unobservable here for the same devnet reason as the #5 CORRECTION. A real-consensus node (or a node whose API contract_engine is populated) is needed to see a live `released` render; the UI code itself is now source-faithful.

**Honest #6 verdict:** every UI **write-path** body (deploy, set_terms w/ tagged args, try_payout) is **live-verified** against the node; the **read-path** is source-correct + fail-safe but unobservable on this devnet. The UI is no longer demo-only — it speaks the verified API. **In-browser DOM/UX was NOT exercised from this headless environment** (stated, not claimed). Same CAVEAT as #5: fix is MacBook-local only; fold into the repo with the §10.x/README drift in a coordinated session.
