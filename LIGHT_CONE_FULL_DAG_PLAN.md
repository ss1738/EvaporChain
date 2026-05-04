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

- [ ] **2.1 — `parents: Vec<[u8; 32]>` on `Block`** (new optional field with `serde(default)`). Existing blocks deserialize with `parents = vec![parent_hash]` (single-parent fall-back). New blocks may emit `parents` directly.
- [ ] **2.2 — Block validation rule.** `Block::validate_parents(dag) -> Result<(), Error>` — every parent must be in the DAG, none equal each other, no parent in the block's causal future (would be a cycle). Emitted by validators on receipt.
- [ ] **2.3 — `protocol_version` bump.** New version 3 (current 2 = post-Lambda-Fold, 1 = pre-fold, 0 = legacy). `parents.len() > 1` requires `protocol_version >= 3`. Validators reject blocks claiming `parents.len() > 1` under lower versions.
- [ ] **2.4 — Hash-stability test.** Old blocks (single-parent) hash identically post-migration (the `parents` field defaults to `vec![parent_hash]` and isn't included in `signable_bytes` until protocol_version 3 is active). Critical for chain-id continuity.
- [ ] **2.5 — Tests.** Round-trip serialize/deserialize at v0, v2, v3; cycle-rejection; multi-parent acceptance under v3; reject under v2.

**Phase 2 deliverable:** the wire format supports DAG-shaped blocks. Backward-compat preserved via `serde(default)` + `protocol_version` gating.

### Phase 3 — Per-fork state branches (3-4 weeks)

**Goal:** materialize the actual state at every DAG tip, not just the longest linear chain. This is the biggest single piece of state-machine surgery.

- [ ] **3.1 — `StateBranchTable: HashMap<BlockId, Arc<StateDB>>`.** Per-tip state snapshot. Populated lazily as forks form; pruned when a fork is finalized-and-orphaned.
- [ ] **3.2 — Per-tip executor.** `ParallelExecutor` instances are per-state-branch (or share a copy-on-write store; the simpler path is per-tip-clone for V1, optimize later).
- [ ] **3.3 — Energy reconciliation rule.** When forks A and B merge (a future block has both A and B as parents), object energies diverge. Rule: **take max energy** (most-recent refresh wins). Documented in a `PHASE_3_DECISIONS.md` sub-doc analogous to `PHASE_2_DECISIONS.md` from Crooks-MEV / Lambda-Fold.
- [ ] **3.4 — Performance budget.** O(n) per-fork executors at scale is unworkable. By V1, cap concurrent forks at 4 (governance flag); validators voting on a 5th fork drop one of the existing four (lowest caliber). Phase 5 optimizes.
- [ ] **3.5 — Tests.** 2-fork branch + commit-and-merge → state matches both forks' contributions; energy reconciliation matches the max-rule.

**Phase 3 deliverable:** the chain materializes parallel state branches. Multi-tip validation + voting + commit is functional. **THIS IS THE TIPPING POINT** — once 3 lands, the chain is genuinely DAG-driven, not just DAG-observed.

### Phase 4 — DAG-aware vote tally + finality (2-3 weeks)

**Goal:** Tendermint's prevote/precommit machinery extended to DAG semantics. Multi-tip voting, antichain finality, equivocation detection across forks.

- [ ] **4.1 — Per-fork RoundState.** `round_state: HashMap<BlockId, RoundState>` instead of a single `RoundState`. Each tip has its own prevote/precommit tally.
- [ ] **4.2 — Antichain finality.** A set of blocks is finalized iff (a) all blocks in the set form an antichain (mutually concurrent), (b) every block in the set has 2f+1 precommits, (c) the set covers a "minimal closing antichain" of the DAG up to height N.
- [ ] **4.3 — Cross-fork equivocation rule.** A validator who precommits on two concurrent tips at the same round is slashable. Reuses the existing `evaporchain-entropic-slashing` primitive.
- [ ] **4.4 — Finality gap on antichains.** `committed_at: HashMap<BlockId, u64>` (was `HashMap<u64, u64>`). Replace height-indexed finality bookkeeping with block-indexed.
- [ ] **4.5 — Tests.** Antichain-finalization happy path; concurrent-tip equivocation slash; round-fail recovery on 2-tip vote-split scenarios.

**Phase 4 deliverable:** the chain finalizes antichain sets, not single heights. Multi-tip voting works end-to-end. Equivocation across forks is slashable.

### Phase 5 — Compaction + orphan GC (1-2 weeks)

**Goal:** garbage-collect orphaned branches so DAG memory doesn't grow unboundedly.

- [ ] **5.1 — Orphan detection.** A branch is orphaned when its tip's caliber falls below a governance-set threshold *and* it's not in the most recent K finalized antichains.
- [ ] **5.2 — Cascade prune.** Pruning a tip cascades: prune all ancestors that are orphaned (not in any other live branch's causal past). `LightCone::prune_orphan_branch(tip)`.
- [ ] **5.3 — State branch GC.** Drop the corresponding `StateBranchTable` entry. Concurrent reads block until prune completes (writer's lock).
- [ ] **5.4 — Tests.** 4-fork DAG, prune the lowest-caliber tip, assert state shrinks; round-trip insertion of a previously-pruned block (the chain rejects re-insertion as a sanity check).

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
