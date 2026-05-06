# Doctrine Rollout Runbook — 2026-05

Operator guide for the three frontier-primitive lanes shipped in the
2026-05-05 doctrine arc:

1. **Lambda-Fold** — sublinear-in-active-energy IVC verifier
2. **Crooks-MEV** — fluctuation-theorem MEV refund pipeline
3. **Light-Cone Full DAG** — partial-order causal-set consensus

All three are gated behind governance flags. **Default-mode chain is
bit-compat with pre-doctrine behaviour.** This runbook documents the
safe-rollout sequence for each lane.

Pairs with: `LAMBDA_FOLD_NOVA_PLAN.md`, `CROOKS_MEV_INTEGRATION_PLAN.md`,
`LIGHT_CONE_FULL_DAG_PLAN.md`, `CHANGELOG.md` 2026-05-05 entry.

---

## Universal pre-flight (do this before any lane)

1. **Confirm the cluster is on a release that includes the doctrine
   substrate.** `git log --oneline | head -10` should show the
   2026-05-05 commits ending in the CHANGELOG entry.

2. **Confirm `cargo test --workspace --release` passes** on a
   representative validator before flipping any flag.

3. **Confirm `governance_flags_snapshot()` reports the doctrine
   defaults**:

   ```
   curl http://localhost:8080/api/governance/flags
   ```

   Expected for a fresh / pre-doctrine chain:
   - `lambda_fold_mode`             = `"hash_chain"` (default)
   - `crooks_mev_settlement_mode`   = `"observe"`    (default)
   - `crooks_mev_missing_refund_slash_enabled` = `"false"` (default)
   - `light_cone_state_branches_enabled`        = `"false"` (default)

   If any of these is non-default on a chain that wasn't expecting it,
   STOP and audit before proceeding.

---

## Lane 1 — Lambda-Fold Nova IVC

### What changes when active

The `evaporchain-proving::nova` real Nova-IVC pipeline runs alongside
the substrate blake3 hash-chain Lambda-Fold accumulator. Light clients
gain `verify_with_vk_bytes` for sublinear-time chain verification.

### Pre-flight (Lane 1 specific)

- Crate features `evaporchain-consensus/lambda_fold_nova` and
  `evaporchain-node/lambda_fold_nova` must be enabled at build time.
  Without these, the governance flag is observed but inert.
- The `RealBlockProver` lazy-init triggers a ~60-90 s `pp` setup on
  first nova-mode fold. **Do not flip the flag at high block-rate
  windows** — schedule for a quiet period.

### Rollout

1. Set the flag on the validator running on the testnet first:

   ```
   curl -X POST http://localhost:8080/api/governance/param \
     -d '{"key": "lambda_fold_mode", "value": "nova"}'
   ```

2. Watch `tendermint::logs` for:
   - `lambda_fold nova path errored; substrate fold stands` — INDICATES
     the lazy-init or fold step failed; the substrate fold continues
     so consensus is not broken, but operators must investigate.
   - First successful fold takes 60-90 s on M4. Subsequent folds are
     ~250-400 ms per block.

3. Confirm via `GET /api/lambda_fold/nova`:
   ```
   { "step_count": <N>, "latest_epoch": <E>, "is_identity": false, ... }
   ```

4. Verify the light-client path:

   ```
   curl http://localhost:8080/api/lambda_fold/nova/vk_bytes
   curl -X POST http://localhost:8080/api/lambda_fold/nova/verify \
     -d '{"expected_remaining_energy": 0}'
   ```

5. Sublinearity check: the `verify_with_vk_bytes` time should be
   essentially constant across fold counts. Empirical numbers (M4
   release): 21 ms @ 10 folds, 23 ms @ 100 folds. If you see >100 ms
   on small fold counts, the prover wasn't built `--release`.

### Rollback

`POST /api/governance/param` with `{"key": "lambda_fold_mode", "value": "hash_chain"}`. The substrate fold continues uninterrupted. The Nova folder's accumulated proofs are retained on-disk (the chain re-uses them on next flip).

---

## Lane 2 — Crooks-MEV refund pipeline

### What changes when active

The chain's per-block sandwich detector starts emitting
`MevObservation` entries. In `enforce` mode, the proposer is REQUIRED
to include `RefundTx` for due observations, and validators reject
blocks that omit them. In `enforce`-with-stake-slash mode, the
`mev_missing_refund_violations` counter actually deducts stake from
proposers who skip refunds.

**Three-step rollout** for safety: observe → enforce → enforce + slash.

### Pre-flight (Lane 2 specific)

- Confirm `evaporchain-mev-detect` crate is in the consensus dep tree.
- Confirm the existing `evaporchain-execution::execute_refund` path
  exists (Phase 3.5a substrate). This is the actual balance movement
  for `Transaction::Refund`. Without it, refund txs would fail
  execution.

### Rollout

#### Step 1 — Observe mode (default, but make it explicit)

```
curl -X POST http://localhost:8080/api/governance/param \
  -d '{"key": "crooks_mev_settlement_mode", "value": "observe"}'
```

Watch `GET /api/mev/observations` — every committed block's sandwich
shapes appear as observations. **No refunds settle, no stake deducted.**

Run for 7-14 days minimum. Watch the false-positive rate. If observers
see honest swap patterns being flagged, flip back; otherwise proceed.

#### Step 2 — Enforce mode (validator rejection)

```
curl -X POST http://localhost:8080/api/governance/param \
  -d '{"key": "crooks_mev_settlement_mode", "value": "enforce"}'
```

Now block proposals MUST include `RefundTx` for due observations
(those past `crooks_mev_grace_period_blocks` and within
`crooks_mev_refund_window_blocks`). Validators reject blocks that
omit them with `MissingRefund`.

The proposer-side `RefundTx` construction is operator/proposer
software responsibility — the chain's `due_refund_txs` accessor
enumerates the required txs.

`mev_missing_refund_violations` counter ticks for any proposer that's
rejected. **No stake deduction yet** — this is policy enforcement
without slashing.

#### Step 3 — Enable stake deduction

After 7+ days of clean enforce-mode operation, flip the slash flag:

```
curl -X POST http://localhost:8080/api/governance/param \
  -d '{"key": "crooks_mev_missing_refund_slash_enabled", "value": "true"}'
```

The `apply_mev_missing_refund_slashes` path now actually deducts
stake via `validator_set::slash_with_amount` (no jail — this is
policy violation, not equivocation). The counter resets per
slashed validator after each application.

### Operator dispute (any active mode)

Within the `crooks_mev_grace_period_blocks` window (default 5
blocks), an operator can dispute a pending refund:

```
curl -X POST http://localhost:8080/api/mev/dispute \
  -d '{"source_block_height": <H>, "source_observation_idx": <I>, "current_height": <C>}'
```

`MevDisputeError::PastGracePeriod` if too late.

### Rollback

Set `crooks_mev_settlement_mode` back to `"observe"`. Counter and ring
buffer continue populating but no settlement runs.

---

## Lane 3 — Light-Cone Full DAG mode

### What changes when active

The chain's tip selection switches from linear `parent_hash` to
DAG-derived `MccForkChoice::select_tip`. Per-fork state branches
populate. Antichain finalization replaces single-tip BFT
finalization. **This is the most behaviourally-different lane** —
flip with caution.

### Pre-flight (Lane 3 specific)

- The `parent_acceptance_mode = "mcc"` flag must already be on (Layer
  4 substrate). If you've been running `linear` mode, flip
  `parent_acceptance_mode` to `mcc` first and observe for 7+ days.
- Confirm cluster has > 3 validators — antichain finality with f=0
  (single-validator) is degenerate.

### Rollout

#### Step 1 — Enable DAG mode

```
curl -X POST http://localhost:8080/api/governance/param \
  -d '{"key": "light_cone_state_branches_enabled", "value": "true"}'
```

State branches populate from `on_block_committed` going forward. The
`light_cone_max_concurrent_forks` cap (default 4) bounds memory.

#### Step 2 — Watch for divergence

The most dangerous failure mode is validators agreeing on the linear
chain but diverging on DAG-mode antichain finalization. Cross-check
across 3+ validators:

```
curl http://localhost:8080/api/state_branches              # validator 1
curl http://other-validator:8080/api/state_branches        # validator 2
```

For a single-numeric inter-validator agreement check, use the Phase
4.4 antichain commit-cert digest — every validator with the same
Light-Cone DAG state produces the same 32-byte digest:

```
curl http://localhost:8080/api/light_cone/antichain_digest        # validator 1
curl http://other-validator:8080/api/light_cone/antichain_digest  # validator 2
```

If `digest` matches across all validators, antichain finality is
agreed. If it diverges, the response also returns the
`closing_antichain` block-id list each validator hashed, so
operators can immediately see *which* blocks differ. Pairs with
Crooks-MEV's `mev_state_digest` (Phase 3.2) — together they cover
both the MEV-state and the Light-Cone-substrate portions of the
inter-validator consensus surface.

#### Step 3 — Tune fork cap if needed

If the cluster sees > 4 concurrent forks under load, raise the cap:

```
curl -X POST http://localhost:8080/api/governance/param \
  -d '{"key": "light_cone_max_concurrent_forks", "value": "8"}'
```

8 is the hard cap; above that, RocksDB snapshot retention starts
pinning LSM tail aggressively (see `PHASE_3_DECISIONS.md` Decision
1's stopping condition).

#### Step 4 — Tune orphan threshold

If long-running forks accumulate uncommitted blocks, set:

```
curl -X POST http://localhost:8080/api/governance/param \
  -d '{"key": "light_cone_orphan_caliber_threshold", "value": "1000"}'
```

`detect_orphan_branches` now flags low-caliber stale tips for
operator inspection.

### Rollback

Set `light_cone_state_branches_enabled` back to `"false"`. State
branches stop populating; the chain falls back to linear-tip
semantics. **Existing finalized antichains stay finalized** — the
flag only gates new finalization.

---

## Lane 4 — MCC full multi-parent enumeration (Phase 8 addendum)

> **Status as of 2026-05-05:** SUBSTRATE COMPLETE behind
> `parent_acceptance_mode = "mcc"` (single-line trajectory walk).
> The `mcc_full` value of this flag — which promotes the
> Light-Cone DAG from co-existing-with-Tendermint to *load-bearing*
> fork choice across multi-parent forks — is **not yet wired into
> the consensus hot path**. This lane describes the *intended*
> rollout; do NOT flip the flag in production until Phase C of
> [`MCC_FULL_MULTI_PARENT_PLAN.md`](../../MCC_FULL_MULTI_PARENT_PLAN.md)
> ships (`authoritative_head` selection at `start_round`, vote
> dispatch by head, proposer multi-parent set selection).

### What changes when active

The chain's fork-choice promotes from single-line MCC trajectory
walk to **full multi-parent enumeration**. At the start of every
consensus round, `authoritative_head` is selected as the argmax of
`enumerate_candidate_heads`. Validators vote on the chosen head;
if the chosen head changes between rounds (branch switch), the
executor calls `replay_and_apply_atomic` to roll state back to the
LCA and forward to the new head atomically. Proposers emit
multi-parent blocks (`block.parents.len() > 1`) when their parent
set is an antichain.

### Pre-flight (Lane 4 specific)

- Lane 3 (Light-Cone Full DAG) must be flipped to
  `light_cone_state_branches_enabled = true` first — Phase 4's
  per-fork state branches are the substrate Lane 4's replay walks
  consume.
- Confirm `state_branches` is populated on every validator:
  `curl http://node:8080/api/light_cone | jq .block_count` should
  match across all validators.
- Confirm every active `state_branch` has an attached snapshot
  (otherwise rollback fails). Today this is wired in
  `evaporchain-node` via `attach_branch_snapshot` calls during
  `on_block_committed`; verify operationally by triggering a fork
  and checking that a subsequent `replay_and_apply_atomic` doesn't
  return `RestoreFailed`.
- Confirm `protocol_version >= 3` chain-wide (multi-parent block
  wire format). Phase 2 of the Light-Cone plan added the
  `Block::parents` field with `serde(skip_serializing_if =
  "Vec::is_empty")` so legacy blocks stay bit-compatible — but
  emitting `parents.len() > 1` requires v3.

### Rollout

Three-step ladder — **never** skip directly from `linear` to
`mcc_full`:

#### Step 1 — Confirm linear baseline

```
curl -X POST http://localhost:8080/api/governance/param \
  -d '{"key": "parent_acceptance_mode", "value": "linear"}'
```

This is the default. Verify by checking
`/api/governance/flags` and `/api/light_cone/candidate_heads`
returns a single-entry list (no concurrent forks under linear
mode).

#### Step 2 — Single-line MCC

```
curl -X POST http://localhost:8080/api/governance/param \
  -d '{"key": "parent_acceptance_mode", "value": "mcc"}'
```

The chain now uses MCC for single-line trajectory walking. Watch
the cluster for at least 7 days — `curl
http://node-N:8080/api/light_cone/authoritative_head` on each
validator and confirm:
- `head` matches across all validators within a few-block window
- `caliber` is non-zero and roughly stable
- `candidates_considered` stays low (1-2; under linear-block-rate
  workload there's typically only one competing head)

If divergence appears in `authoritative_head` reports persisting
beyond 5 blocks, halt — the cluster has a forking issue that needs
investigation before promoting further.

#### Step 3 — Full multi-parent enumeration **(POST-PHASE-C ONLY)**

```
curl -X POST http://localhost:8080/api/governance/param \
  -d '{"key": "parent_acceptance_mode", "value": "mcc_full"}'
```

**Do not run this in production until Phase C of
`MCC_FULL_MULTI_PARENT_PLAN.md` ships.** The substrate is in place
(B.0/B.1/B.2/B.3/B.4/B.5/B.6 — `replay_and_apply_atomic` works
end-to-end) but the consensus hot path (`start_round`, voting
handler dispatch, proposer parent-set selection) does NOT yet
consume it.

Once Phase C lands, this step:
- Routes `start_round` through `enumerate_candidate_heads().argmax()`
- Routes `handle_prevote` / `handle_precommit` to
  `dag_round_states[authoritative_head]`
- Has the proposer emit `block.parents` as the antichain of
  current candidate heads (where `is_antichain` returns true)

### Monitoring

The Lane-4-relevant operator endpoints to watch:

```
curl http://node:8080/api/light_cone/candidate_heads
  # All competing heads + calibers, sorted descending.

curl http://node:8080/api/light_cone/authoritative_head
  # The MCC-chosen head this validator is building on.

curl http://node:8080/api/light_cone/antichain_digest_history
  # Past 128 blocks of antichain-digest pairs — cross-compare
  # across cluster validators to detect any historical divergence.

curl http://node:8080/api/light_cone/block_clock/<hex_block_id>
  # Decay-Lamport clock at a specific DAG block (shipped 2026-05-06).
  # Pure function of (DAG, block_id, tick_quantum). Pin a known
  # fork point, ask each validator: equality across all 4
  # validators is the substrate-level convergence claim;
  # inequality is either DAG drift OR tick_quantum mismatch.
  # The response surfaces tick_quantum so operators can
  # distinguish those two failure modes.

curl http://node:8080/api/lamport_time
  # Chain-global running clock — pairs with block_clock.
  # If chain-global ticks agree across nodes but per-block
  # clocks at the antichain head disagree, the divergence is
  # in the DAG itself, not the clock.
```

For cluster-divergence diagnosis:

```
# Authoritative-head agreement (fast).
for node in node1 node2 node3 node4; do
  echo "--- $node ---"
  curl -s http://$node:8080/api/light_cone/authoritative_head | jq -r .head
done

# Antichain-digest agreement at a specific past height.
HEIGHT=12345
for node in node1 node2 node3 node4; do
  echo "--- $node ---"
  curl -s http://$node:8080/api/light_cone/antichain_digest_history \
    | jq -r ".history[] | select(.block_height == $HEIGHT) | .digest"
done

# Per-block clock agreement (deepest substrate check). Pick a
# known fork-point block_id and ask every validator for its
# Decay-Lamport clock.
BLOCK=0x<hex_block_id>
for node in node1 node2 node3 node4; do
  echo "--- $node ---"
  curl -s http://$node:8080/api/light_cone/block_clock/$BLOCK \
    | jq -c '{tick: .current_tick, accum: .accumulated_energy, q: .tick_quantum}'
done
```

The three checks form a layered diagnosis ladder:

1. **Authoritative-head agreement** — fastest signal. If heads
   disagree for more than a few blocks, the cluster is forking
   live; `replay_and_apply_atomic` is the recovery path under
   `mcc_full`.
2. **Antichain-digest agreement** — historical signal. If past
   digests diverged at any height, that's a freeze-class flag
   even if the live state has converged.
3. **Per-block clock agreement** — deepest substrate signal. If
   clocks at the same block_id differ, either the DAG topology
   below that block diverged OR validators are running with
   different `tick_quantum`. The response surfaces `tick_quantum`
   for ops to disambiguate.

### Rollback

```
curl -X POST http://localhost:8080/api/governance/param \
  -d '{"key": "parent_acceptance_mode", "value": "mcc"}'
```

Returns to the single-line MCC trajectory walk. Existing
state_branches stay populated; replay machinery becomes idle.
Subsequent rollback to `linear` is bit-immediate via the same
flag-flip path.

### 72-hour cluster soak (Phase D.5 of MCC_FULL_MULTI_PARENT_PLAN)

> **Status as of 2026-05-05:** Substrate-level soak gates D.1
> through D.4 are GREEN (10 normal + 4 perf-budget + 2 substrate-
> soak tests pass on Mini under release; perf budgets cleared by
> 45-294×). The 72hr operational soak below is the **only**
> remaining D-item before the MCC plan is fully done.

The synthetic in-test soak (`mcc_phase_d5_*` in
`tests/mcc_phase_d.rs`, `--ignored` flagged) validates the
substrate doesn't drift under 5,000 block insertions × 4 active
forks × 5 hot-path accessor calls per insertion (=25,000 calls)
in ~40 ms with zero stalls and zero antichain-digest divergence
across 4 simulated validators. That's the CI gate.

The operational gate is a 72-hour cluster soak on the 4-validator
Mini setup. The substrate is hardened; this is the
network-layer + gossip + execution-layer integration test.

**Pre-flight:**

1. Confirm the chain is at `parent_acceptance_mode = "mcc"` AND
   `light_cone_state_branches_enabled = true` for ≥7 days with
   no divergence (Lane 3 + Lane 4 step 2 baselines).
2. On all 4 validators, snapshot `git log --oneline | head -1` —
   the binary version must match across the cluster. Ship via
   `cargo build --release` on Mini, scp to all 4.
3. Capture a 4-hour LINEAR baseline first:
   ```
   for node in v1 v2 v3 v4; do
     curl -X POST http://$node:8080/api/governance/param \
       -d '{"key": "parent_acceptance_mode", "value": "linear"}'
   done
   # 4 hours pass; record block-rate, finality-gap, mempool depth.
   ```
4. Promote to `mcc_full`:
   ```
   for node in v1 v2 v3 v4; do
     curl -X POST http://$node:8080/api/governance/param \
       -d '{"key": "parent_acceptance_mode", "value": "mcc_full"}'
   done
   ```

**Soak measurements (every hour for 72 hours):**

| Metric | Endpoint | Pass criterion |
|---|---|---|
| Stall events | `chain.height` advancement | 0 stalls (height monotone-increasing across all 4 nodes) |
| Antichain-digest divergence | `/api/light_cone/antichain_digest_history` | 0 divergent entries between any pair of nodes |
| Throughput | `block_rate / sec` | ≥ 95% of linear baseline (the < 5% degradation budget) |
| Equivocation false-positives | `/api/light_cone/cross_fork_equivocations` | 0 across all 4 nodes (nobody is byzantine in the soak) |
| Memory | `state_branches_count` | Bounded by `light_cone_max_concurrent_forks` (default 4); does not grow unboundedly |
| Replay-failure rate | log scrape: `RestoreFailed` / `BlockNotFound` / `ApplyFailed` | 0 across all 4 nodes |

**Cluster-wide divergence diagnosis:**

```
for node in v1 v2 v3 v4; do
  echo "--- $node ---"
  curl -s http://$node:8080/api/light_cone/antichain_digest \
    | jq -r '.digest_hex'
done
```

All 4 must print the same digest at the same chain height.

**Pass:** all metrics above hit budget for the full 72 hours.
Operator marks D.5 ✅ and promotes the doctrine punch-list Layer
4 row to its load-bearing-active state.

**Fail:** any single metric breach. Halt soak immediately, flip
back to `mcc` (single-line MCC), file an incident with the
`/api/light_cone/antichain_digest_history` JSON dump from all 4
nodes attached. The substrate's atomic-replay machinery means
the chain itself doesn't get poisoned by the soak — what's at
risk is the *correctness contract* under network-induced jitter,
which the substrate-level soak cannot exercise.

---

## Composition

The three lanes are designed to compose:

| Lane combination | What you get |
|---|---|
| Lambda-Fold only | Sublinear-time light-client chain verification (no MEV / DAG semantics changes) |
| Crooks-MEV only | Sandwich-attack restitution + slashing (linear chain unchanged) |
| Light-Cone only | DAG consensus + antichain finalization |
| Lambda-Fold + Crooks-MEV | Default-light-client + MEV defense (most operators' first state) |
| All three | Frontier-grade chain end-to-end |

There is **no composition risk** — each lane operates on a different
substrate (proving / mempool-detection / consensus-state). Activating
all three is the doctrine's intended end-state.

---

## Emergency rollback (any lane, any state)

All three lanes are rollback-safe via `governance_set_param`:

```
curl -X POST http://localhost:8080/api/governance/param -d '{"key": "lambda_fold_mode",                           "value": "hash_chain"}'
curl -X POST http://localhost:8080/api/governance/param -d '{"key": "crooks_mev_settlement_mode",                 "value": "observe"}'
curl -X POST http://localhost:8080/api/governance/param -d '{"key": "crooks_mev_missing_refund_slash_enabled",    "value": "false"}'
curl -X POST http://localhost:8080/api/governance/param -d '{"key": "light_cone_state_branches_enabled",          "value": "false"}'
```

Each flip is bit-immediate — chain returns to the previous behaviour
on the next block. No state migration, no chain restart needed.

The `evaporchain-consensus` test suite includes regression tests
locking the bit-compat contract for every flag's default-off state
(`test_state_branches_starts_empty_and_flag_off_keeps_empty`,
`test_lambda_fold_nova_mode_no_op_without_feature`,
`test_governance_lambda_fold_mode_default_hash_chain`, etc.).
