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

- [x] **4.1 — Per-fork RoundState (substrate field).** SHIPPED. `dag_round_states: HashMap<[u8; 32], RoundState>` field on `TendermintConsensus`, additive per Decision 1 (existing single `round_state` stays as primary-fork state). Initialized empty in both constructors. Accessors: `dag_round_states_count()` (cardinality) and `dag_round_state_counts(&tip)` returning `(prevote_count, precommit_count)` since `RoundState` itself is private to the crate. The voting-handler dispatch (routing prevote/precommit messages to the right tip) is the focused next-session piece.
- [x] **4.2 — Antichain primitives (substrate).** SHIPPED in `evaporchain-light-cone::concurrency`. `is_antichain(set: &[BlockId]) -> bool` (every pair concurrent or set ≤ 1) + `closing_antichain(lc) -> Vec<BlockId>` (returns DAG leaves in canonical sorted order). 6 new tests: empty/singleton-vacuous, concurrent pair, comparable-pair rejection, three-concurrent-siblings, diamond closing-antichain, three-sibling closing-antichain. The full Phase 4.2 finality predicate (antichain + 2f+1 precommits) consumes these primitives; that wiring is Phase 4.1's job in tendermint.rs.
- [x] **4.3 — Cross-fork equivocation (counter substrate).** SHIPPED per Decision 3. `cross_fork_equivocations: HashMap<u64, u64>` field on `TendermintConsensus` (validator_id → count). Read-only accessor `cross_fork_equivocations()`. Operators feed `[counts]` into `evaporchain_entropic_slashing::entropic_slash(stake, counts)` to derive the slash amount — same pattern as Crooks-MEV's `mev_missing_refund_violations`. Lifecycle hook (increment on observed cross-fork double-precommit) is Phase 4.3 implementation alongside the voting-handler dispatch.
- [x] **4.4 — Finality gap migration (dual-mode bookkeeping).** SHIPPED per Decision 4. `committed_at_block: HashMap<[u8; 32], u64>` populates alongside the existing `committed_at: HashMap<u64, u64>` on every commit. Cap at 1024 entries (oldest by commit timestamp pruned). Accessor `committed_at_block()`. Test `test_committed_at_block_dual_mode_bookkeeping` confirms both populate after `on_block_committed`.
- [x] **4.5 — Tests.** 10 new tests across Phase 4 substrate: 6 antichain primitives in `evaporchain-light-cone`, 4 consensus-state fields in `evaporchain-consensus` (`test_dag_round_states_starts_empty`, `test_dag_round_states_insert_surfaces_via_counts`, `test_cross_fork_equivocations_counter`, `test_committed_at_block_dual_mode_bookkeeping`). End-to-end antichain-finalization tests pending the full Phase 4.1 voting-handler dispatch.

**Phase 4 substrate deliverable: SHIPPED.** All four sub-system fields landed on `TendermintConsensus` + the antichain primitives in `evaporchain-light-cone`. The remaining Phase 4 work is **wiring**: route prevote/precommit messages to per-tip `dag_round_states`, implement `try_finalize_antichain` predicate from substrate, increment `cross_fork_equivocations` on observed double-precommits. All gated on the existing `light_cone_state_branches_enabled` flag.

### Phase 5 — Compaction + orphan GC (1-2 weeks)

**Goal:** garbage-collect orphaned branches so DAG memory doesn't grow unboundedly.

- [ ] **5.1 — Orphan detection.** Pending. The detection rule (caliber threshold + recency check) is the consumer-side; the substrate primitive in 5.2 is shipped.
- [x] **5.2 — Cascade prune.** SHIPPED. `LightCone::prune_orphan_branch(tip) -> BTreeSet<BlockId>` walks the DAG backwards from `tip`, removing every ancestor exclusively in `tip`'s causal past. Stops at branch points shared with live tips. Safety guards: rejects non-leaf tips (would orphan downstream); idempotent on unknown tips. 5 tests green: unknown-tip no-op, non-leaf rejection, linear-chain full cascade, branch-point stop, diamond full cascade.
- [ ] **5.3 — State branch GC.** Phase 3.4 LRU eviction in `tendermint.rs::prune_state_branches` already drops `LightConeBranchMetadata` entries; pairing it with 5.2's DAG-side `prune_orphan_branch` is the focused next-session piece — when LRU evicts a tip, also call `light_cone_dag.prune_orphan_branch(tip)` to trim the underlying DAG ancestors.
- [x] **5.4 — Tests.** 5 substrate tests shipped (above); end-to-end "4-fork DAG, prune lowest-caliber tip, assert state shrinks" pending the 5.3 wiring.

**Phase 5 deliverable:** memory bounded under continuous DAG growth. GC is deterministic across validators (every validator prunes the same set).

### Phase 6 — Tests + integration + doctrine (2-3 weeks)

**Goal:** lock the contract end-to-end and update doctrine.

- [ ] **6.1 — End-to-end DAG-mode test.** Drive a 100-block DAG through `on_block_committed` with 4 forks, finalize via antichain rule, assert state convergence + correct caliber-based tip selection.
- [ ] **6.2 — Adversarial 2-fork test.** Validators split into two groups voting on different tips; assert MCC + equivocation rule converges within 5 rounds.
- [ ] **6.3 — Performance budget.** 1000 DAG blocks @ 4 concurrent forks: insertion < 100 ms/block, fork-choice select_tip < 50 ms, state-branch creation < 200 ms.
- [ ] **6.4 — `DOCTRINE_PUNCH_LIST.md` Layer 6.** Flip Light-Cone full DAG row from ⏳ to ✅. Layer 6 closes.
- [ ] **6.5 — `INVENTION_STACK.md §A1.2 row 1`.** "Light-Cone Consensus" row updated with SHIPPED date + test names.
- [ ] **6.6 — Whitepaper §4.** Replace the rotating-leader Tendermint section with the DAG-consensus section. Big chunk; touches academic-press lane.

**Phase 6 deliverable:** Light-Cone is the chain's actual consensus mechanism. Linear Tendermint stays as a fallback governance mode for emergency rollback.

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

- [ ] Confirm `evaporchain-light-cone::LightCone::insert` rejects cycles (currently relies on caller-side `MissingParent`; explicit cycle test missing).
- [ ] Confirm `MccForkChoice::choose_parent` is deterministic across validators with the same DAG view (no HashMap iteration order dependence).
- [ ] Sanity-check the chain ships `protocol_version: u8` field on `Block` (it does — line 190 of `evaporchain-types/src/lib.rs`).
- [ ] Sanity-check `LightCone` is `Send + Sync` (it is — used inside the `Arc<Mutex<TendermintConsensus>>` on the API side).
- [ ] Decide whether Phase 1 ships under a new governance flag (`light_cone_consensus_mode = "dag" | "linear"`) or reuses the existing `parent_acceptance_mode = "mcc"`. Recommendation: new flag — `parent_acceptance_mode` is parent-picking, not full DAG consensus.

---

## Honest scope summary

**This is a 3-6 month plan, not a session-bounded one.** Phases 1-2 are bounded enough to ship in dedicated sessions of 1-2 weeks each. Phases 3-6 each warrant their own planning sub-docs and sessions. The whole arc is genuinely the largest remaining piece of doctrine work in the chain's roadmap (per `DOCTRINE_PUNCH_LIST.md` Layer 6).

If you only have one session: ship **Phase 1 first**. It's the smallest unit that delivers DAG-driven consensus (under MCC mode), even if state branches don't materialize until Phase 3. Phase 1 alone closes ~30% of the doctrine gap.
