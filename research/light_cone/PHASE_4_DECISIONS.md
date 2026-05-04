# Light-Cone Full DAG — Phase 4 Decisions

**Date:** 2026-05-04
**Pairs with:** `LIGHT_CONE_FULL_DAG_PLAN.md` Phase 4 + `PHASE_3_DECISIONS.md`.

This file locks five design choices that gate Phase 4 implementation. Phase 4 is **the consensus-state-machine surgery**: replacing Tendermint's single `round_state: RoundState` with per-fork voting tallies + antichain finalization. The riskiest piece in the Light-Cone arc — re-litigating mid-implementation will fork the chain.

Mirrors `PHASE_3_DECISIONS.md` and `research/lambda_fold/PHASE_1_DECISIONS.md` in shape.

---

## Reconnaissance findings

Verified against the live codebase before locking:

1. **`round_state: RoundState`** at `tendermint.rs:511` — single per-height/per-round voting state. `RoundState { round, phase, prevotes: HashMap<u64, [u8;32]>, precommits: HashMap<u64, [u8;32]>, ... }`. Every voting handler dispatches against this single field.

2. **Voting handlers**: `handle_prevote`, `handle_precommit`, `handle_proposal` — all pre-dispatch by `(height, round)`. Multi-tip dispatch needs an additional tip key.

3. **`committed_at: HashMap<u64, u64>`** at `tendermint.rs` finality bookkeeping — height → wall-clock-ms. Phase 4.4's antichain finality replaces this with `HashMap<BlockId, u64>`.

4. **`evaporchain-entropic-slashing::entropic_slash(stake, counts)`** primitive shipped (Crooks-MEV Phase 3.5c uses it for `MissingRefund` slashing). Phase 4.3 cross-fork equivocation reuses the same primitive with a different counts source.

5. **Phase 4.2 substrate primitives** shipped in `evaporchain-light-cone::concurrency`: `is_antichain(lc, set)` and `closing_antichain(lc)`. The full finalization predicate (antichain + 2f+1 precommits per block) is Phase 4.1's wiring job.

6. **Phase 3 substrate** complete: `state_branches: HashMap<[u8;32], LightConeBranchMetadata>` with `LightConeBranchSnapshot` trait seam. Phase 4 voting state can co-exist with Phase 3's per-fork state — they key on the same `BlockId`.

---

## Decisions locked

### Decision 1 — Voting state: parallel `dag_round_states` HashMap, not wholesale replacement. LOCKED.

**Choice:** add `dag_round_states: HashMap<BlockId, RoundState>` to `TendermintConsensus`, **alongside** the existing single `round_state`. The existing single field stays as the "primary fork" voting state; the HashMap accumulates votes for non-primary tips when they're observed.

**Rationale:** wholesale replacement of `round_state` would touch 50+ voting-handler sites in tendermint.rs and break the chain's bit-compat with linear-mode validators. The parallel-table approach is additive: when `light_cone_state_branches_enabled = false` (default), `dag_round_states` stays empty and the chain is identical to pre-Light-Cone. When the flag is on, the table accumulates per-tip tallies that Phase 4.2 finalization checks.

**Side effect for Phase 4.1 implementation:** voting-handler dispatch becomes:
```rust
fn record_vote(&mut self, vote: Vote) {
    // Existing path: route to self.round_state (primary fork).
    self.route_vote_to_primary_round(vote);
    // Phase 4.1 addition: when state_branches_enabled, also route
    // to the matching tip's dag_round_states entry (if any).
    if self.state_branches_enabled() {
        if let Some(tip) = self.tip_for_block_hash(vote.block_hash) {
            self.dag_round_states.entry(tip).or_default().record(vote);
        }
    }
}
```

**Honesty caveat:** the parallel-table approach means the chain has two voting state machines running side-by-side under `state_branches_enabled = true`. Determinism contract: every validator must agree on which votes go to which tip. The mapping `vote.block_hash → tip` is the chain's `block_hash` itself (a vote names a specific block; that block IS the tip). So the routing is deterministic.

### Decision 2 — Antichain finality predicate: substrate primitives + 2f+1 per block. LOCKED.

**Choice:** a set of blocks `S` is finalized iff:

1. `is_antichain(lc, &S)` — every pair concurrent (substrate primitive shipped Phase 4.2).
2. For every `b ∈ S`: `dag_round_states.get(&b).map(|r| r.precommit_count()).unwrap_or(0) >= 2 * f + 1`.
3. `S` covers `closing_antichain(lc)` — every leaf in the DAG is either in `S` or finalized in an earlier antichain.

**Rationale:** this is the standard antichain-BFT contract from CASS / DAG-Rider style consensus, adapted to EvaporChain's `RoundState` shape. Condition 3 ("covers the closing antichain") prevents partial finalization — finalizing only some leaves while others sit unvoted leaves the chain in an indeterminate state.

**Side effect:** Phase 4.4's finality bookkeeping migrates `committed_at: HashMap<u64, u64>` to `HashMap<BlockId, u64>` (block-indexed instead of height-indexed). The migration is gated on the same `light_cone_state_branches_enabled` flag.

### Decision 3 — Cross-fork equivocation: counts-based, not certificate-based. LOCKED.

**Choice:** track per-validator equivocation counts in `cross_fork_equivocations: HashMap<u64, u64>` (validator_id → count). Increment by 1 every time a validator is observed precommitting on two concurrent tips at the same round. Operators feed `[counts]` into `entropic_slash(stake, counts)` to derive the slash amount — same pattern as Crooks-MEV Phase 3.5c.

**Rationale:** the alternative (storing the conflicting precommit pair as a certificate, signed evidence of equivocation) is heavier and requires a new tx variant. Counts are simpler and reuse the existing pattern.

**Honesty caveat:** counts-based is **less verifiable** than certificate-based — operators must trust the chain's own observation of equivocation. Phase 4.3d follow-up: certificate-based equivocation evidence with on-chain proof. Phase 4 ships the counts surface; certificates are research-track.

**Side effect:** new field `cross_fork_equivocations: HashMap<u64, u64>` on `TendermintConsensus`, initialized empty. Lifecycle hook in `record_vote` increments on detected equivocation.

### Decision 4 — Finality gap migration: dual-mode bookkeeping. LOCKED.

**Choice:** add `committed_at_block: HashMap<BlockId, u64>` (block-indexed) **alongside** the existing `committed_at: HashMap<u64, u64>` (height-indexed). Both populate on every commit. When `light_cone_state_branches_enabled = true`, finality-gap calculations consult both: height-indexed for linear-mode chain consumers, block-indexed for DAG-aware consumers.

**Rationale:** keeping both means existing `unfinalised_tail()` callers don't break. The DAG-aware accessor (Phase 4.4 new) gives the per-block view. Phase 6 doctrine sweep can then deprecate height-indexed once DAG mode is the default — but that's the post-V1 cleanup.

**Side effect:** double the finality-bookkeeping memory under DAG mode. Cap at 1024 entries (matching `MEV_OBSERVATION_BUFFER_CAP`'s pattern); prune on overflow.

### Decision 5 — Phase 4 ships under same governance flag as Phase 3. LOCKED.

**Choice:** Phase 4 is gated on `light_cone_state_branches_enabled = "true"` (the existing Phase 3.5 flag). No new flag.

**Rationale:** Phase 4 voting infrastructure operates against Phase 3's state branches. Activating Phase 4 without Phase 3 state-branches makes no semantic sense. Reusing the flag means a single governance flip activates the full DAG-mode behaviour (state branches + per-fork voting + antichain finality + cross-fork equivocation).

**Compose with Decision 1:** when the flag is `false`, `dag_round_states` stays empty. When `true`, all four Phase 4 sub-systems activate together.

---

## Implications for Phase 4 implementation

Phase 4 sub-tasks update as follows:

- **4.1 — Per-fork RoundState:** `dag_round_states: HashMap<BlockId, RoundState>` field on `TendermintConsensus`. Initialized empty. Voting handlers dispatch primary → `self.round_state`, secondary → `self.dag_round_states[tip]` per Decision 1.

- **4.2 — Antichain finalization:** the substrate primitives (`is_antichain`, `closing_antichain`) shipped Phase 4.2. The finalization predicate per Decision 2 lives on `TendermintConsensus::try_finalize_antichain(&mut self) -> Vec<BlockId>` — runs on every commit, returns the just-finalized antichain.

- **4.3 — Cross-fork equivocation:** `cross_fork_equivocations: HashMap<u64, u64>` field per Decision 3. Lifecycle hook in voting handlers detects + increments. Read-only accessor `cross_fork_equivocations() -> &HashMap<u64, u64>` for operator slashing tooling.

- **4.4 — Finality gap migration:** `committed_at_block: HashMap<BlockId, u64>` field per Decision 4. Existing `committed_at` stays. New accessor `unfinalised_tail_by_block() -> Vec<BlockId>`.

- **4.5 — Tests:** consensus-state-machine integration tests covering: 2-tip voting → antichain finalizes; 3-tip vote-split → no finalization until antichain forms; cross-fork equivocation → counts increment + entropic_slash applies; round-fail recovery on per-tip basis.

---

## Open questions deferred to Phase 4 implementation

1. **Block-hash → tip mapping.** A vote names `block_hash`. To dispatch to `dag_round_states[tip]`, we need `tip_for_block_hash(hash)`. Today's `light_cone_dag` keys on the LightCone-side BlockId, which is computed from the block's content (per `block_hash` in `tendermint.rs:2590`). The mapping is identity — `tip_for_block_hash(hash) = hash` if `hash` is a leaf, else `None`. Confirm this when implementing.

2. **Round timeouts under multi-tip.** `propose_timeout`, `prevote_timeout`, `precommit_timeout` are per-`RoundState`. With multiple `RoundState` instances, do timeouts fire per-tip or chain-globally? Plan: per-tip — each `dag_round_states` entry runs its own timer. Phase 4.1 implementation detail.

3. **Antichain finality interaction with `commit_certificate`.** Today's `commit_certificate` aggregates 2f+1 precommit signatures for a single block. Antichain finality aggregates them for a SET of blocks. New `antichain_commit_certificate` shape needed. Phase 4.4 sub-task.

4. **DAG-mode validator-set updates.** Today `apply_validator_set_changes` runs at commit on a single block. With antichain finalization, multiple blocks finalize in a batch — validator-set changes need ordering. Plan: process changes in BlockId-sorted order within the antichain. Phase 4.2 sub-decision.

---

## Acceptance for Phase 4

This file's existence + commit is Phase 4's design deliverable. Phase 4 implementation starts when:

1. This file is committed.
2. The five decisions above are not contradicted by any code change between now and Phase 4 implementation start.
3. Cross-checked against `LIGHT_CONE_FULL_DAG_PLAN.md` Phase 4 — done in this commit.

**Phase 4 implementation is genuinely a 2-3 week piece per the parent plan.** Decisions above tame the risk; the actual surgery (per-fork voting tables + antichain finalization + cross-fork equivocation tracking + dual-mode finality gap bookkeeping) ships across a sequence of focused commits, each gated on the existing `light_cone_state_branches_enabled` flag for safe rollout.
