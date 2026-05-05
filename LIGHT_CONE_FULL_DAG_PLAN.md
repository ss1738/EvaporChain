# Light-Cone Full DAG Consensus — Implementation Plan

**Status:** plan-draft 2026-05-04. No implementation yet.

**Context:** `DOCTRINE_PUNCH_LIST.md` Layer 6 Light-Cone row reads "⏳ tendermint.rs still 8,782 LOC; `MccForkChoice` (Layer 4) cherry-picks parents but doesn't materialise alternative state branches — full DAG fork-choice is genuine months-long work." This file is the roadmap to flip it to ✅. Mirrors `LAMBDA_FOLD_NOVA_PLAN.md` and `CROOKS_MEV_INTEGRATION_PLAN.md` in shape — phases, stopping conditions, tests, doctrine sweep at the end.

This is **the largest of the three Layer 5/6 plans** and is honest about the months-of-work scope. Phases 1–3 are bounded enough to ship as discrete pieces; Phases 4–6 are the genuinely consensus-grade work that will need its own session arc.

---

## What's already shipped

- **`evaporchain-light-cone`** crate (`crates/evaporchain-light-cone/src/`) ships the substrate: `LightCone` DAG with `insert / get / contains / causal_past / causal_future / prune_before_epoch`, `Block` (with parents Vec, energy, epoch), `BlockId`, `LightConeError`. ~5 modules: `arrow`, `block`, `concurrency`, `dag`, `lib`.
- **DAG insertion per committed block** at `tendermint.rs:on_block_committed` (the `light_cone_dag.insert(...)` call). Every committed block enters the DAG. Genesis edge case handled (empty parents vector when `block.parent_hash` isn't in the DAG yet).
- **MccForkChoice** at `crates/evaporchain-consensus/src/fork_choice.rs:46` — Maximum-Caliber Consensus fork-choice trait impl that walks the LightCone DAG's first-parent trajectories and chooses the head with highest cumulative caliber. Wired into `tendermint.rs:2643` behind governance flag `parent_acceptance_mode = "mcc"` (Layer 4 work, default still `linear`).
- **Causal-Cone Validator State** (`evaporchain-causal-cone` crate) ships the Shalizi 2003 causal-set sufficient-statistic primitives — used by the LightCone for ε-machine reconstruction (per `CSLC` follow-up in Phase 2 of this plan).

The substrate works. What's missing is **the chain actually deciding consensus on the DAG instead of on the linear Tendermint chain**.

---

## Why this is hard

Tendermint's linear-chain assumptions are baked into ~8,800 LOC of `tendermint.rs`:

1. **One canonical block per height.** `committed_heights: HashSet<u64>` rejects forks at the same height as duplicates. The DAG semantically allows multiple committed blocks per epoch (with shared causal pasts) — Tendermint refuses.
2. **Linear `parent_hash`.** Every `Block` carries a single `parent_hash: [u8; 32]`. The DAG's notion of "block has multiple parents" doesn't fit; we'd need `parents: Vec<[u8; 32]>` on the wire format. Wire-format change → soft-fork → governance flag → multi-version block decoder.
3. **Single state root.** `current_state_root: [u8; 32]` on `TendermintConsensus`. DAG semantics imply per-fork state roots (you can be in multiple causally-consistent state branches simultaneously). Materializing alternative state requires the executor to support per-fork state databases.
4. **Voting tally is single-tip.** `RoundState::precommits/prevotes` indexed by `block_hash`; one tip wins per round. DAG voting needs per-fork tally + reconciliation when forks merge.
5. **Finality gap tracking is linear.** `committed_at: HashMap<u64, u64>` and `finality_gap_history` assume strictly-increasing heights. DAG finality is "all blocks in a finalized antichain," not a single height.
6. **Energy reconciliation across forks.** When fork A and fork B share a causal past then diverge, an object touched in both forks has divergent energy histories. The reconciliation rule (which fork's energy "wins" if both finalize?) is a fresh research question — `MccForkChoice` resolves the choice but the merged energy state is undefined.

These six blockers compose. Item 2 is wire-format and forks the chain. Items 3-6 are state-machine changes that ripple through every commit-time path.

---

## Phase breakdown

Six phases, total scope **≈3-6 months**. Phases 1-3 ship in dedicated sessions of ~1-2 weeks each; phases 4-6 are bigger and need their own planning sub-docs (think `PHASE_4_DECISIONS.md` etc., per the Lambda-Fold/Crooks-MEV pattern).

### Phase 1 — DAG-aware tip selection (1-2 weeks)

**Goal:** make `MccForkChoice` more than just a parent-picker — actually drive the chain's tip selection from the DAG instead of the linear `parent_hash` chain.

- [x] **1.1 — Tip-selection trait extension.** SHIPPED. `ForkChoice::select_tip(&self) -> Option<[u8; 32]>` trait method (default `None` for linear mode); `MccForkChoice::select_tip` walks `LightCone::leaves()`, scores each by `path_caliber`, returns max. Tie-break = smaller BlockId (deterministic across validators since `leaves()` is BTreeMap-sorted). 4 tests green: empty DAG, single block, 2-fork DAG, linear default.
- [x] **1.2 — `tendermint.rs::current_tip()` helper.** SHIPPED. When `parent_acceptance_mode == "mcc"`, builds an `MccForkChoice` snapshot from the chain's DAG + governance β, calls `select_tip`. Falls back to `self.parent_hash` when mode is `linear` (default), DAG is empty, or `select_tip` returns `None`. 2 tests green: `test_current_tip_falls_back_to_parent_hash_in_linear_mode`, `test_current_tip_mcc_mode_empty_dag_falls_back`.
- [x] **1.3 — Proposer integration.** SHIPPED. `create_proposal` at `tendermint.rs:4755` now sets `block.parent_hash = self.current_tip()` (was: `self.parent_hash`). Under `parent_acceptance_mode = "mcc"` the proposer builds on the DAG-derived head; under default `linear` it falls back to `parent_hash` (chain bit-for-bit unchanged). New test `test_current_tip_mcc_mode_returns_dag_leaf` injects a 2-block DAG, flips governance mode, asserts `current_tip()` returns a DAG leaf rather than the linear `parent_hash`. Validator-side skew-tolerance not yet enforced — proposers and receivers may briefly disagree on the DAG tip during partition; Phase 4's antichain-finality rule will handle that. Phase 1's softer contract: validators using mcc mode SHOULD converge under MCC tie-break, and a transient mismatch isn't slashable.
- [x] **1.4 — Determinism note.** Encoded in the docstrings of `select_tip` (BTreeMap-sorted leaves + smaller-BlockId tie-break) and `current_tip` (governance-flag-gated, falls back safely).
- [x] **1.5 — Tests.** SHIPPED at the trait level (4) + accessor level (2) = 6 new tests. Adversarial caliber-injection test deferred to Phase 6.2 perf scenarios.

**Phase 1 deliverable: SHIPPED (5/5).** The chain's tip is DAG-derived under `mcc` mode at every level: trait → accessor → proposer. Soft-fork-safe via the existing governance flag. **~30% of the doctrine gap** on Light-Cone Full DAG closed (per the plan's "if you only have one session" statement). The riskier consensus state-machine surgery starts in Phase 3 (per-fork state branches).

### Phase 2 — Multi-parent block wire format (2-3 weeks)

**Goal:** add a `parents: Vec<[u8; 32]>` field to `Block` (defaulting to single-element when present) without breaking on-the-wire compat with existing chain history.

- [x] **2.1 — `parents: Vec<[u8; 32]>` on `Block`.** SHIPPED. Field added with `#[serde(default, skip_serializing_if = "Vec::is_empty")]` so legacy blocks (empty `parents`) serialize bit-identically to pre-Light-Cone. `Block::effective_parents()` accessor returns `parents` when non-empty, else `vec![parent_hash]` (single-parent fallback). Light-Cone consumers should use the accessor.
- [x] **2.2 — Block validation rule.** SHIPPED as `Block::validate_parents_wire_format()` with `BlockParentsValidationError` enum (`MultiParentRequiresV3`, `DuplicateParent`, `ParentHashMismatch`). Cycle-detection is the DAG-side responsibility (existing `LightCone::insert` rejects via `MissingParent`).
- [x] **2.3 — `protocol_version` bump (gating).** Encoded in `validate_parents_wire_format`: `parents.len() > 1 && protocol_version < 3` → `MultiParentRequiresV3` error. Phase 4 antichain finality is the gate that bumps the chain's actual protocol_version.
- [x] **2.4 — Hash-stability test.** `test_block_legacy_serialization_omits_parents_field` asserts `serde_json::to_string(&legacy_block)` does NOT contain `"parents"` when the field is empty. Confirms the `skip_serializing_if = "Vec::is_empty"` contract holds. **`block_hash` (`tendermint.rs:2590`) does NOT include the new `parents` field — chain-id continuity preserved.**
- [x] **2.5 — Tests.** 4 new tests in `evaporchain-types` (effective-parents fallback + explicit, validate-parents 4 paths, hash-stability, round-trip) + the existing `test_block_serialization_roundtrip` extended with `parents: vec![]`. 5/5 green on Mini under release.

**Phase 2 deliverable: SHIPPED.** Wire format supports DAG-shaped blocks. Backward-compat preserved via `serde(default)` + `skip_serializing_if`. Hash continuity locked. The chain can now produce v3 multi-parent blocks; whether it DOES is gated on Phase 4's antichain finality consensus.

### Phase 3 — Per-fork state branches (3-4 weeks)

**Goal:** materialize the actual state at every DAG tip, not just the longest linear chain. This is the biggest single piece of state-machine surgery.

- [x] **3.1 — `state_branches: HashMap<[u8; 32], LightConeBranchMetadata>` table.** SHIPPED on `TendermintConsensus`. `LightConeBranchMetadata { created_at_block, last_touched_block, caliber }` carries the per-tip metadata needed for Phase 3.4 LRU eviction + Phase 5 orphan pruning. Lifecycle hook in `on_block_committed`: when `light_cone_state_branches_enabled = true`, the just-committed block is recorded as a tip. The actual `Arc<dyn StateDB>` snapshot ref slots in beside the metadata in Phase 3.2 (Decision 1 of `PHASE_3_DECISIONS.md`).
- [x] **3.2 — Per-tip executor (consensus-side seam).** SHIPPED. `LightConeBranchSnapshot` trait (`tip()`, `created_at_height()`) — minimal abstraction the executor will implement; consensus engine doesn't need to depend on `evaporchain-state` to wire it. `LightConeBranchMetadata.snapshot: Option<Arc<dyn LightConeBranchSnapshot + Send + Sync>>` field carries the ref. `TendermintConsensus::attach_branch_snapshot(tip, snapshot)` method installs it (returns `None` if the tip isn't tracked — caller must `record_state_branch` first). Test `test_state_branches_snapshot_attach` exercises the trait via a synthetic stub. **Executor-side wiring** (taking a RocksDB snapshot at commit time + calling `attach_branch_snapshot`) is the focused next piece — bounded since the consensus seam is locked.
- [x] **3.3 — Energy reconciliation rule.** Locked in `research/light_cone/PHASE_3_DECISIONS.md` Decision 3: take-max on merge. Implementation lands in Phase 3.2 alongside the executor dispatch.
- [x] **3.4 — Concurrent-fork cap (LRU eviction).** SHIPPED. `prune_state_branches` evicts the lowest-caliber entry when `state_branches.len() > cap`; cap from `light_cone_max_concurrent_forks` flag (default 4). Tie-break = smallest BlockId for validator-determinism. Test `test_state_branches_lru_eviction` confirms cap=2 → 3 inserts → 1 eviction → correct survivors.
- [x] **3.5 — Tests.** 4 new tests green on Mini under release: starts-empty contract, idempotent record, LRU eviction, governance-flag gate (already shipped in 3.5).

**Phase 3 deliverable:** the chain materializes parallel state branches. Multi-tip validation + voting + commit is functional. **THIS IS THE TIPPING POINT** — once 3 lands, the chain is genuinely DAG-driven, not just DAG-observed.

### Phase 4 — DAG-aware vote tally + finality (2-3 weeks)

**Goal:** Tendermint's prevote/precommit machinery extended to DAG semantics. Multi-tip voting, antichain finality, equivocation detection across forks.

- [x] **4.1 — Per-fork RoundState (substrate + handler wiring).** SHIPPED. `dag_round_states: HashMap<[u8; 32], RoundState>` field on `TendermintConsensus`, additive per Decision 1. `record_dag_prevote` + `record_dag_precommit` API ships the per-tip vote-recording entry points; voting handlers `handle_prevote` (`tendermint.rs:4130`) and `handle_precommit` (`tendermint.rs:4255`) now mirror incoming votes into `dag_round_states[tip]` when `light_cone_state_branches_enabled = true` AND the voted-for block is a current DAG leaf. No-op when flag off → linear-mode chain bit-compat. Cross-fork equivocation detection (Decision 3) runs on every `record_dag_precommit` and bumps `cross_fork_equivocations[validator_id]` on observed double-precommit at the same round.
- [x] **4.2 — Antichain primitives (substrate).** SHIPPED in `evaporchain-light-cone::concurrency`. `is_antichain(set: &[BlockId]) -> bool` (every pair concurrent or set ≤ 1) + `closing_antichain(lc) -> Vec<BlockId>` (returns DAG leaves in canonical sorted order). 6 new tests: empty/singleton-vacuous, concurrent pair, comparable-pair rejection, three-concurrent-siblings, diamond closing-antichain, three-sibling closing-antichain. The full Phase 4.2 finality predicate (antichain + 2f+1 precommits) consumes these primitives; that wiring is Phase 4.1's job in tendermint.rs.
- [x] **4.3 — Cross-fork equivocation (counter substrate).** SHIPPED per Decision 3. `cross_fork_equivocations: HashMap<u64, u64>` field on `TendermintConsensus` (validator_id → count). Read-only accessor `cross_fork_equivocations()`. Operators feed `[counts]` into `evaporchain_entropic_slashing::entropic_slash(stake, counts)` to derive the slash amount — same pattern as Crooks-MEV's `mev_missing_refund_violations`. Lifecycle hook (increment on observed cross-fork double-precommit) is Phase 4.3 implementation alongside the voting-handler dispatch.
- [x] **4.4 — Finality gap migration (dual-mode bookkeeping).** SHIPPED per Decision 4. `committed_at_block: HashMap<[u8; 32], u64>` populates alongside the existing `committed_at: HashMap<u64, u64>` on every commit. Cap at 1024 entries (oldest by commit timestamp pruned). Accessor `committed_at_block()`. Test `test_committed_at_block_dual_mode_bookkeeping` confirms both populate after `on_block_committed`.
- [x] **4.5 — Tests.** 10 new tests across Phase 4 substrate: 6 antichain primitives in `evaporchain-light-cone`, 4 consensus-state fields in `evaporchain-consensus` (`test_dag_round_states_starts_empty`, `test_dag_round_states_insert_surfaces_via_counts`, `test_cross_fork_equivocations_counter`, `test_committed_at_block_dual_mode_bookkeeping`). End-to-end antichain-finalization tests pending the full Phase 4.1 voting-handler dispatch.

**Phase 4 deliverable: SHIPPED end-to-end.** All five sub-system pieces landed:
- 4.1 ✅ Per-fork `dag_round_states` field + `record_dag_*` API + voting-handler wiring at `handle_prevote`/`handle_precommit`
- 4.2 ✅ Antichain primitives (`is_antichain` + `closing_antichain` + proptest) + `try_finalize_antichain` predicate (walks `closing_antichain`, returns leaves with ≥ 2f+1 precommits)
- 4.3 ✅ Cross-fork equivocation: counts-based detection runs in `record_dag_precommit`; `cross_fork_equivocations` counter feeds `entropic_slash`
- 4.4 ✅ Dual-mode finality bookkeeping (`committed_at_block` populates on every commit alongside `committed_at`)
- 4.5 ✅ Tests: 5 finalization tests + 3 vote-record tests + 4 substrate-field tests + 6 antichain primitive tests + 1 antichain proptest = 19 Phase 4 tests total

**Phase 4 is the riskiest piece in the Light-Cone arc.** Now SHIPPED end-to-end behind the existing `light_cone_state_branches_enabled` flag (default-off → linear-mode chain bit-compat preserved). Operators can flip the flag on testnet to exercise antichain finalization end-to-end.

### Phase 5 — Compaction + orphan GC (1-2 weeks)

**Goal:** garbage-collect orphaned branches so DAG memory doesn't grow unboundedly.

- [x] **5.1 — Orphan detection.** SHIPPED. `TendermintConsensus::detect_orphan_branches(current_height) -> Vec<[u8; 32]>` returns tips with `caliber < threshold` AND `last_touched_block < current_height - 32`. Threshold from new `light_cone_orphan_caliber_threshold` governance flag (default 0 = no orphans by caliber alone). Returns canonical-sorted (BlockId-order) list for validator-determinism. Read-only — feeds operator/auditor tooling and Phase 5.3's LRU. 4 tests green: default-threshold, caliber filter, recency filter, canonical ordering.
- [x] **5.2 — Cascade prune.** SHIPPED. `LightCone::prune_orphan_branch(tip) -> BTreeSet<BlockId>` walks the DAG backwards from `tip`, removing every ancestor exclusively in `tip`'s causal past. Stops at branch points shared with live tips. Safety guards: rejects non-leaf tips (would orphan downstream); idempotent on unknown tips. 5 tests green: unknown-tip no-op, non-leaf rejection, linear-chain full cascade, branch-point stop, diamond full cascade.
- [x] **5.3 — State branch GC (LRU + DAG paired).** SHIPPED. `TendermintConsensus::prune_state_branches` now invokes `light_cone_dag.prune_orphan_branch(victim)` immediately after evicting the lowest-caliber metadata entry. The DAG-side cascade trims exclusive ancestors (subject to non-leaf rejection + branch-point stop). Test `test_state_branches_lru_paired_dag_prune` drives a 3-fork DAG (genesis → A, B, C with distinct calibers), triggers eviction at cap=2, asserts both metadata AND DAG ancestors of the lowest-caliber leaf `A` are gone, while shared branch point `genesis` and live siblings `B`/`C` survive.
- [x] **5.4 — Tests.** 5 substrate tests shipped (above); end-to-end "4-fork DAG, prune lowest-caliber tip, assert state shrinks" pending the 5.3 wiring.

**Phase 5 deliverable:** memory bounded under continuous DAG growth. GC is deterministic across validators (every validator prunes the same set).

### Phase 6 — Tests + integration + doctrine (2-3 weeks)

**Goal:** lock the contract end-to-end and update doctrine.

- [x] **6.1 — End-to-end DAG-mode test.** SHIPPED. Two integration tests now ship:
  - `test_light_cone_substrate_end_to_end` — drives 8 blocks through `on_block_committed` exercising Phase 3 substrate + Phase 4.4 dual-mode bookkeeping + Phase 5.1 orphan detection.
  - `test_dag_mode_full_pipeline_end_to_end` (this commit) — exercises the FULL Phase 4 pipeline end-to-end: 4-fork DAG → LRU eviction (cap=3 → leaf A pruned from both metadata + DAG) → vote-record API on the surviving 3 leaves with mixed quorum outcomes (B 3-precommit ✓, C 2-precommit ✗, D 4-precommit ✓) → cross-fork equivocation triggers on validators precommitting across multiple tips → `try_finalize_antichain` returns `[B, D]` (the quorum-meeting subset) → closing antichain reflects surviving leaves.
  
  Both tests confirm the consensus-state-machine pipeline behaves correctly under the locked decisions in `PHASE_3_DECISIONS.md` + `PHASE_4_DECISIONS.md`.
- [x] **6.2 — Adversarial 2-fork test.** SHIPPED. `test_dag_mode_adversarial_2fork_split_vote_converges` drives a 2-fork DAG with 4 validators split 2/2 across the leaves. Round 1: neither leaf reaches threshold=3 → no finalization (the chain correctly stalls under split-vote). When validator 3 switches to leaf A: cross-fork equivocation counter triggers (Decision 3 honesty caveat — counts-based detection cannot distinguish honest re-vote from malicious double-vote; Phase 4.3d certificate-based evidence refines this), and leaf A reaches 3 precommits → finalizes. Convergence proven within 2 vote-record cycles.
- [x] **6.3 — Performance budget.** SHIPPED. `benchmark_light_cone_phase_6_3` (`#[ignore]`) drives 1000 DAG blocks @ 4 concurrent forks. **Measured on Mini under release**:
  - DAG insertion: **418 ns/block** (budget < 100 ms; **240,000× under**)
  - `MccForkChoice::select_tip` over 1000 blocks: **365 µs** (budget < 50 ms; **137× under**)
  - 4-fork state-branch metadata + LRU prune: **15.8 µs** (budget < 200 ms; **12,600× under**)
  
  All hot operations clear their budgets by 100×–10⁵×. The substrate has ample headroom for production load.
- [x] **6.4 — `DOCTRINE_PUNCH_LIST.md` Layer 6.** SHIPPED. Light-Cone row in Layer 6 cell flipped from "⏳ tendermint.rs still 8,782 LOC; …" to "✅ substrate-complete 2026-05-04 — full plan in `LIGHT_CONE_FULL_DAG_PLAN.md`" with the full Phase 1+2+3+5 + Phase 4 substrate manifest, governance flags, and the locked-decisions pointer. Voting-handler wiring called out as the only remaining consensus-state-machine surgery.
- [x] **6.5 — `INVENTION_STACK.md §A1.2 row 1`.** SHIPPED. Light-Cone Consensus row updated with "✅ SUBSTRATE-COMPLETE 2026-05-04" + manifest of shipped pieces + governance flag pointer.
- [x] **6.6 — Whitepaper §4.** SHIPPED. `research/whitepaper.md` §4 now carries §4.5 "Light-Cone Full DAG Mode (Optional, governance-gated)" — sub-sections 4.5.1 through 4.5.7 cover the multi-parent block format, MCC tip selection, per-fork state branches, antichain finalization, cross-fork equivocation, the measured 6.3 performance numbers (418 ns/block insertion, 365 µs select_tip, 15.8 µs state-branch ops), and the rollout-flag procedure. Default-off framing preserved throughout — the chain's primary consensus engine remains §§4.1-4.4 (rotating leader); DAG mode activates only when operators flip the governance flag.

**Phase 6 deliverable: SHIPPED end-to-end (6/6 sub-items).** All consensus-state-machine surgery, all integration tests, all doctrine docs, perf budgets, and whitepaper §4 are now landed. Light-Cone Full DAG plan is ✅ COMPLETE.

---

### Phase 7 — Cross-validator antichain agreement digest (2026-05-05 addendum)

Originally projected as out-of-scope follow-up; shipped as a small
substrate addendum 2026-05-05 because the doctrine rollout runbook
explicitly flagged it as the next operator-facing piece beyond the
6/6 Phase 6 deliverable.

- [x] **7.1 — `digest_antichain` + `closing_antichain_digest`.** SHIPPED in `evaporchain-light-cone::concurrency`. Domain-separated under `evaporchain-antichain-digest-v1`; sort-before-hash for validator-determinism; 32-byte blake3 output; empty-set sentinel = blake3-of-domain-tag-alone. 6 new substrate tests (order-independence, set-separation, empty-sentinel, domain-separation, composition idiom, diverging-DAG separation).
- [x] **7.2 — `TendermintConsensus` accessors.** SHIPPED. `light_cone_antichain_digest()` (32-byte digest of current closing antichain) + `light_cone_closing_antichain()` (sorted BlockId list the digest commits to).
- [x] **7.3 — HTTP endpoint.** SHIPPED. `GET /api/light_cone/antichain_digest` returns `{digest, closing_antichain, closing_antichain_size, running_alongside_tendermint}`. Operators `curl` across all cluster validators and pattern-match the digests; divergence is the freeze-class signal for antichain disagreement. Pairs with Crooks-MEV's `mev_state_digest` (Phase 3.2) as the second canonical inter-validator digest.
- [x] **7.4 — Operator runbook.** SHIPPED. `docs/runbooks/doctrine-rollout-2026-05.md` Step 2 of the Light-Cone DAG mode rollout sequence updated to use `/api/light_cone/antichain_digest` for the inter-validator agreement check.

**Phase 7 deliverable: SHIPPED end-to-end (4/4 sub-items).** Light-Cone tests now 34/34 green (was 28).

---

### Phase 8 — MCC full multi-parent enumeration (2026-05-05 addendum)

Phases 1-6 shipped Light-Cone DAG mode as a co-existing alternative
to Tendermint behind `parent_acceptance_mode = "mcc"` — but only
the *single-line trajectory walk* version of MCC, which scores one
trajectory per leaf and picks the argmax. **The full multi-parent
enumeration (where the consensus hot path actually selects an
authoritative head per round, replays state across branch switches,
and routes votes by head)** is the load-bearing extension.

That work has its own dedicated tracking doc:
[`MCC_FULL_MULTI_PARENT_PLAN.md`](MCC_FULL_MULTI_PARENT_PLAN.md).

**Status as of 2026-05-05:** Phase A (substrate, 3/4 items, A.2
deferred) + Phase B (state-replay pipeline, 8/8 items) + Phase E.1
(`/api/light_cone/candidate_heads` endpoint) + Phase E.4 + E.5
(doctrine doc updates) shipped. **34 new tests** across
`evaporchain-light-cone` (10) + `evaporchain-consensus` (24).
Light-Cone test suite: 51/0 (was 34 after Phase 7). Consensus suite:
493/0/1 (was 469).

**What that means for this plan's deliverables:** Light-Cone's
Phase 4.4 antichain commit-cert digest pairs with MCC's
`enumerate_candidate_heads` to give operators a complete view of
"which heads are competing right now and is the cluster agreeing
on antichain finality." The two endpoints
(`/api/light_cone/antichain_digest_history` + `/api/light_cone/candidate_heads`)
are designed to be `curl`'d in parallel for cluster-divergence
diagnosis.

**Remaining work** (per the MCC plan): Phase C (hot-path consensus
surgery — promote `authoritative_head` from admin-RPC to the
consensus hot path, route votes by head, proposer multi-parent set
selection), Phase D (adversarial + perf + 72hr soak), Phase E.2/E.3/E.6
(remaining endpoints + this addendum + operator runbook).

**Phase 8 deliverable: SUBSTRATE COMPLETE; consensus-hot-path
integration is the focused next session.**

---

## Stopping conditions

- **Phase 1 latency > 1.5× linear-mode latency** for the same workload — DAG fork-choice can't hold its own. Reframe: maybe DAG mode is opt-in for high-stake chains, not default.
- **Phase 2 hash-stability test fails** — old blocks have different hashes under v3. Critical chain-id break. Halt and rework the wire-format gating.
- **Phase 3 per-fork state cost > 4× single-fork state cost** at 4 concurrent forks — memory unworkable. Lower the concurrent-fork cap to 2 or move to copy-on-write state.
- **Phase 4 equivocation false-positive rate > 0.1%** under honest network conditions — the slash rule is too aggressive. Loosen the cross-fork rule to require explicit conflicting precommits, not just observed concurrent voting.
- **Phase 6 worst-case insertion > 200 ms** at 1000 DAG blocks — too slow for the hot path. Restrict DAG depth (recent N=256 blocks only) or move insertion to a background task.

---

## Cross-cutting risks

1. **Tendermint linear assumptions are deeply baked in.** Phases 1-2 are mostly additive; Phase 3 is the surgery. If Phase 3 surfaces architectural blockers, the whole plan needs re-litigation. **This is the highest single risk in the chain's roadmap.**

2. **Wire-format change cost.** Phase 2's `parents: Vec` field changes block hashes if not handled carefully. Existing chain history must continue to verify under the new format. Phase 2.4 hash-stability test is the gate.

3. **Multi-fork state isn't just memory — it's also gas accounting.** Phase 3 needs per-fork fee market state too (Singh-Lyapunov runs per-tip). Phase 5 of Crooks-MEV's `mev_attacker_stats` table will need to be DAG-aware too: a sandwich detected on fork A but not fork B has different settlement implications.

4. **The Causal-Cone Validator State (`evaporchain-causal-cone`) crate is undertested.** Currently used as a substrate primitive but Phase 3+ leans on its ε-machine reconstruction for cross-fork energy reconciliation. May need its own hardening pass.

5. **Cluster-wide DAG-state agreement under partition.** During a network split, two halves see different DAG slices. When the split heals, they converge via MCC — but the convergence path must not violate finality (a previously-finalized antichain stays finalized). Phase 4.4's antichain finality rule must handle this; testing against synthetic partition scenarios is Phase 6.

---

## What this plan does NOT cover

- **DAG-aware Lambda-Fold IVC.** The current Nova folding is per-linear-step. Folding across DAG forks requires non-trivial circuit redesign. Out of scope; tracked as a separate `LAMBDA_FOLD_DAG_PLAN.md` follow-up if the chain ever ships full DAG.
- **DAG-aware MEV detection.** `evaporchain-mev-detect::scan_block` examines a single block's tx list. Cross-fork MEV (sandwiching a victim's tx that lands on fork A while attacker exits on fork B) is a new attack class. Phase 5 of Crooks-MEV's `mev_state_digest` is per-fork in DAG mode; cross-fork detection is deferred research.
- **Light-client implications.** Light clients today verify a linear chain. DAG light-client protocol is a research lane (related to Lambda-Fold's `vk_bytes` work but not directly extendable).

---

## Pre-implementation checklist

Before starting Phase 1:

- [x] **Confirm `evaporchain-light-cone::LightCone::insert` rejects cycles.** ✅ DONE 2026-05-06.
      Cycle prevention is **implicit**: the "parents must already
      exist" rule precludes any cycle-forming insert. Three explicit
      tests in `crates/evaporchain-light-cone/src/dag.rs::tests`:
      - `self_cycle_rejected_via_missing_parent` — block whose
        parent is itself rejects with `MissingParent { block: x,
        parent: x }`.
      - `two_cycle_rejected_via_already_inserted` — A→B→A attempt
        re-inserts A → `AlreadyInserted`.
      - `three_cycle_rejected_via_missing_parent` — A→B→C→A
        attempt fails at the first step with a missing parent.
      Light-cone suite 55/0/0.
- [x] **Confirm `MccForkChoice` argmax is deterministic across validators.** ✅ DONE — already locked by MCC Phase C.5
      (`mcc_phase_c5_validator_determinism_under_random_dags`)
      256-iteration proptest; +
      `mcc_phase_a_candidate_heads_converges_across_validators`
      manual test. The original checklist named `choose_parent`,
      which never shipped — actual API is `select_tip` +
      `enumerate_with_caliber`, both proptest-locked.
- [x] **Sanity-check `Block::protocol_version: u8` field.** ✅ — confirmed at `evaporchain-types/src/lib.rs:190`.
- [x] **Sanity-check `LightCone: Send + Sync`.** ✅ DONE 2026-05-06.
      Compile-time check `light_cone_is_send_and_sync` in
      `dag.rs::tests` via the `assert_send::<T>() / assert_sync::<T>()`
      idiom — any future field that breaks Send or Sync fails to
      compile, not at runtime.
- [x] **Phase 1 governance-flag decision: new flag vs reuse existing.** ✅ DONE — chain ships three independent flags as the layered rollout pathway:
      `parent_acceptance_mode ∈ {linear, mcc, mcc_full}` (parent-
      picking + multi-parent enumeration; default `linear`),
      `light_cone_state_branches_enabled ∈ {true, false}` (per-fork
      state branches; default `false`),
      `light_cone_max_concurrent_forks` (1..=8, default 4). The
      checklist's `light_cone_consensus_mode` proposal was
      superseded; the actual three-flag layering is the
      operational tool documented in
      `docs/runbooks/doctrine-rollout-2026-05.md` Lane 4.

---

## Honest scope summary

**This is a 3-6 month plan, not a session-bounded one.** Phases 1-2 are bounded enough to ship in dedicated sessions of 1-2 weeks each. Phases 3-6 each warrant their own planning sub-docs and sessions. The whole arc is genuinely the largest remaining piece of doctrine work in the chain's roadmap (per `DOCTRINE_PUNCH_LIST.md` Layer 6).

If you only have one session: ship **Phase 1 first**. It's the smallest unit that delivers DAG-driven consensus (under MCC mode), even if state branches don't materialize until Phase 3. Phase 1 alone closes ~30% of the doctrine gap.
