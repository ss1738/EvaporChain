# Light-Cone Full DAG — Phase 3 Decisions

**Date:** 2026-05-04
**Pairs with:** `LIGHT_CONE_FULL_DAG_PLAN.md` Phase 3.

This file locks five design choices that gate Phase 3+. Phase 3 is "the tipping point" of the Full DAG plan — per-fork state branches replace the linear `current_state_root` with a `HashMap<BlockId, StateDB>` keyed by DAG tip. The consensus-state-machine surgery starts here, so the decisions need to be locked up-front; re-litigating mid-implementation will fork the plan.

Mirrors the shape of `research/lambda_fold/PHASE_1_DECISIONS.md` and `research/crooks_mev/PHASE_2_DECISIONS.md`.

---

## Reconnaissance findings

Verified against the live codebase before locking:

1. **`current_state_root: [u8; 32]` on `TendermintConsensus`** at `tendermint.rs:625` — single state root, single executor. Phase 3 keeps this as the "primary fork" state and adds the `state_branches` table for the alternative tips.

2. **`ParallelExecutor`** (`evaporchain-execution::parallel`) is the live executor. It holds its own DB handle and the LightCone-DAG / VS / mempool state via mutable references. `Clone` is not implemented — adding it just for state-branch fan-out would be a heavy lift.

3. **`StateDB` trait** (`evaporchain-state`) is a backed-by-RocksDB key-value store with snapshot semantics. `Snapshot` exists; the executor reads from `db.snapshot()` for consistency within a block. Phase 3 leans on `Snapshot::clone` (cheap — RocksDB snapshots are O(1) refs to immutable LSM-tree state).

4. **Phase 1 + 2 substrate is in place.** `LightCone::leaves()`, `MccForkChoice::select_tip`, `Block::effective_parents()`, `Block::validate_parents_wire_format()`. Phase 3 builds on top — state branches are keyed on DAG leaves; merge nodes (`parents.len() > 1`) reconcile per Decision 4.

5. **The Singh-Lyapunov fee controller, MEV detector, attacker-stat table, settled-refunds set** all live on the linear `TendermintConsensus`. Phase 3 must decide whether these are per-fork or chain-global. The naive answer is "per-fork" but doubles the state machinery.

---

## Decisions locked

### Decision 1 — State-branch storage: per-tip RocksDB snapshots. LOCKED.

**Choice:** `state_branches: HashMap<BlockId, Arc<dyn StateDB>>` on `TendermintConsensus`. Each entry is a RocksDB-snapshot-backed `StateDB` that shares the underlying LSM tree with the primary executor's DB. Snapshots are O(1) to create and reference-counted; pruning is `drop`.

**Rationale:** RocksDB snapshots are cheap (a single seq-no marker on the LSM tree). Cloning the executor + DB per fork would be O(state-size); snapshot-sharing is O(1). The cost is paid only on commits to a non-primary fork (which writes a divergent key set into a new column-family). This keeps the per-fork state cost ≪ 4× single-fork even at the Phase 3.4 cap of 4 concurrent forks.

**Alternatives considered:**
- **Per-tip clone of `ParallelExecutor`** — too heavy. ~10k LOC of state to clone per fork; defeats the point.
- **Copy-on-write state via differential layers** — elegant but needs a custom KV layer EvaporChain doesn't ship. Out of scope for V1.

**Side effect for Phase 3.2+:** the executor's serial-phase block-commit needs to know which fork it's executing against; the dispatch becomes `executor.execute_block(block, &mut state_branches[&fork_tip])`. Today's executor signature changes from `&mut self` to `&mut self, fork: BlockId` (or equivalent — the actual API will be settled in Phase 3.2).

### Decision 2 — Concurrent-fork cap: governance flag, default 4. LOCKED.

**Choice:** `light_cone_max_concurrent_forks` governance flag, default `4`, range `1..=8`. When the chain has more than `cap` live tips (post-prune), the lowest-caliber leaf is evicted: its state branch dropped, its DAG ancestors that are not in any other branch's causal past pruned per Phase 5.

**Rationale:**
- **`1`** = linear chain (no DAG fan-out). Useful for testnets / emergency rollback.
- **`4`** = production default. Empirical Tendermint-style chains rarely see >2 concurrent forks; 4 is a 2× headroom. Memory cost: 4× snapshot refs (cheap).
- **`8`** = ceiling. Beyond this, the per-fork state cost stops being constant in the Snapshot-shared model (RocksDB snapshot retention pinning growing LSM tail). Phase 3.5 stopping condition: ratio > 4× single-fork at 4 forks → drop the cap.

**Allowlist enforcement:** `governance_set_param("light_cone_max_concurrent_forks", v)` accepts `u8 in 1..=8`; out-of-range → `InvalidValue`.

### Decision 3 — Energy reconciliation on merge: take-max. LOCKED.

**Choice:** when forks A and B merge (a future block has both A and B as parents), object energies diverge. Rule: **for every object touched in either fork, take `max(energy_A, energy_B)` as the merged energy.** Refresh-monotonicity (refreshes are positive; decay is negative) means max-rule never loses a refresh that happened on either branch.

**Rationale:** Refresh-monotonicity is the chain's only soundness guarantee for energy. Taking min would discard refreshes on the losing branch (operator-hostile). Taking sum would double-count (math-wrong). Max preserves the invariant "any honest refresh that landed on any branch is preserved on merge."

**Side effect for Phase 3.3:** the merge handler walks both branches' touched-object set, computes max for each. Worst-case O(N_A + N_B) where N is the number of objects touched per branch. Bounded by the per-block tx count (≤ 1000) so the per-merge cost is < 50 ms on M4.

**Honesty caveat:** max-rule is semantically defensible but provably suboptimal under adversarial-fork scenarios — an attacker who refreshes an object on a phantom fork that's about to lose still gets the refresh on the surviving branch. Mitigation: Phase 4's antichain finality requires the merge node to actually be voted in; ghost forks don't merge. **A rigorous correctness proof of max-rule under partial-finality semantics is deferred research.**

### Decision 4 — Per-fork ancillary state: SHARED, NOT FORKED. LOCKED.

**Choice:** the following stay on the linear `TendermintConsensus` (chain-global, not per-fork):
- Singh-Lyapunov fee controller (`fee_state`)
- MEV detector (`mev_observations`, `mev_attacker_stats`, `settled_refunds`, `disputed_observations`, `mev_missing_refund_violations`)
- TUR liveness window (`tur_window`)
- Cartel alarm (`cartel_alarm`)
- Causal-CHSH samples
- Lambda-Fold instances (`lambda_fold`, `lambda_fold_nova_instance`)

**Rationale:** these are observation / accumulator primitives whose semantic is "what has the chain seen so far across all forks." Per-fork copies would multiply the state machinery for no clear soundness benefit. The right semantic is "primary-fork-canonical": these primitives observe the primary fork's commit stream; alternative forks contribute to detection but not to the canonical state.

**Side effect:** Phase 4's antichain finality may need to revisit some of these (e.g., if an alternative fork finalizes, its observations should retroactively merge into the canonical accumulators). Tracked as a Phase 4 sub-decision.

### Decision 5 — Phase 3 ships behind a NEW governance flag: `light_cone_state_branches_enabled`. LOCKED.

**Choice:** Phase 3 is OFF by default. `governance_set_param("light_cone_state_branches_enabled", "true")` activates the per-fork state machinery; `false` (default) keeps the chain in linear-state mode (only `current_state_root` matters; `state_branches` is empty).

**Rationale:** Phase 3 is the "tipping point" — flipping it on changes the chain's commit-time semantics globally. A safe rollout flag lets operators activate per-fork state on testnets, observe behavior, then flip on mainnet via governance once Phases 4-5 close.

**Allowlist:** `light_cone_state_branches_enabled ∈ {"true", "false"}`. Default `"false"`.

**Compose with Decision 2:** when `light_cone_state_branches_enabled = "false"`, the `light_cone_max_concurrent_forks` flag is observed but inert (no branches to cap).

---

## Implications for Phase 3 implementation

Phase 3 sub-tasks update as follows:

- **3.1 — `state_branches` table:** `HashMap<BlockId, Arc<dyn StateDB>>` on `TendermintConsensus`. RocksDB Snapshot under each Arc per Decision 1.
- **3.2 — Per-tip executor dispatch:** when committing a block on tip `T`, the executor receives `state_branches[&T]`. The signature change is gated behind Phase 3.5's `light_cone_state_branches_enabled` flag so legacy linear executors stay bit-compat.
- **3.3 — Merge handling:** for each `Block` with `parents.len() > 1`, the merge handler computes max-energy reconciliation per Decision 3 across the parent forks' state branches.
- **3.4 — Concurrent-fork cap:** at the start of each commit, if `state_branches.len() > cap`, evict the lowest-caliber leaf (drop its Arc; the underlying RocksDB snapshot deref is O(1)).
- **3.5 — `PHASE_3_DECISIONS.md` (this file)** + governance flag rollout per Decision 5.

---

## Open questions deferred to Phase 3 implementation

1. **Snapshot retention semantics under continuous DAG growth.** RocksDB snapshots pin LSM tree segments; if a fork is held for hours, the LSM tree grows. Need to test the snapshot retention vs. compaction trade-off under sustained 4-fork load.

2. **Cross-fork tx replay.** A tx that landed on fork A's mempool but not B's — should B accept it on its merge ancestor? Plan: the mempool stays chain-global (Decision 4); txs visible on fork A's commit are also visible on fork B's commit-time reads. The merged block carries both forks' tx sets.

3. **Lambda-Fold IVC across forks.** Today's Nova folding is per-linear-step. Per-fork folding would need either separate prover instances (heavy memory) or a "fold-then-merge" circuit redesign. Out of scope for Phase 3 — tracked as `LAMBDA_FOLD_DAG_PLAN.md` follow-up if the chain ever ships full DAG.

4. **MEV digest semantics with shared observations.** Per Decision 4, MEV state is chain-global. The `mev_state_digest` (Crooks-MEV Phase 3.2) is unaffected. Phase 4 antichain finality may revisit this.

---

## Acceptance for Phase 3

This file's existence + commit is Phase 3's design deliverable. Phase 3 implementation starts when:

1. This file is committed.
2. The five decisions above are not contradicted by any code change between now and Phase 3 implementation start.
3. Cross-checked against `LIGHT_CONE_FULL_DAG_PLAN.md` Phase 3 — done in this commit.

Phase 3 implementation is genuinely a 3-4 week piece per the parent plan. The decisions above tame the risk; the actual surgery (per-tip executor dispatch + merge handler + concurrent-fork cap + governance flag) ships across a sequence of focused commits.
