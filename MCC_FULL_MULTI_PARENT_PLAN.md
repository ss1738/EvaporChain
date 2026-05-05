# MCC Fork-Choice Full Multi-Parent Enumeration — Plan

**Created:** 2026-05-05
**Status:** Phase 0 (planning) — implementation not started.
**Owner:** Satyawan Singh
**Sibling plans:** `LIGHT_CONE_FULL_DAG_PLAN.md`, `CROOKS_MEV_INTEGRATION_PLAN.md`, `LAMBDA_FOLD_NOVA_PLAN.md`.

## What this is

The single biggest blast-radius engineering item left in `DOCTRINE_PUNCH_LIST.md`
Layer 4. Until this lands, Light-Cone DAG mode is **decorative** — it
co-exists with Tendermint via `parent_acceptance_mode = "mcc"` but
can't replace it. The Phase 4.4 substrate shipped 2026-05-05 (digest
+ history + divergence finder) only matters at full operational
weight once this is done.

## Why "full" matters

Today's MCC fork-choice (shipped behind the `parent_acceptance_mode = "mcc"`
flag, Lane I.3 + I.4 + I.6, commits `c1a05bb`, `ded1a73`, `a45588c`) **walks
first-parent trajectories from both tips back to genesis** and scores them
via `mcc_choose` at β derived from chain CFM. Two big restrictions:

1. **Single-line tracking.** `tendermint.rs:2526` rejects any block off
   the single line — meaning sibling heads beyond the chosen tip are
   discarded, not enumerated.
2. **No state replay across tip changes.** When MCC selects a different
   head, validators have no machinery to roll the StateDB forward/back
   to that head's state.

The doctrine claim (`INVENTION_STACK.md §A1.2 T1`) is *"Our fork choice
is the unique trajectory `argmax exp(−β·E_path)` over candidate chain
trajectories — closed form by Lagrange duality on the maximum-entropy
program."* Honest reading: the *fork-choice math* is closed-form on a
single trajectory, but a real Maximum-Caliber Consensus over a DAG
requires enumerating **all** candidate trajectories (sibling heads) and
selecting the argmax across them. That's what this plan delivers.

## Phase breakdown

Each phase has substrate + integration + tests. Phase letters used
(not numbers) to avoid name-clash with the existing Light-Cone Phase
1-7 work which already touches some of the same files.

### Phase A — Substrate: track all sibling heads (1-2 days)

- [x] **A.1 — `candidate_heads()` accessor on `TendermintConsensus`.** ✅ SHIPPED 2026-05-05.
      *Design deviation (recorded in plan progress log):* shipped as
      a *derived* accessor on `light_cone_dag.leaves()` rather than a
      separately-maintained `sibling_heads: BTreeSet<BlockId>` field.
      Rationale: a parallel field would duplicate state and create a
      desync hazard; the DAG itself is the single source of truth for
      "what's a leaf right now." Returns `BTreeSet<[u8; 32]>` —
      validator-deterministic via `BTreeMap`-key iteration order on
      the underlying `LightCone`.

- [ ] **A.2 — Per-head trajectory caliber cache. DEFERRED to Phase C.**
      Premature optimisation at substrate level. The N-candidate
      scoring cost is observable only when the hot-path actually
      enumerates per-round; if Phase C reveals it's a bottleneck,
      revisit. Cache `(head_id, last_known_caliber, last_scored_block)`
      with O(1) re-score and invalidate on commit.

- [x] **A.3 — `enumerate_candidate_heads()` sorted-by-caliber.** ✅ SHIPPED 2026-05-05.
      Returns `Vec<([u8; 32], u64)>` of `(BlockId, caliber)` pairs
      sorted by caliber descending; ties broken by smaller `BlockId`
      first (matches `MccForkChoice::select_tip`'s argmax rule).
      Implemented via new `MccForkChoice::enumerate_with_caliber()`
      method that reuses `first_parent_trajectory` + `path_caliber` —
      `select_tip` now derives its argmax from this list (same
      scoring code path, single source of truth). β sourced from
      `governance_params["crooks_mev_beta_mb"]` default 1000 — same
      path `current_tip` uses.

- [x] **A.4 — Tests.** ✅ SHIPPED 2026-05-05. 5 unit tests in
      `evaporchain-consensus::tendermint::tests` (consensus suite
      474/0/1):
  - `mcc_phase_a_candidate_heads_empty_at_genesis`
  - `mcc_phase_a_candidate_heads_grows_under_concurrent_proposals`
  - `mcc_phase_a_candidate_heads_shrinks_on_extension`
  - `mcc_phase_a_candidate_heads_converges_across_validators`
    (validator-determinism: two `TendermintConsensus` instances with
    identical block-insertion sequences produce identical candidate
    sets at every step, including iteration order)
  - `mcc_phase_a_enumerate_candidate_heads_sorted_by_caliber`
    (key-set agreement with `candidate_heads()`, descending caliber
    order, argmax-equals-select_tip)

**Phase A acceptance:** ✅ ALL GREEN. No consensus-state-machine
surgery happened in Phase A as planned; both new accessors are
observability-only. `parent_acceptance_mode = "mcc"` continues to
behave exactly as before — `select_tip` was refactored to derive its
argmax from the new `enumerate_with_caliber` list, but the result is
unchanged. Phase A lays the foundation; Phases B and C build on it.

### Phase B — State-replay infrastructure (3-5 days, biggest risk)

This is the hardest work. The Light-Cone Phase 3 substrate already
ships `state_branches: HashMap<BlockId, LightConeBranchMetadata>` and
the `LightConeBranchSnapshot` trait. Phase B extends that from
metadata-tracking to actual state-replay.

- [x] **B.0 — LCA + block-path primitives on `LightCone`.** ✅ SHIPPED 2026-05-05.
      Foundation for B.2 (`replay_to_head`). Pure DAG operations on
      the existing `evaporchain-light-cone::dag` module:
      - `find_lca(lc, a, b) -> Option<BlockId>` — Lowest Common
        Ancestor; deepest (highest `observed_epoch`) common ancestor
        wins, smaller-`BlockId` tiebreak. None when either block is
        absent OR no common ancestor.
      - `block_path_from_to(lc, from, to) -> Option<Vec<BlockId>>` —
        first-parent path from `from` (excluded) to `to` (included)
        in chronological order. None when `from` is not a first-parent
        ancestor of `to`. `from == to` returns `Some(vec![])`.
      - 10 unit tests across linear / diamond / 3-fork / unrelated /
        missing-block / self-LCA / replay-walk-composition cases.
      - Light-cone test suite: 51 / 0 / 0 (was 41).
      - Re-exported from `evaporchain_light_cone` crate root.

- [x] **B.0+ — `plan_replay_to_head` planning substrate.** ✅ SHIPPED 2026-05-05.
      Composes the B.0 primitives into a `ReplayWalk` plan that the
      executor (Phase B.2) consumes:
      - `ReplayWalk { lca, forward_path, rollback_required }` struct
        on the consensus crate's public API.
      - `TendermintConsensus::plan_replay_to_head(from, to) ->
        Option<ReplayWalk>` accessor that calls `find_lca` +
        `block_path_from_to` and sets `rollback_required = lca != from`.
      - **No execution happens here** — pure planning. The executor
        consumes the `ReplayWalk` in B.2 to (a) roll back to `lca`
        if needed (the deferred B.1 snapshot work) and (b) apply
        `forward_path`'s blocks via `db.execute_block`.
      - 5 new tests: self→self no-op, forward-only no-rollback,
        rollback-required on branch switch, missing-head None,
        validator-determinism convergence. Consensus suite 479/0/1.

- [ ] **B.1 — Concrete `LightConeBranchSnapshot` impl in `evaporchain-state`.**
      Today the trait is consumed by `attach_branch_snapshot` on
      `TendermintConsensus` but no production impl exists; the trait
      is stubbed for tests. Wire to RocksDB snapshot per tip — one
      `Snapshot` per active head, lifetime-bounded by the head's
      presence in `state_branches`.

- [ ] **B.2 — `replay_to_head(target_head: BlockId)` on the executor.**
      Given the current StateDB (at some tip A) and a target tip B,
      compute the LCA in the Light-Cone DAG, roll back from A to LCA,
      then forward from LCA to B by re-executing the block sequence.
      Must be deterministic and atomic — partial replay leaves a
      corrupted state.

- [ ] **B.3 — Block-executor head awareness.**
      `execute_block` currently assumes `block.parent_hash` matches
      the StateDB's current state. Under multi-parent enumeration the
      proposer may build on a non-current MCC head; the executor
      needs to first `replay_to_head(block.parent_hash)`, then execute.
      Idempotent if already at the right head.

- [ ] **B.4 — Atomic head-switch transactional contract.**
      `replay_to_head` either succeeds completely (StateDB reflects
      target head) or fails completely (StateDB unchanged from
      original head). No partial state. Use the existing
      `db_guard.begin_batch()` / `commit_batch()` pattern.

- [ ] **B.5 — Memory cap enforcement.**
      `light_cone_max_concurrent_forks` already caps `state_branches`
      at default 4. Phase B must NOT exceed this — if a 5th head
      appears, evict the lowest-caliber existing head (and its
      RocksDB snapshot) before admitting the new one.

- [ ] **B.6 — Tests.** 6 unit tests + 1 integration:
  - `replay_to_head_no_op_when_already_at_target`
  - `replay_to_head_rolls_forward_through_linear_chain`
  - `replay_to_head_via_lca_through_diamond_dag`
  - `replay_to_head_preserves_atomic_contract_on_failure`
  - `head_switch_is_idempotent`
  - `state_branch_eviction_drops_rocksdb_snapshot`
  - Integration: `test_multi_head_replay_e2e` — drive 4-head DAG
    through `replay_to_head` cycles, assert final state at each
    head matches direct execution.

**Phase B acceptance:** state replay is correct and atomic across all
fork shapes the test fixtures cover; no consensus-state-machine surgery
yet (still gated on `parent_acceptance_mode = "mcc"`).

### Phase C — Hot-path integration (3-5 days)

Promote `authoritative_head` from admin-RPC to consensus hot path.

- [ ] **C.1 — `authoritative_head` selection at `start_round`.**
      Today (`tendermint.rs:954-969`) lives behind admin RPC; no
      hot-path consumer. Phase C calls
      `enumerate_candidate_heads().argmax(mcc_choose)` at the top of
      every consensus round and writes the chosen head to
      `current_authoritative_head` field.

- [ ] **C.2 — Voting handler dispatch by head.**
      `handle_prevote` / `handle_precommit` (already mirrored to
      `dag_round_states[tip]` in Phase 4 substrate) need to route
      votes to `current_authoritative_head`'s tally, not the legacy
      `parent_hash`'s. Existing per-tip `dag_round_states` HashMap
      is the receiver; just need to flip the dispatch.

- [ ] **C.3 — Proposer parent-set selection.**
      `create_proposal` (`tendermint.rs:4755`) currently sets
      `block.parent_hash = self.current_tip()` and `block.parents =
      vec![]`. Under multi-parent mode, the proposer:
      1. Sets `block.parent_hash = current_authoritative_head`
      2. Sets `block.parents = enumerate_candidate_heads()` (validates
         antichain via `is_antichain` from
         `evaporchain-light-cone::concurrency`)
      3. Or: explicitly multi-parent only when `parent_acceptance_mode
         = "mcc_full"` AND `protocol_version >= 3`.

- [ ] **C.4 — Equivocation rules under multi-parent.**
      A validator that prevotes for two different heads at the same
      round is equivocating. Cross-fork equivocation counter
      (`cross_fork_equivocations` HashMap, shipped Phase 4.3) already
      counts these; Phase C ensures the counter increments correctly
      on every observed double-vote at the consensus hot path.

- [ ] **C.5 — Validator-determinism gate.**
      Every honest validator at the same DAG state MUST select the
      same `current_authoritative_head`. Property test (proptest, 256
      random DAG shapes): two `TendermintConsensus` instances driven
      through identical block sequences produce identical
      `authoritative_head_history`. Mirrors the Phase 4.4 antichain
      digest convergence test pattern.

- [ ] **C.6 — Tests.** 5 integration tests:
  - `authoritative_head_selected_at_start_round`
  - `votes_route_to_authoritative_head_tally`
  - `proposer_emits_multi_parent_block_under_mcc_full`
  - `cross_fork_equivocation_increments_on_double_prevote`
  - `authoritative_head_converges_across_validators` (proptest 256x)

**Phase C acceptance:** `parent_acceptance_mode = "mcc_full"` flag (NEW,
distinct from existing `"mcc"`) routes consensus through the multi-parent
enumeration hot path. Default mode `"linear"` unchanged. `"mcc"` continues
to work as today (single-line trajectory walk).

### Phase D — Adversarial + performance tests (3-5 days)

- [ ] **D.1 — 4-validator 3-fork integration test.**
      Genesis → 3 concurrent proposals at h=1 (forks A, B, C). All
      4 validators vote on MCC-selected head; chain converges within
      2 rounds. End-state: single committed antichain across A, B, C
      consistent with Phase 4.2 finality predicate.

- [ ] **D.2 — Byzantine validator votes for non-MCC head.**
      Validator 4 prevotes for B when MCC selects A. Honest validators
      reject the precommit (low signature score on B's head), counter
      `cross_fork_equivocations[4]` increments. Stake slashed via
      `entropic_slashing` proportional to KL-rate.

- [ ] **D.3 — State-replay correctness under head churn.**
      Drive 100 blocks where MCC head switches every 10 blocks
      between two competing forks. Assert: final state at each head
      matches direct re-execution from genesis along that fork's path.

- [ ] **D.4 — Performance budget under 4 concurrent heads.**
      Match the Phase 6.3 Light-Cone perf budget: insertion < 500ns,
      `current_authoritative_head` < 500µs, state-branch ops < 20µs.
      Benchmark with `#[ignore]` annotation, run on Mini under
      release.

- [ ] **D.5 — Soak test on 4-validator local cluster.**
      Cluster runs `parent_acceptance_mode = "mcc_full"` for 72hr.
      Measures: zero stall events, zero divergent antichain digests
      via `/api/light_cone/antichain_digest`, < 5% throughput
      degradation vs `parent_acceptance_mode = "linear"` baseline.

**Phase D acceptance:** all five tests pass; performance numbers within
budget; soak test runs clean for 72hr.

### Phase E — Doctrine + operator surfaces (1-2 days)

- [ ] **E.1 — `/api/light_cone/candidate_heads` HTTP endpoint.**
      Returns all current sibling heads with caliber + selected flag.
      Operator-debugging surface for "which heads are competing right
      now."

- [ ] **E.2 — `/api/light_cone/authoritative_head` HTTP endpoint.**
      Returns current `authoritative_head` BlockId + the trajectory
      walked + the caliber score. Per-validator (different validators
      may briefly disagree during the round; converge by end of round).

- [ ] **E.3 — `LIGHT_CONE_FULL_DAG_PLAN.md` Phase 8 addendum.**
      Captures the MCC-full work as a follow-up to the existing
      Phase 4.x DAG-aware vote tally. Pointer to this plan doc.

- [ ] **E.4 — `INVENTION_STACK.md §A1.2 T1` update.**
      Doctrine claim updated from "argmax exp(−β·E_path) over candidate
      trajectories" (current Lagrangian re-label) to "argmax exp(−β·E_path)
      over the FULL multi-parent trajectory enumeration" (matches
      shipped). Honest about what's now load-bearing.

- [ ] **E.5 — `DOCTRINE_PUNCH_LIST.md` Layer 4 row.**
      Flip from `[ ]` to `[x]` with full-path references to phases
      A-E.

- [ ] **E.6 — Operator runbook addendum.**
      Add an `mcc_full` rollout section to
      `docs/runbooks/doctrine-rollout-2026-05.md` covering:
      pre-flight (governance flag default check), three-step rollout
      (linear → mcc → mcc_full), monitoring (`/api/light_cone/*`
      endpoints), rollback procedure (set flag back to `"linear"`).

**Phase E acceptance:** every doctrine doc reflects shipped state;
operator can roll out `mcc_full` from a written guide without
re-reading this plan.

## Stopping conditions

- **Phase A — `enumerate_candidate_heads` non-deterministic** under
  proptest (>0.1% failure rate across 256 random DAG shapes). The
  whole plan rests on validator-determinism; this is the canary.
  Halt and rework.

- **Phase B — `replay_to_head` correctness fails** any of the 6 unit
  tests OR the integration test. State-replay is the biggest risk;
  if the substrate is wrong, no consensus surgery is safe. Halt and
  rework.

- **Phase B memory growth >4× single-fork at 4 concurrent heads.**
  RocksDB snapshot retention is unbounded if not paired correctly
  with eviction. Lower `light_cone_max_concurrent_forks` cap to 2
  or move to copy-on-write state model.

- **Phase C `authoritative_head_converges_across_validators` proptest fails.**
  Validator-determinism canary at the consensus level. Halt — likely
  indicates a non-deterministic input (HashMap iteration order, time-
  based tie-break, etc.) leaked into the dispatch path.

- **Phase D performance >2× Phase 6.3 budget.** Multi-parent
  enumeration is supposed to be cheap; if it's not, the whole approach
  needs reconsideration (smaller fork cap, lazy enumeration, etc.).

## Cross-cutting risks

1. **State-replay is the load-bearing risk.** Phase B is where most
   plans of this shape get stuck. If Phase B reveals architectural
   blockers in `evaporchain-execution` (e.g., the executor mutates
   StateDB without atomic rollback semantics), the whole plan needs
   re-litigation. **This is the highest single risk.**

2. **Wire-format change cost.** Phase C.3 may emit blocks with
   `parents.len() > 1` for the first time. The Light-Cone Phase 2
   wire-format work shipped `serde(default, skip_serializing_if =
   "Vec::is_empty")` so chain-id is preserved when `parents` is
   empty. But once non-empty `parents` appears, `protocol_version`
   must bump. Coordinate with chain-id continuity strategy.

3. **MCC fork-choice cost is per-block.** Today's "mcc" mode walks
   first-parent trajectories at every parent-acceptance check.
   Multi-parent enumeration grows that to N candidate heads per
   block. The Phase A.2 cache mitigates but a poorly-tuned cache
   could flat-line throughput.

4. **Cross-fork equivocation has both honest and malicious modes.**
   Phase 4.3 substrate counts double-precommits without distinguishing
   "honest re-vote after view change" from "malicious double-sign."
   Phase C must NOT escalate the counts-based detection to slashing
   without the certificate-based evidence track (deferred Phase
   4.3d). Risk: false-positive slashing of honest validators during
   network partitions.

5. **Cluster-wide DAG-state agreement under partition.** Same risk as
   Light-Cone Phase 4.4 (commit-cert digest already shipped 2026-05-05
   — pair this MCC plan's authoritative_head with the digest history
   for cross-validator audit).

## Effort estimate

| Phase | Days | Sub-items |
|---|---|---|
| A — Substrate (sibling heads tracking) | 1-2 | 4 |
| B — State replay (biggest risk) | 3-5 | 6 |
| C — Hot-path integration | 3-5 | 6 |
| D — Adversarial + perf tests | 3-5 | 5 |
| E — Doctrine + operator surfaces | 1-2 | 6 |
| **Total** | **11-19 days** | **27** |

Matches the punch-list 1.5-2.5 weeks estimate.

## Rollout governance flag

New flag `parent_acceptance_mode` value: `"mcc_full"`. Three-state
ladder once the plan ships:

```
linear    (default)        — single-line, bit-exact pre-doctrine compat
mcc       (Phase I shipped) — single-line trajectory walk, no enumeration
mcc_full  (this plan)       — full multi-parent enumeration + state replay
```

Flip on testnet first (Phase D.5 soak), monitor via the
`/api/light_cone/*` endpoint family, then governance-vote to flip
chain-wide.

## Progress log

(Updated as phases ship. Most-recent at top.)

- **2026-05-05 (late evening cont'd)** — Phase B.0+ landed:
  `plan_replay_to_head` accessor on TendermintConsensus + `ReplayWalk`
  struct. Pure planning — no execution side. 5 new tests; consensus
  479/0/1. Sets up the API contract Phase B.2 will implement against.

- **2026-05-05 (late evening)** — Phase B.0 landed. Pure DAG primitives:
  `find_lca` and `block_path_from_to` added to
  `evaporchain-light-cone::dag`. 10 new unit tests; light-cone suite
  51 / 0 / 0. Foundation for B.2 (`replay_to_head`) — the executor
  side will compose these to derive replay walks. B.2-B.5 (executor
  integration + RocksDB snapshots + atomic head-switch + memory cap)
  remain for the next focused session.

- **2026-05-05 (evening)** — Phase A landed. A.1 + A.3 + A.4 shipped;
  A.2 deferred to Phase C.
  - A.1 deviated from the original plan (no `sibling_heads` field;
    derived accessor instead). Rationale captured in the doc + tests.
  - `MccForkChoice::enumerate_with_caliber()` is the new substrate
    method; `select_tip` now derives its argmax from it (single
    source of truth). Behaviour bit-for-bit unchanged; refactor only.
  - 5 new tests in `evaporchain-consensus::tendermint::tests`. Full
    consensus suite: 474 passed / 0 failed / 1 ignored.
- **2026-05-05** — plan doc created. Phase 0 (planning) only.

## Cross-references

- `LIGHT_CONE_FULL_DAG_PLAN.md` — sibling plan, already at Phase 7
  (4/4 sub-items shipped 2026-05-05).
- `DOCTRINE_PUNCH_LIST.md` Layer 4 — current MCC fork-choice status.
- `INVENTION_STACK.md §A1.2 T1` — doctrine claim being made
  load-bearing by this plan.
- `crates/evaporchain-mcc/src/lib.rs` — `mcc_choose` math primitive.
- `crates/evaporchain-light-cone/src/concurrency.rs` — antichain +
  digest substrate that this plan builds on top of.
- `crates/evaporchain-consensus/src/fork_choice.rs` — `ForkChoice`
  trait + `MccForkChoice` impl (current single-line walker that
  Phase C extends).
- `crates/evaporchain-consensus/src/tendermint.rs` — voting handlers
  + `start_round` + `create_proposal` that Phase C touches.
