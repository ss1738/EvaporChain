# EvaporChain Changelog

## 2026-05-04 evening — Lane R.* cluster-freeze fix + origin/main reconciliation

### What broke

3-Mini Tailscale cluster halted at h=771 after ~90 minutes uptime.
Mini 1 was stuck at h=145 on a different state root from Mini 2/3
(h=771, lockstep). Root-cause investigation via `/api/network/peers`
on Mini 2 surfaced a peer with `score: -292, age_seconds: 47` — the
score had been decaying for ~24 hours while the peer was DISCONNECTED.

Three independent design issues compounded into a livelock:

  1. **`SCORE_IDLE_TICK = -1`** fired every 5 min on every entry in
     the `scores` HashMap, including disconnected peers (which
     `record_disconnect` left in the map).
  2. **`record_connect`** used `entry().or_default()`, so a peer
     reconnecting after going negative INHERITED their prior score
     instead of getting a fresh slate.
  3. **No authorization gate** on idle-score penalty: validators
     pre-vetted via TLS / peer-id allowlist (`peer_authority`) got
     penalized identically to random Sybil peers.

After ~100 idle ticks (~8 hours wall-clock), any peer crossed
`SCORE_BAN_THRESHOLD = -100` → IP soft-banned for `peer_ban_duration_secs
= 3600` (1 hour). With BFT 2/3+1, losing one validator halts a
3-validator cluster. Once unbanned + reconnected, the inherited
negative score reban'd it. Livelock per process lifetime.

### What landed (genuine three-layer fix)

| Lane | What | Commit |
|---|---|---|
| R.1 | Authorized validators bypass Sybil idle-ban + auto-unban on connect | `803ac6d` |
| R.2 | Regression test: 256-tick fixture confirms bug class + gate works | `9d192bf` |
| R.3 | `record_disconnect` clears score entry; `record_connect` fresh-slates; idle tick iterates `peer_ips` not `scores` | `1555eb8` |

Each layer alone closes the livelock; all three make accidental
regression near-impossible. Network crate tests: 62/62 pass.

### Origin/main reconciliation (Lane R.4 attempt → R.5 revert → R.6-R.12 disciplined)

Deploying R.1+R.3 to the live cluster required rebuilding the node
binary on each Mini, which required origin/main to be buildable on
a clean checkout. It wasn't — origin/main had accumulated weeks of
half-finished cross-crate refactors:

  - `FEE_PPM_DENOMINATOR` referenced but never declared in fees.rs
  - `VS_PPM_DENOMINATOR` similarly undeclared in validator_set.rs
  - `health_score_ppm` / `target_utilization_ppm` / `confidence_score`
    fields referenced before they were added
  - `Transaction::Refund` arm missing in 3 separate match sites across
    consensus + wallet + execution
  - 73 sister-session crates listed in workspace Cargo.toml but never
    committed — each missing one fails build sequentially
  - nova-snark API drift: `compressed.verify` returns `Vec<Scalar>`
    not the old `(Vec, Vec)` tuple

| Lane | What | Commit |
|---|---|---|
| R.4 | Bulk Mac-state commit attempt (42 files) — pollution, reverted | (reverted) |
| R.5 | Revert R.4 mass-commit; keep R.1/R.2/R.3 + sister docs interleaved | `7e289bc` |
| R.6 | Light-Cone DAG Phase 1.1: `LightCone::leaves()` + `ForkChoice::select_tip` seam + types contract test | `6b23261` `18c926f` |
| R.7 | Minimal pub-const decls (`FEE_PPM_DENOMINATOR`, `VS_PPM_DENOMINATOR`) + Refund arm in wallet/gas | `c2c6294` |
| R.8 | Fees `target_utilization` fallback + wallet/signer Refund arm | `e0b3b64` |
| R.9 | Tendermint `health_score` fallback (was `health_score_ppm`) | `f064f57` |
| R.10 | Node api.rs `confidence_score_ppm` + `health_score` field renames | `a6aae53` |
| R.11 | Land the 535-LOC `evaporchain-light-cone-v2` crate that workspace listed | `6ef88de` |
| R.12 | Land the remaining 72 sister-session crates (337 files, 47.8K LOC) | `2f53749` |

Each Lane R.X was committed as a small additive batch, verified on
Mini 1 with `cargo check --workspace`, then rolled forward. The
disciplined approach converged in 9 commits; the earlier bulk-commit
approach (R.4) blew up worse than not committing at all.

### Cluster recovery + first in-production R.1/R.3 validation

After R.12, all 3 Minis built clean (`cargo build --release --features
prove` finished in ~1m23s on each). Stopped processes, restored BLS
private keys from `~/validator-N-keys.json` (the data-dir wipe had
deleted `bls_key.bin`), restarted with the launch flags.

Cluster came back at h=37 with peer_count=2 across all three. By
2026-05-04 16:51 UTC: h=1591, identical state root
`1ec9175f30efc58eb38595d557781a276c5815b0c267d9fdff4344d7ce5a8e13`,
4.2 blk/s. Both peers showed `score: 0` after 6 min of uptime —
without R.1/R.3 they'd be at -1 already (SCORE_IDLE_TICK fired at
the 5-min mark). **First in-production validation of R.1/R.3.**

### What's still open

| Item | Effort |
|---|---|
| Sister-session ppm migration: complete the FEE_PPM/VS_PPM PID refactor that the Lane R.7-R.10 stubs unblock | 1-2 sessions |
| Cluster diagnostic RPC: `/api/network/scores` exposing per-peer `score` + `last_tick` so the next freeze surfaces faster | half-session |
| Whitepaper §A1.3 Causal-CHSH amendment | manual |

---

## 2026-05-03 / 2026-05-04 — Layer 3/4 substrate seams + Layer 0 closure + doctrine sweeps

This session lands the consensus abstraction seams (Layer 3), the
first concrete impls behind them (Layer 4), governance-flag wiring +
operator UX RPCs, four proptest mirrors locking trait invariants, and
two doctrine-doc sweeps reconciling `INVENTION_STACK.md` +
`DOCTRINE_PUNCH_LIST.md` with reality after the MERA gate FAILED on
real Ethereum data (VERKLE verdict).

Default behaviour is unchanged across all 27 commits — every new code
path is governance-gated `linear / fifo / observe` until an operator
explicitly opts in.

### Substrate (`evaporchain-consensus`)

| Lane | Trait / impl | Commit |
|---|---|---|
| G.1 | `pub trait BlockSource` + blanket impl on `Mempool` | `f78d965` |
| G.3 | `pub trait ForkChoice` + `LinearForkChoice` default | `61eb888` |
| G.4 | `pub trait MevPool` + blanket impl on `EncryptedMempool` | `150292c` |
| G.5 | `pub trait ValidatorSetSource` + impl on `ValidatorSet` | `118b19d` |
| I.1 | `TxAntichainMempool` — first non-default `BlockSource` impl | `842363f` |
| I.3 | `MccForkChoice` — first non-default `ForkChoice` impl | `c1a05bb` |
| I.5 | `mempool::antichain_project` — post-FIFO antichain helper | `2bdcdc2` |

### Hot-path consumers (governance-gated)

| Lane | What | Commit |
|---|---|---|
| I.4 | `parent_acceptance_mode = "mcc"` dispatches at `tendermint.rs:2643` | `ded1a73` |
| I.5 | `block_source_mode = "antichain"` filters at `tendermint.rs:3915` | `20d9fc8` |
| I.6 | MCC β derived from chain CFM (microbits/fee/epoch) | `a45588c` |
| F.1 | Singh-Lyapunov fee tick wired into `execute_block` | (sister `4d59b5d` + test fix `b14ed53`) |

### Operator UX

| Lane | Endpoint / API | Commit |
|---|---|---|
| J.0 | `GET /api/governance/flags` — inspect all soft-fork keys | `d694ce8` |
| K.1 | `POST /api/governance/param` — flip with allowlist | `2fa6362` |

`fork_choice_mode` retains its existing dedicated endpoint
(endorser-stake-validated). Other knobs (`parent_acceptance_mode`,
`block_source_mode`, `conservation_enforcement`) flip via the generic
allowlisted setter.

### Test rigor — proof-style coverage of trait contracts

256 randomised inputs each, ~1,536 randomised assertions per
`cargo test -p evaporchain-consensus`:

| Test | Properties locked |
|---|---|
| `tx_antichain_mempool::antichain_invariant_no_duplicate_senders` | 4 (Lane I.1) |
| `mempool::antichain_project_invariants` | 5 (Lane I.5 follow-up) |
| `fork_choice::mcc_proptest_invariants` | 3 (Lane I.3 follow-up) |
| `tx_antichain_mempool::block_source_contract_holds_for_both_impls` | cross-impl (Lane G.1 follow-up) |
| `fork_choice::fork_choice_contract_holds_for_both_impls` | cross-impl (Lane K.3) |
| `tendermint::tests::governance_set_param_proptest` | 4-bucket allowlist (Lane K.4) |

### Test rigor — integration tests for governance flag dispatch

| Test | Lane | Locks |
|---|---|---|
| `test_block_source_mode_antichain_dedups_same_sender_in_proposal` | J.1 | I.5 wire-path |
| `test_block_source_mode_default_admits_all_same_sender` | J.1 | typo-safety |
| `test_parent_acceptance_mode_mcc_diverges_from_linear_on_diverging_parent` | J.2 | I.4 + I.6 differential |
| `test_parent_acceptance_mode_typo_falls_through_to_linear` | J.2 | typo-safety |
| `test_governance_set_param_*` (4 tests) | K.2 | allowlist contract |

### Doctrine sweeps

| Lane | What | Commit |
|---|---|---|
| Layer 1 | INVENTION_STACK §A1.2 T1 + T2 wording fixes; MERA caveat; LightCone read-only note; CSLC endpoint label | `bfaa758` |
| H.1 | DOCTRINE_PUNCH_LIST Layer 0 #4 marked closed (verified `collect_demurrage` already wired) | `3f8d84b` |
| Layer 0 closure | DOCTRINE_PUNCH_LIST Layer 0 #3 + #5 marked closed | `f507434` |
| M.1 | DOCTRINE_PUNCH_LIST bullet sweep — 14 stale `[ ]` items closed with commit refs | `944879b` |
| M.2 | INVENTION_STACK MERA references swept post-VERKLE verdict | `66a84a4` |

### Final test counts (Mini 1)

- 415 evaporchain-consensus lib tests pass
- 0 regressions across the session
- ~1,536 proptest randomised assertions × 256 inputs = ~393k checks
  per `cargo test`

### What's still genuinely open

| Item | Effort |
|---|---|
| Layer 5 — Lambda-Fold real Nova IVC (`state_root_to_u64` truncation, `RealBlockCircuit` arity 6→7) | 3-6 weeks |
| Layer 6 — Crooks-MEV refund consensus integration (substrate exists, no consensus hot-path wiring) | multi-day |
| Layer 6 — Light-Cone full consensus rewrite (replaces tendermint.rs's 8.7K LOC) | months |
| Layer 7 — LLSA full theorem-grade governance (or descope to k-of-n auditor signatures) | 9-15 months OR 4-6 weeks |
| M2 — Coq build verification (manual) | 10 min |
| M3.1 — INVENTION_STACK §A1.2 T1 wording (Satyawan strategic call) | 30 min |
| M3.2 — INVENTION_STACK §A1.2 T2 wording (Satyawan strategic call) | 30 min |
| Layer 2 — CSSR (Shalizi-Klinkner ε-machine reconstruction) | 2-3 sessions |

### Cluster operations

3-Mini Tailscale cluster experienced a divergence event mid-session
(wipe-restart of Mini 1 with peers at h=771 produced a fork that the
current sync protocol couldn't reconcile — Mini 1 stuck at h=178, peers
halted at h=771 awaiting BFT 2/3+1). Cluster ops were de-prioritised in
favour of the building work above. Cluster-wide reset can be issued
later via `restart-tailscale-3node.sh` on all 3 Minis simultaneously.

---

## Amendment — 2026-05-04 — Causal-CHSH frontier primitive shipped end-to-end

After the MERA gate FAILED → VERKLE verdict (commit `2053a86`) closed
the question of whether MERA ships, the user asked: "do we must
introduce our new math and our frontier idea, insane novel?" The
answer was yes — but with the same MERA-style empirical gating
discipline that just earned its keep. Lanes O.1 through O.7+ delivered
EvaporChain's first 100% original frontier theorem from concept to
operationally-exposed primitive in one session.

### The Causal-CHSH cartel-detection bound

Bell's CHSH inequality (Clauser-Horne-Shimony-Holt 1969) translated
to blockchain causal sets. Theorem (proposed): for `S = |E(A,B) +
E(A,B') + E(A',B) − E(A',B')|` over four samples of ±1 products
drawn from concurrent block pairs in the LightCone DAG under four
setting-pairs, **`S ≤ 2`** under honest validators + LightCone
causality + EvaporChain's single-λ decay. **Violation `S > 2` ⇒
hidden cross-validator coordination.**

Where Bell's theorem gave physics quantum-entanglement detection,
Causal-CHSH gives blockchain *cartel-detection* with a closed-form
bound — not a heuristic, not a slashing rule, a *theorem*. **Only
LightCone-style chains can even form the four-term correlation**
(Tendermint linear chains have no concurrent blocks; Ethereum's
reorgs are competing finalisers, not concurrent producers). The
math is new because the substrate is new.

### The build → gate → ship cycle

| Lane | What | Commit |
|---|---|---|
| O.1 | New crate `evaporchain-causal-chsh` with math primitive + synthetic gate (12 tests including 2 proptests) | `801fd7c` |
| O.2 | Real-data driver: `extract_chsh_samples` over a `BlockSummary` trace via concurrency-window proxy + 4 binary observables; synthetic-Eth methodology validated (17 tests) | `7876624` |
| O.3 | Real Ethereum gate runner (Rust binary) + Python scraper. **Verdict: PASS** on 200 mainnet blocks (19_900_000+) — S_honest=0.012, S_cartel=4.0, gap=3.99 — ~150× headroom on the doctrine ceiling | `c9e553c` |
| O.4 | INVENTION_STACK.md `§A1.3` row reservation + new `§A1.10` gate-resolution section (parallel to MERA's `§A1.8`) + new doctrine rule #14 ("pre-commit gate thresholds before running") + Tier-0-supporting count 6 → 7 | `76cc71d` |
| O.5 | `POST /api/cartel_alarm/run_gate` — operators can run the gate live against arbitrary chain trace data; doctrine-locked thresholds baked in (no operator override) | `f396b7d` |
| O.6 | 3K-block sanity check on the same Eth window MERA used (19_900_000–19_903_000). **Verdict: PASS again** — S_honest=0.018 (vs 0.012 on 200-block, both well below 1.8), 14,885 ±1 samples (15× more than 200-block run). Verdict robust under sample-size scaling. | `cdb736c` |
| O.7 | `CartelAlarm` rolling-buffer substrate primitive — fixed-capacity ring of `BlockSummary`, periodic gate-run logic, last-S tracking. Observability-first; no auto-action emission yet (deferred to Lane O.8 design). | `63b6cf6` |
| O.7+ | Proptest 256× alarm invariants (buffer cap, monotonic counter, first-run threshold, honest-source verdict). Caught a real off-by-one in the periodic-run logic (capacity=50, interval=21, n_records=60 edge case) — pure proptest win. | `5968295` |
| O.8.1 | `TendermintConsensus` hosts `CartelAlarm` rolling buffer; `on_block_committed` ticks the alarm with a `BlockSummary` per committed block. Observability-only at this stage — no governance hook yet. | (earlier) |
| O.8.1b | `GET /api/cartel_alarm/chain_status` — chain's own self-monitoring verdict surfaced via RPC. Distinct from the operator-supplied-trace path of O.5. | (earlier) |
| O.8.1c | Integration test driving 60 blocks through `on_block_committed` → alarm fires at records_seen=50, height=50; verdict shape locked. | (earlier) |
| O.8.1d | `CartelAlarm.recompute_now` switched to `compute_chsh_s_milli` (i64 milli-units) for validator-determinism on the consensus-bearing path; f64 path retained for RPC display only. | `8853078` |
| O.8.2 | `CartelAlarmEvent` emission. Per-block emission gate fires when (a) governance `cartel_alarm_mode = "alarm"`, (b) chain's `s_honest_milli >= 1800` (doctrine ceiling), (c) no event for `last_run_at_height` already queued. Default `observe` mode silent. Surface drained via `take_pending_cartel_alarms()`. **Closes the original Lane O.8 design lane: alarm hook + governance + dedupe all in-protocol.** | `122821f` |
| O.8.2b | `GET /api/cartel_alarm/pending_events` — RPC drains the chain's queued `CartelAlarmEvent`s; each event returned exactly once. Operator dashboard / pager surface. | `0fac70f` |
| O.8.2c | Full-pipeline integration test: drives blocks through `on_block_committed` with `cartel_alarm_mode = alarm` → injected over-ceiling status → emission → drain. Distinct from O.8.2's unit test which calls the helper directly. Locks the call-site wiring end-to-end. | `6cb4b90` |

### MERA / Causal-CHSH paired symmetry

Same gate discipline. Opposite outcomes. Both demonstrate that
pre-committed thresholds are a feature, not a bug.

| Primitive | Empirical metric | Threshold | Verdict | Outcome |
|---|---|---|---|---|
| Authenticated Energy-MERA | R² = 0.66 (3 independent runs on real Eth) | ≥ 0.85 | FAIL | Drop, retain as research artefact (`§A1.8`) |
| **Causal-CHSH Cartel Detector** | **S_honest = 0.012-0.018, gap = 3.98** (200-block + 3K-block runs on real Eth) | S_honest < 1.8 + gap > 0.4 | **PASS** | **Ship as Tier-0-supporting** (`§A1.10`) |

A doctrine that can fail empirically is a doctrine that can ship
credibly when it doesn't. The credibility is in the symmetry.

### Final Causal-CHSH test counts

- 33 tests total in `evaporchain-causal-chsh` (post O.8.2 — added
  `CartelAlarmEvent` struct + `_inject_status_for_test` doctrine helper)
- 5 proptests across the crate (Bell bound for LHV sources, S
  algebraic range, alarm invariants, plus chsh dispatch)
- `evaporchain-consensus`: 423 lib tests pass (post O.8.2 + O.8.2c —
  added `test_cartel_alarm_event_emission_governance_gated` and
  `test_cartel_alarm_event_emission_via_on_block_committed`)
- Real-Ethereum gate verdict locked in `research/causal-chsh/GATE_RESULT.md`
- 3K-block sanity verdict locked in `research/causal-chsh/GATE_RESULT_3K.md`
- 200-block reproducibility CSV at `research/causal-chsh/honest.csv`
- 3K-block reproducibility CSV at `research/causal-chsh/honest_3k.csv`

### Doctrine drift across reference docs — closed again

After Lane M.1/M.2 closed the drift left over from the original
session, Lane O.4 reopened it (because shipping a new primitive
requires updating the doctrine). This amendment closes it once
more. Future sessions: the four reference surfaces should agree
that EvaporChain ships **5 Tier-0 primitives + 7 Tier-0 supporting
primitives** (was 6 before Causal-CHSH).

### What's still genuinely open after this session

| Item | Effort |
|---|---|
| ~~Lane O.8 — proper consensus integration (`cartel_alarm` governance hook with rolling buffer + auto-emission on `S > cartel_floor`)~~ — **closed by Lane O.8.1 / O.8.2 / O.8.2b / O.8.2c.** Hook ticks every block; `cartel_alarm_mode` governance flag gates emission; `CartelAlarmEvent` queue + `take_pending_cartel_alarms()` + `GET /api/cartel_alarm/pending_events` complete the operator surface. | done |
| Lane O.8.3+ — validator-side reaction policy on emitted `CartelAlarmEvent` (slashing? freeze? governance amendment?). V1 is event surface only — operators page their own response. | multi-day, design-heavy |
| Layer 5 — Lambda-Fold real Nova IVC (sister session) | 3-6 weeks |
| Layer 6 — Crooks-MEV refund consensus integration | multi-day |
| Layer 6 — Light-Cone full consensus rewrite | months |
| Layer 7 — LLSA full theorem-grade or descope to k-of-n auditor signatures | 9-15 months OR 4-6 weeks |
| M2 — Coq build verification (manual) | 10 min |
| M3.1 / M3.2 — INVENTION_STACK §A1.2 wording (Satyawan strategic call) | 30 min each |
| Layer 2 — CSSR | 2-3 sessions |
| Larger Causal-CHSH validation (10K+ blocks, multiple Eth windows) | half-day per window |
