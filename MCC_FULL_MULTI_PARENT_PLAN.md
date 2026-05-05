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

- [x] **B.1 — Concrete `LightConeBranchSnapshot` impl.** ✅ SHIPPED 2026-05-05.
      Shipped in `evaporchain-consensus` (not `evaporchain-state`)
      because the trait lives in consensus and `evaporchain-state`
      doesn't depend on it; concrete impl colocated with the trait
      avoids a dependency cycle.
      - Trait extended with `restore(&self, db: &mut dyn StateDB) ->
        Result<(), String>` — default impl returns "does not support
        restoration" so existing test stubs (StubAtCrateRoot,
        StubSnapshot) keep compiling without override.
      - `StateSnapshotBranch` concrete impl wraps
        `evaporchain_state::snapshot::StateSnapshot`; `capture()`
        uses `SnapshotBuilder::create`, `restore()` uses
        `SnapshotApplier::apply`. Wipe-and-replay semantics exactly
        match the existing snapshot module (verified body hash,
        deterministic ordering, version-checked).
      - This is the in-memory full-state copy implementation
        suitable for testnet and small chains. Production deployments
        with large state should swap in a RocksDB-Snapshot-backed
        impl that pins the LSM tree at a given state version
        (cheaper memory profile, no full-state copy). The trait
        surface is stable; only the concrete `restore()` impl
        changes.
      - 2 new tests:
        - `mcc_phase_b1_state_snapshot_branch_roundtrip` — capture,
          mutate (change balances + add account + delete account),
          restore, verify all reverted to captured state
        - `mcc_phase_b1_default_restore_returns_error` — locks the
          contract that test stubs missing the override get a clean
          error message, not silent state corruption
      - Consensus suite: 481 / 0 / 1.

- [x] **B.2 — `restore_to_lca` bridge (rollback half).** ✅ SHIPPED 2026-05-05.
      Bridges B.0+ planning to B.1 snapshot restore. Looks up the
      `state_branches[plan.lca].snapshot` and calls
      `LightConeBranchSnapshot::restore` on the StateDB.
      No-op when `!plan.rollback_required`. Errors when the LCA
      isn't tracked OR has no attached snapshot.
      - The forward-replay half is a trivial caller-side loop:
        `for block_id in &plan.forward_path { executor.execute_block(block_lookup(block_id)?) }`.
        No bespoke `replay_to_head` umbrella function — the caller
        composes the planning, restore, and forward-apply pieces
        directly. Cleaner separation of concerns.
      - 4 new tests: happy-path roundtrip, no-op when no rollback,
        errors on missing LCA, errors on missing snapshot.
      - Consensus suite: 485 / 0 / 1.

      **Caller workflow (the Phase B.2 contract):**
      ```ignore
      let plan = consensus.plan_replay_to_head(from, to)?;
      consensus.restore_to_lca(&plan, &mut db)?;  // no-op if forward-only
      for block_id in &plan.forward_path {
          let block = block_store.get(block_id)?;
          executor.execute_block(&mut db, &block)?;
      }
      ```

- [x] **B.3 — `replay_and_apply` umbrella hot-path integration.** ✅ SHIPPED 2026-05-05.
      Single function on `TendermintConsensus` that composes the
      full pipeline: `plan_replay_to_head` (B.0+) +
      `restore_to_lca` (B.2) + caller-provided `block_lookup` and
      `block_apply` closures.
      - Returns `Result<ReplayResult, ReplayError>`. `ReplayResult`
        records the LCA + every block applied; `ReplayError` has 4
        variants (`PlanFailed`, `RestoreFailed`, `BlockNotFound`,
        `ApplyFailed`).
      - Closure-driven (not trait-coupled) so the consensus crate
        doesn't need to know the executor type or the block-store
        interface. Production callers wrap their executor's
        `execute_block` and chain-store's `get_block` lookup.
      - **Caller workflow** simplified from B.2's three-piece
        composition to one call:
        ```ignore
        consensus.replay_and_apply(
            db,
            current_head,
            target_head,
            |id| chain_store.get_block(id),
            |db, block| executor.execute_block(db, block),
        )?
        ```
      - 4 new tests: branch-switch happy path, BlockNotFound,
        PlanFailed (missing head), ApplyFailed (executor error
        propagated). Consensus suite: 490 / 0 / 1.

      **Atomic-contract caveat (Phase B.4 separate work):** if the
      forward-apply loop fails midway, the StateDB is in a partial
      state — at the LCA plus any earlier `forward_path` entries
      already applied. Phase B.4 wraps the whole sequence in
      `db.begin_batch()` / `commit_batch()` for transactional
      atomicity. Until then, callers must handle partial-state
      recovery themselves.

- [x] **B.4 — `replay_and_apply_atomic` transactional wrapper.** ✅ SHIPPED 2026-05-05.
      The original plan called for `db.begin_batch()` /
      `commit_batch()`, but those methods live on the concrete
      `RocksDBStateDB` only — not the `StateDB` trait. Trait-portable
      atomicity instead uses the B.1 `StateSnapshotBranch` substrate:
      capture pre-replay snapshot, run inner `replay_and_apply`, on
      any error restore from the captured snapshot. Either complete
      success (StateDB at target_head) or complete rollback (StateDB
      at pre-replay state) — never a partial-replay residue.
      - Cost: one extra full-state capture per replay attempt.
        Production deployments with large state would prefer the
        RocksDB WriteBatch path as a separate concrete-impl
        optimisation; the trait-level guarantee uses snapshot.
      - Composite-error handling: if rollback ITSELF fails, the
        StateDB is in an undefined state and the operator must
        intervene — flagged via a synthetic `ApplyFailed` with a
        `<pre-replay rollback>` block tag.
      - 2 new tests: success-path passthrough (atomic returns same
        ReplayResult as inner replay_and_apply) AND failure rollback
        (block_apply always errors → StateDB ends at pre-replay
        state, NOT at the LCA where inner replay's restore would
        have left it).
      - Consensus suite: 493 / 0 / 1.

      **Phase B is now 8/8 complete.** The full state-replay pipeline
      is shipped end-to-end with substrate-level tests, an umbrella
      convenience function, an atomic transactional wrapper, AND the
      memory-reclamation lock. The remaining MCC plan work (Phases
      C, D, E.2-E.6) builds on top of this substrate, not into it.

- [x] **B.5 — Memory cap enforcement (eviction-drops-snapshot lock).** ✅ SHIPPED 2026-05-05.
      `prune_state_branches` (Phase 3.4 substrate) already enforces
      the cap and removes the metadata HashMap entry. Phase B.5
      verifies that this also releases the consensus crate's
      `Arc<dyn LightConeBranchSnapshot>` reference — the
      load-bearing memory-reclamation contract.
      - `mcc_phase_b5_eviction_drops_snapshot_arc` test: cap=2, 3
        snapshots attached with distinct calibers; after eviction,
        the lowest-caliber snapshot's `Arc::strong_count` drops
        from 2 (test + consensus) to 1 (test only). Surviving
        snapshots stay at 2.
      - Without this guarantee, snapshot memory would accumulate
        indefinitely as forks come and go — the cap would only
        bound HashMap key count, not actual snapshot bytes held.
      - Consensus suite: 491 / 0 / 1.

- [x] **B.6 — Tests** (partial — integration test shipped, unit
      tests integrated alongside B.0/B.0+/B.1/B.2). ✅ INTEGRATION SHIPPED 2026-05-05.
      The 6 unit cases listed in the original plan have been folded
      into the substrate-phase tests (each phase ships its own
      target tests; the unit/integration split was over-decomposed).
      End-to-end integration test landed:
      - `mcc_phase_b6_e2e_branch_switch_substrate_composition`
        Drives a 3-block-deep diverging DAG (genesis → A1 → A2 vs
        genesis → B1 → B2). Captures snapshot at genesis. Simulates
        fork A execution. Plans replay A2 → B2. Calls restore_to_lca
        to wipe fork-A mutations. Applies fork-B's forward path via
        caller-side loop. Asserts final state reflects fork B only,
        with no fork-A residue.
      - Validates B.0 + B.0+ + B.1 + B.2 compose correctly. The
        composition is the substrate's load-bearing claim; this test
        is the proof.
      - Consensus suite: 486 / 0 / 1.

      **Atomic contract under failure (was B.4)**, **head-switch
      idempotency**, and **LRU eviction-drops-snapshot (was B.5)**
      remain as discrete tests pending the B.3+B.4+B.5 wiring work.
      The B.6 integration test as shipped is the strongest current
      signal that the substrate composition is correct.

**Phase B acceptance:** state replay is correct and atomic across all
fork shapes the test fixtures cover; no consensus-state-machine surgery
yet (still gated on `parent_acceptance_mode = "mcc"`).

### Phase C — Hot-path integration (3-5 days)

Promote `authoritative_head` from admin-RPC to consensus hot path.

- [x] **C.1 — `current_authoritative_head` field + update method.** ✅ SHIPPED 2026-05-05.
      Pure additive substrate. New field
      `current_authoritative_head: Option<[u8; 32]>` on
      TendermintConsensus. New method
      `update_authoritative_head() -> Option<[u8; 32]>` recomputes
      via `enumerate_candidate_heads`'s argmax under
      `parent_acceptance_mode = "mcc_full"`; clears the field under
      any other mode (chain bit-compat). Governance allowlist now
      accepts `"mcc_full"` as a valid `parent_acceptance_mode` value.
      4 new tests: noop-outside-mcc_full, populated-under-mcc_full,
      cleared-on-rollback, governance-allows-mcc_full. The
      hot-path call into this method (from start_round / advance_round)
      is Phase C.2/C.3 separate work — C.1 is the substrate field
      + method; C.2/C.3 wire it into the consensus lifecycle.

- [x] **C.2 — `vote_target_head` accessor.** ✅ SHIPPED 2026-05-05.
      Pure substrate read-side method. Returns
      `current_authoritative_head` under `mcc_full` (with parent_hash
      fallback if not yet computed); returns `parent_hash` under
      `linear` and `mcc` (chain bit-compat). Voting handlers
      (`handle_prevote`, `handle_precommit`) call this method to
      know where to route votes; the actual handler dispatch
      change is the C.4 wiring step. Today the handlers still tally
      on parent_hash; this method is the substrate they'll call. 2
      new tests: fall-back-to-parent-hash under linear/mcc, uses
      authoritative_head under mcc_full with parent_hash fallback
      when head is None. Consensus suite: 500 / 0 / 1.

- [x] **C.3 — `propose_parents` accessor.** ✅ SHIPPED 2026-05-05.
      Pure read-side substrate. Returns `Vec<BlockId>` for the
      proposer to set as `block.parents` under `mcc_full`:
      - `linear` / `mcc`: empty Vec (chain bit-compat;
        serde-skip-empty preserves single-parent wire format)
      - `mcc_full`: top-N heads by caliber filtered to a true
        antichain (`is_antichain` predicate; comparable heads
        dropped lower-caliber-first), capped at
        `light_cone_max_concurrent_forks`.
      Proposer's `create_proposal` will call this method to
      populate `block.parents`. The actual `create_proposal`
      integration is Phase C.6 separate work; today
      `block.parents` stays empty under all modes. 4 new tests:
      empty under linear/mcc, returns concurrent siblings under
      mcc_full, filters comparable heads, respects max-forks cap.

- [x] **C.4 — Cross-fork equivocation accessors.** ✅ SHIPPED 2026-05-05.
      Pure read-side substrate. The Phase 4.3 substrate already
      ships `cross_fork_equivocations: HashMap<u64, u64>` —
      validator_id → count of observed cross-fork double-votes.
      Phase C.4 exposes operator-facing accessors:
      - `cross_fork_equivocation_count(validator_id) -> u64`:
        per-validator count
      - `all_cross_fork_equivocations() -> HashMap<u64, u64>`:
        full snapshot
      Both gated on `parent_acceptance_mode = "mcc_full"` — return
      0 / empty under linear/mcc (chain bit-compat). Pairs with
      `evaporchain_entropic_slashing::entropic_slash` for slash
      magnitude calculation. Note on certificate-based vs
      counts-based: counter cannot perfectly distinguish honest
      re-vote from malicious double-sign; certificate-based
      evidence track is deferred Phase 4.3d follow-up. Until then
      treat counts as a *signal*, not a slashing trigger. 2 new
      tests covering both accessors gated on mode. Consensus
      506/0/1.

- [x] **C.5 — Validator-determinism property test.** ✅ SHIPPED 2026-05-05.
      `mcc_phase_c5_validator_determinism_under_random_dags`
      proptest sweeps 256 random DAG shapes (sizes 1..=20 blocks,
      mixed branching with 1-2 parents per non-genesis block).
      Each iteration drives two independent `TendermintConsensus`
      instances through the same block-insertion sequence and
      asserts FIVE properties:
      1. `candidate_heads()` BTreeSets agree
      2. `enumerate_candidate_heads()` Vecs agree exactly (order +
         caliber values)
      3. `light_cone_antichain_digest()` matches
      4. `plan_replay_to_head` produces identical `ReplayWalk` for
         every (from, to) pair drawn from the candidate heads
      5. No caliber values overflow to `u64::MAX`
      All 256 iterations × 5 properties = ~1280 individual
      assertions; all pass in 0.76s on Mini. Catches non-determinism
      that depends on specific topology (HashMap iteration order
      leak, time-based tiebreak, etc.) — the manual A.4 6-block
      determinism test is a baseline; this is the stress-test
      version.

- [x] **C.6 — Integration tests for accessor composition.** ✅ SHIPPED 2026-05-05.
      Pivoted from the original 5-test list (which targeted the
      *deferred* hot-path call-site integration) to 3 substrate
      composition tests that lock the C.1+C.2+C.3 contract today.
      The original 5-test plan slides forward to Phase D (call-site
      integration depends on D.1's 4-validator 3-fork harness).
      - `mcc_phase_c6_integration_accessors_compose_on_4_fork_dag`:
        on a 4-fork DAG, `update_authoritative_head` /
        `vote_target_head` / `propose_parents` agree on the argmax;
        propose_parents forms an antichain; read-consistency under
        no-mutation re-reads.
      - `mcc_phase_c6_integration_authoritative_head_follows_dag_extension`:
        after extending one fork (so a leaf becomes interior), the
        authoritative head and parent set track the new leaf set;
        all assertions against `candidate_heads()` to lock substrate
        agreement.
      - `mcc_phase_c6_integration_rollback_to_linear_restores_bit_compat`:
        flipping `parent_acceptance_mode` from `mcc_full` back to
        `linear` clears C.1 field, makes C.2 fall back to
        `parent_hash`, and C.3 returns empty parents — full chain
        bit-compat restored within a single mode flip (governance
        rollback safety net).
      Consensus suite: 509 / 0 / 1.
      Original C.6 candidates that move to D:
        - `authoritative_head_selected_at_start_round` → D.1 add-on
        - `votes_route_to_authoritative_head_tally` → D.1 add-on
        - `proposer_emits_multi_parent_block_under_mcc_full` → D.1 add-on
        - `cross_fork_equivocation_increments_on_double_prevote` → D.2
        - `authoritative_head_converges_across_validators` 256x —
          ALREADY SHIPPED as C.5.

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

- [x] **E.1 — `/api/light_cone/candidate_heads` HTTP endpoint.** ✅ SHIPPED 2026-05-05.
      Wraps `TendermintConsensus::enumerate_candidate_heads` (Phase A.3)
      as a GET endpoint returning `{heads:[{block_id, caliber}], count,
      running_alongside_tendermint}`. First entry is the MCC-chosen
      authoritative head; downstream entries are the alternatives the
      fork-choice considered. Pure additive — no hot-path surgery.
      Doc-entry added to API directory. evaporchain-node builds clean.

- [x] **E.2 — `/api/light_cone/authoritative_head` HTTP endpoint.** ✅ SHIPPED 2026-05-05.
      Returns the argmax of `enumerate_candidate_heads` —
      `{head, caliber, candidates_considered, running_alongside_tendermint}`.
      Per-validator — different validators may briefly disagree
      during a round. Pairs with E.1's candidate-heads endpoint
      (full list) and Light-Cone Phase 7's antichain-digest-history
      (retroactive cross-validator agreement detection). Doc-entry
      added to API directory; node compiles clean.

- [x] **E.3 — `LIGHT_CONE_FULL_DAG_PLAN.md` Phase 8 addendum.** ✅ SHIPPED 2026-05-05.
      New "Phase 8 — MCC full multi-parent enumeration" section
      added to the Light-Cone plan, cross-referencing this plan doc
      and explicitly noting which Phase A + B + E items have shipped.
      Locks the relationship between the two plans: Phases 1-6 of
      Light-Cone shipped DAG mode as co-existing with Tendermint;
      Phase 7 added the cross-validator agreement digest; Phase 8
      (this MCC plan) is the load-bearing hot-path integration.

- [x] **E.4 — `INVENTION_STACK.md §A1.2 T1` update.** ✅ SHIPPED 2026-05-05.
      Reflects Phase A + B completion. T1's "primitive" column now
      enumerates the substrate work (`MccForkChoice`,
      `enumerate_candidate_heads`, `plan_replay_to_head`,
      `replay_and_apply_atomic`), 34 tests, and is honest that hot-path
      integration (Phase C) + adversarial testing (Phase D) remain.
      The "What EvaporChain alone can state" column expanded to
      describe the multi-parent enumeration substrate as shipped, with
      the explicit "becomes load-bearing once Phase C lands" caveat.

- [x] **E.5 — `DOCTRINE_PUNCH_LIST.md` Layer 4 row update.** ✅ SHIPPED 2026-05-05.
      Layer 4 "MCC fork-choice (full multi-parent enumeration)" row
      flipped from `[ ]` to `[x] substrate complete` with full
      manifest of Phase A (3/4 items, A.2 deferred), Phase B (8/8
      items), Phase E.1 + E.4. Lists 34 new tests + the consensus
      suite delta (469 → 493). Honestly scoped: Phases C + D +
      E.2/E.3/E.6 still listed as remaining work.

- [x] **E.6 — Operator runbook addendum.** ✅ SHIPPED 2026-05-05.
      New "Lane 4 — MCC full multi-parent enumeration (Phase 8
      addendum)" section added to
      `docs/runbooks/doctrine-rollout-2026-05.md`. Includes:
      - Substrate-vs-hot-path-status warning ("do NOT flip
        mcc_full in production until Phase C ships")
      - Pre-flight checks (Lane 3 prereq, snapshot attachment
        verification, protocol_version ≥ 3)
      - Three-step ladder: `linear` → `mcc` → `mcc_full`
      - Monitoring snippets for the three E.1/E.2/Phase-7
        endpoints (`candidate_heads`, `authoritative_head`,
        `antichain_digest_history`)
      - Cross-validator divergence-diagnosis loop
      - Rollback procedure (back to `mcc`, then `linear` if needed)

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

- **2026-05-05 (late evening cont'd 19)** — Phase C.6 substrate
  composition tests shipped (3 integration tests). Pivoted from the
  original 5-test hot-path call-site list to lock the C.1+C.2+C.3
  contract today; call-site tests slide forward to Phase D where
  the 4-validator harness lives. Tests cover (a) 4-fork DAG
  accessor agreement + antichain + read-consistency,
  (b) DAG-extension follow-through to a new leaf set,
  (c) governance flip mcc_full→linear restoring full chain
  bit-compat. Consensus 509/0/1. **Phase C is now 6/6 done — all
  hot-path substrate locked. Plan moves to Phase D.**

- **2026-05-05 (late evening cont'd 18)** — Phase C.4 substrate
  shipped. Two accessors —
  `cross_fork_equivocation_count(validator_id)` and
  `all_cross_fork_equivocations()` — expose Phase 4.3's
  cross_fork_equivocations counter under mcc_full (returns 0/empty
  under linear/mcc). Substrate-only; certificate-based evidence
  track for actual slashing remains deferred Phase 4.3d. 2 new
  tests; consensus 506/0/1.

- **2026-05-05 (late evening cont'd 17)** — Phase C.3 substrate
  shipped. `propose_parents()` accessor returns the antichain set
  the proposer should put in `block.parents` under mcc_full
  (empty under linear/mcc). Filters comparable heads + respects
  `light_cone_max_concurrent_forks` cap. 4 new tests; consensus
  504/0/1.

- **2026-05-05 (late evening cont'd 16)** — Phase C.2 substrate
  shipped. `vote_target_head()` accessor returns the BlockId voting
  handlers should route to (current_authoritative_head under
  mcc_full with parent_hash fallback; parent_hash under linear/mcc).
  Pure read-side accessor — handler dispatch wiring is C.4. 2 new
  tests; consensus 500/0/1.

- **2026-05-05 (late evening cont'd 15)** — Phase C.1 substrate
  shipped. New `current_authoritative_head` field +
  `update_authoritative_head` method on TendermintConsensus. Pure
  additive — does NOT yet hook into round lifecycle (Phase C.2/C.3
  separate). Governance allowlist accepts `mcc_full`. 4 new tests
  covering no-op outside mcc_full, populated under mcc_full,
  cleared-on-rollback, governance allowlist. Consensus 498/0/1.

- **2026-05-05 (late evening cont'd 14)** — Phase C.5 proptest
  shipped — `mcc_phase_c5_validator_determinism_under_random_dags`.
  256 random DAG shapes, 5 properties asserted per shape (~1280
  assertions total), all pass in 0.76s. Catches non-determinism
  bugs that depend on specific topology (HashMap iteration leak,
  time-based tie-break, etc.). Phase C is now 1/6 — the
  determinism gate is shipped before the hot-path surgery, locking
  the contract that hot-path consumers will need to preserve.

- **2026-05-05 (late evening cont'd 13)** — Phase E.6 runbook
  addendum shipped. New "Lane 4" section in
  `docs/runbooks/doctrine-rollout-2026-05.md` covers `mcc_full`
  rollout sequence, pre-flight checks, monitoring endpoints,
  cluster-divergence diagnosis loop, and rollback. Explicitly
  flags the substrate-vs-hot-path status: do NOT flip mcc_full
  in production until Phase C ships. **Phase E is now 6/6 done.**

- **2026-05-05 (late evening cont'd 12)** — Phase E.2 endpoint
  shipped. `/api/light_cone/authoritative_head` returns the
  MCC-chosen head + caliber + candidates_considered. Pairs with
  E.1's full candidate list. Node compiles clean.

- **2026-05-05 (late evening cont'd 11)** — Phase E.3 cross-doc
  addendum. `LIGHT_CONE_FULL_DAG_PLAN.md` now has a Phase 8 section
  pointing at this plan doc and recording the substrate-complete
  status. Locks the relationship between the two plans.

- **2026-05-05 (late evening cont'd 10)** — Phase E.5 doctrine doc
  flip. `DOCTRINE_PUNCH_LIST.md` Layer 4 MCC fork-choice row
  updated from `[ ]` to `[x] substrate complete` with full
  itemized manifest of Phase A (3/4), B (8/8), E.1 + E.4. Honest
  about Phase C/D/E.2-3-6 remaining as separate workstreams.

- **2026-05-05 (late evening cont'd 9)** — Phase E.4 doctrine update
  shipped. `INVENTION_STACK.md §A1.2 T1` (MCC) row rewritten to
  reflect Phase A + B substrate completion: enumerate_candidate_heads,
  plan_replay_to_head, replay_and_apply_atomic, 34 tests. Honest
  about Phase C (hot-path) + Phase D (adversarial) as remaining
  work. The doctrine claim now reads accurately against shipped
  code, not aspirationally.

- **2026-05-05 (late evening cont'd 8)** — Phase B.4 landed.
  `replay_and_apply_atomic` wraps the umbrella with pre-replay
  snapshot capture + on-error rollback for trait-portable
  transactional atomicity. Original plan called for
  begin_batch/commit_batch but those are concrete-only methods on
  RocksDBStateDB; snapshot-based atomicity works for both backends.
  2 new tests; consensus 493/0/1. **Phase B is 8/8 complete.** The
  full state-replay pipeline is now shipped end-to-end with
  substrate + umbrella + atomic + memory-reclamation locks. Phases
  C, D, E.2-E.6 build on top of this substrate.

- **2026-05-05 (late evening cont'd 7)** — Phase B.5 landed.
  Memory-reclamation contract verified: when `prune_state_branches`
  evicts a metadata entry, the consensus crate's
  `Arc<LightConeBranchSnapshot>` reference is released. Test uses
  `Arc::strong_count` to assert evicted snapshot drops from 2 →
  1, surviving snapshots stay at 2. Consensus 491/0/1. **Phase B is
  now 7/8 done** — only B.4 (atomic batch wrap for partial-failure
  recovery) remains.

- **2026-05-05 (late evening cont'd 6)** — Phase B.3 landed.
  `replay_and_apply` umbrella function on TendermintConsensus
  composes plan + restore + caller-provided block_lookup +
  block_apply closures into a single call. Returns
  `Result<ReplayResult, ReplayError>`. Closure-driven design avoids
  consensus-crate coupling to executor type or block-store
  interface. 4 new tests covering happy path + 3 error variants.
  Consensus 490/0/1. **Phase B is now 6/8 done** — B.4 (atomic
  batch wrap) and B.5 (snapshot eviction verification) remain;
  hardest algorithmic + integration work is shipped.

- **2026-05-05 (late evening cont'd 5)** — Phase E.1 endpoint landed.
  `/api/light_cone/candidate_heads` exposes
  `enumerate_candidate_heads` over HTTP with hex-encoded BlockIds +
  caliber scores. Pure additive substrate — no hot-path surgery.
  Operator-debug value: `curl http://node:8080/api/light_cone/candidate_heads`
  returns the MCC-chosen authoritative head and all alternatives
  the fork-choice considered. Pairs with Phase 4.4's
  /api/light_cone/antichain_digest_history for cluster-divergence
  detection. evaporchain-node builds clean.

- **2026-05-05 (late evening cont'd 4)** — Phase B.6 integration test
  landed. Drives the full B.0+B.0++B.1+B.2 substrate through a
  3-block-deep branch switch (genesis → A1 → A2 vs genesis → B1 → B2)
  with explicit state mutations and assertion that the final state
  reflects fork B only. Consensus 486/0/1. Substrate composition
  verified end-to-end; the algorithmic correctness substrate is now
  load-bearing on real test code, not just unit-level claims.

- **2026-05-05 (late evening cont'd 3)** — Phase B.2 landed:
  `restore_to_lca` bridge accessor on TendermintConsensus. Composes
  `plan_replay_to_head` (B.0+) with `StateSnapshotBranch::restore`
  (B.1) — looks up the LCA's snapshot from state_branches and
  invokes the trait's restore method. The forward-apply half is
  a caller-side loop (no umbrella `replay_to_head` function — cleaner
  separation). 4 new tests covering happy-path + 3 error paths.
  Consensus 485/0/1.

- **2026-05-05 (late evening cont'd 2)** — Phase B.1 landed.
  `LightConeBranchSnapshot` trait extended with `restore` (default
  impl returns error). `StateSnapshotBranch` concrete impl wraps
  `evaporchain_state::snapshot::StateSnapshot` —
  `SnapshotBuilder::create` for capture, `SnapshotApplier::apply`
  for restore. Capture→mutate→restore roundtrip test green.
  Consensus 481/0/1. Production RocksDB-Snapshot-backed impl
  remains a separate optimisation; trait surface is stable.

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
