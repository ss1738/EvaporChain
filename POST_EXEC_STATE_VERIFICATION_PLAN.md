# Post-Execution State Root Verification — Design Plan

**Status:** Draft, awaiting review.
**Date:** 2026-05-07.
**Trigger:** Today's M1 incident (cluster soak h≈15872) revealed that EvaporChain's consensus does not commit to the post-execution state root. A node with corrupt local state silently forks while only its votes are rejected by quorum. That's a doctrine-level safety gap — the chain claims BFT safety but cannot detect or punish state divergence between honest-protocol nodes.

## What's actually wrong today

Three observations, each from reading the running code:

1. **The block header's `state_root` is the parent's post-exec state, not the new block's.** Proposer at `tendermint.rs:6179` sets `state_root: self.current_state_root` — that's the state *before* applying this block's transactions. After local execution, every node overwrites `block.state_root` in place at `main.rs:4301` with `result.execution.state_root`. The wire field is overloaded: pre-execution as gossiped, post-execution as persisted.

2. **No `apply_block` / `execute_block` path verifies that local post-execution state matches anything.** Today's failed `c9b65a3` patch tried to compare `execution.state_root` against `block.state_root` — but as (1) shows, those are *supposed* to differ. There is no claim to compare against. Reverted in `6cbb51f`.

3. **The `commit_certificate` signs `block_hash`, which covers only the wire-format header — i.e. parent's post-exec.** No validator signature ever covers this block's post-execution state. So a 4-of-5 quorum signing the cert makes no statement about post-exec state agreement.

The result: M1 diverged from canonical state at h=15872 (zero txs, just block-reward + per-block bookkeeping) and silently committed ~80 blocks with a different state_root before we noticed via the dashboard.

## Two viable shapes

### Option A — Proposer-side post-exec commitment (recommended)

The proposer executes the block locally before broadcasting and includes the resulting post-execution state root in the proposal. Validators run their local execution before precommitting and refuse to sign if their post-exec doesn't match the proposer's claim.

**Wire change:** add `Block.post_state_root: Option<[u8; 32]>` (with `#[serde(default, skip_serializing_if = "Option::is_none")]` for bit-compat). Field is the post-exec state root claimed by the proposer.

**Validator flow:**
```
on_proposal(block):
    execution = execute_block_locally(block)
    if block.post_state_root.is_some()
       && block.post_state_root != Some(execution.state_root)
    {
        warn-or-reject  // governed by flag
        prevote NIL
    }
    else { proceed }
```

**Block-hash inclusion:** roll `post_state_root` into the bytes hashed for `block_hash`, so the commit cert's BLS-aggregate signature implicitly covers post-exec agreement once enforcement is on. Behind a fork-epoch gate to keep legacy blocks bit-compatible.

**Pros:** one round-trip (no extra phase), strong guarantee (covered by commit cert).
**Cons:** wire-format change; needs fork-epoch coordination.

### Option B — Two-phase post-commit attestation

Keep the current proposal flow as-is. After a block commits via the existing path, validators sign a second cert over `(block_hash, post_state_root)` and gossip it. A node ingests this second cert and reclassifies blocks where its local post-exec disagrees with ≥2f+1 stake.

**Pros:** no wire-format change to `Block`; can be added as an additive observability primitive first, then made consensus-blocking later.
**Cons:** extra gossip round per block (latency cost); divergent node still commits its bad state for one block before the second-phase cert arrives — recovery is reorg-then-replay rather than don't-commit.

### Recommendation

**Option A.** The cost of (B) is paying every block forever for a check that should have been part of the original cert; (A) is the right shape and the wire-format change is small. Roll out via three governance modes:

1. `post_state_verify_mode = "off"` — current behaviour, field absent or unread.
2. `post_state_verify_mode = "warn"` — proposer fills field, validators check locally and `warn!` on mismatch but still precommit. Diagnostic-only. Default after this lands.
3. `post_state_verify_mode = "enforce"` — validators prevote NIL on mismatch. Activates at a governance-set fork epoch, after `warn` mode has stayed clean across the cluster soak.

## Implementation phases

1. **Phase 1 — wire field.** Add `post_state_root: Option<[u8; 32]>` to `Block`, `#[serde(default, skip_serializing_if = "Option::is_none")]`. Bit-compat regression test. Existing blocks without the field deserialize with `None`.
2. **Phase 2 — proposer fill.** Local execution before proposal broadcast. The proposer already runs `executor.execute_block` once on its own block before sending — wire that result's `state_root` into `post_state_root`. Don't change `block.state_root` semantics yet (still parent-state). Test: proposer-built blocks have `Some(...)`, sync-bootstrapped blocks reaching old paths have `None`.
3. **Phase 3 — warn-mode validator check.** In `apply_block` and `execute_block`, after local executor returns Ok, if `block.post_state_root` is `Some` and != `execution.state_root`, log a structured `WARN` with both roots, the height, and the proposer id. Do NOT reject. Behind `post_state_verify_mode != "off"`.
4. **Phase 4 — enforce-mode prevote rejection.** When mode is `enforce`, mismatch causes the validator to prevote NIL for this round. Round timeout advances; next proposer's block gets the same check. Behind a fork-epoch governance lever.
5. **Phase 5 — block-hash inclusion.** Roll `post_state_root` into the bytes hashed for `block_hash`. Behind the same fork-epoch gate as Phase 4 — once flipped, the field is consensus-load-bearing. Pre-fork blocks keep computing `block_hash` with the legacy formula (fold them into the formula via `if block.post_state_root.is_some() { include } else { skip }`).

## Backwards-compatibility / fork-epoch coordination

- Phases 1–3 are bit-compatible: chain advances exactly as before, just emits warnings.
- Phase 4 is consensus-affecting but governance-flag-gated. Operators flip it only after Phase 3 has stayed clean for an agreed soak window (proposed: 14-day private decay-proven soak).
- Phase 5 changes block-hash; needs a fork-epoch in genesis-amendment style. Ship in lockstep with Phase 4 once we trust the warn signal.

## Open questions for review

1. **Should this commit also cover MMR root?** `BlockExecutionResult` has `mmr_root` distinct from `state_root` — same logic for under-commitment applies. Probably yes, but adds another field.
2. **Does this interact with Lambda-Fold Nova IVC?** The IVC commits to a chain of state roots already; Phase 4 enforcement should make Nova's `vk` strictly more binding (it already verifies post-exec roots; now those roots are also consensus-binding). No conflict expected; worth confirming with the Lambda-Fold lane owner.
3. **Operator UX when a node's post-exec disagrees.** `warn`-mode log alone is too quiet; we likely want a dashboard surface (similar to peer-count) so operators see "post-exec mismatch in last N blocks: K". Filed as a follow-up dashboard ticket.
4. **MEV-detect / Crooks refund interaction.** The post-execution state already includes the MEV-refund executor's mutations (Phase 3.5 of `CROOKS_MEV_INTEGRATION_PLAN.md`). Verification covers it for free. Note for clarity, no action.

## Out of scope for this plan

- Diagnosing *what specifically* diverged on M1 between h=15871 and h=15872. The chain protocol shouldn't depend on knowing every divergence cause — it should detect any of them. M1's specific bug remains an open investigation but is no longer a safety blocker once Phase 4 is on.
- Re-bootstrapping M1 to canonical state. Operational, separate ticket.
- Generalising to arbitrary post-block commitments (e.g., light-client friendly headers). Phase 5's block-hash inclusion is the minimum; richer header schemas are a future Lambda-Fold integration.
