# Executor parity harness — multi-week plan

**Owner:** Opus 4.7 session starting 2026-05-13.
**Driver:** AUDIT_2026_05_13.md cross-cutting Theme B.
**Closes structurally:** C3, C4, H9 from the same audit, plus any new divergences
the harness surfaces along the way.

---

## 1. Why this work

3 of the 5 CRITICALs and 1 of the 13 HIGHs in AUDIT_2026_05_13.md trace to one
root cause: `ParallelExecutor` (the production hot path per CLAUDE.md) was
never brought to parity with `SimpleExecutor` across the 25 `Transaction`
variants. Finding-by-finding closure is necessary but not sufficient — the
class will recur with the next new tx variant unless we make divergence
**structurally impossible**.

A parity harness asserts: *for every `(initial_state, transaction, epoch)`
tuple in the test matrix, the post-state under `SimpleExecutor` equals the
post-state under `ParallelExecutor` byte-for-byte across every account,
stake, delegation, sentinel, governance proposal, vesting schedule, refresh
pool, nullifier set, and note commitment.*

A divergence is either a SimpleExecutor bug, a ParallelExecutor bug, or
both. Each one becomes a finding + fix.

## 2. Architectural shape

The harness lives in `crates/evaporchain-execution/tests/parity_harness.rs`
(integration test, not a sub-crate — both executors and the `StateDB` impls
are already in scope here without new workspace members).

```rust
struct ParityFixture {
    name: &'static str,
    build_initial_state: fn(&mut StateDB),  // both executors start from the same state
    transaction: Transaction,
    epoch: u64,
    governance_flags: HashMap<String, String>,
}

#[derive(Debug)]
struct Divergence {
    domain: &'static str,                   // "accounts" / "stakes" / "nullifiers" / ...
    detail: String,
    simple_value: String,
    parallel_value: String,
}

fn run_parity(fixture: &ParityFixture) -> Result<(), Vec<Divergence>>;
```

Comparator domains (in order of precedence):

1. `execute_block` return value (success / error variant + error message).
2. `accounts` — every address present in either DB, balance + nonce + vesting.
3. `stakes` — every validator id, full `StakeRecord`.
4. `delegations` — every (delegator, validator) pair, full `Delegation`.
5. `sentinel_params` + `sentinel_votes` — entire maps.
6. `governance_proposals` + `governance_params` — entire maps.
7. `refresh_pool` — total balance per namespace.
8. `objects` — every object id, full record.
9. `ghosts` — every ghost id, full record.
10. `spent_nullifiers` — full set.
11. `note_commitments` — full set.
12. `state_root` — final root from each DB.

Each divergence is reported (not just the first). The test fails with the
full list so one parity run surfaces every misalignment, not just the
earliest.

## 3. Phase plan

### Phase 1 — Scaffolding + Transfer baseline *(this PR)*

- `parity_harness.rs` with `ParityFixture` + `Divergence` types + `run_parity()`.
- StateDB-clone helper so both executors start identical.
- Comparator covering accounts / stakes / delegations (the 3 most commonly
  mutated maps).
- First fixture: `Transfer` with a positive balance. Known-working in both
  executors; a green run here proves the harness itself is sound.

### Phase 2 — Validator-set parity *(closes C3)*

- Fixture: `ValidatorStake { stake_amount, ... }` with sufficient balance.
- **Expected divergence:** SimpleExecutor produces `db.get_stake(vid) ==
  Some(record)`; ParallelExecutor produces `db.get_stake(vid) == None` and
  the deployer's balance debit happens anyway.
- Fix: move `ValidatorStake` into `parallel.rs:1535-1554` serial-bucket
  filter; port `SimpleExecutor::execute_validator_stake` (lib.rs:1668-1683)
  into the serial loop body around line 1857; implement
  `OverlayStateDB::put_stake` / `get_stake` (parallel.rs:467-470) properly.
- Mirror fixtures for `ValidatorExit` and `ValidatorClaimStake` (already
  in serial bucket — should be green; parity verifies).

### Phase 3 — 7 blackholed tx types *(closes C4)*

- One fixture per type: Governance / MultiSig / UserOp / UpgradeContract /
  Delegate / Undelegate / ClaimDelegation.
- **Expected divergence on each:** ParallelExecutor returns
  `Err("<class> txs execute in serial phase")`; SimpleExecutor succeeds.
- Fix per arm: port the corresponding `SimpleExecutor::execute_*` body
  from lib.rs:3132-3214 into the ParallelExecutor serial loop.
- **Invert the existing T1.20 tests at parallel.rs:4631-4720** —
  `t1_20_governance_via_parallel_partition_fails` and siblings currently
  assert `r.txs_failed == 1`; rewrite to assert success + correct
  state mutation.

### Phase 4 — Privacy variants

- Fixtures: `Shield`, `Unshield`, `PrivateTransfer` against a bootstrapped
  nullifier set + note tree.
- Spot-check whether C5 (shielded pool ∉ conservation domain) can be
  closed in scope or needs a separate design call. If `conservation_
  enforcement="enforce"` causes parity to fail on both executors equally,
  it's a separate fix not a parity fix.

### Phase 5 — Contract + tail variants

- DeployContract, CallContract, DeployScript, CallScript,
  RotateValidatorKey, Refund, Deferred.
- Add a `#[cfg(test)]` enum-exhaustiveness check using a match expression
  over `Transaction` that fails to compile if a new variant lands without
  a parity fixture. This is the recurrence-prevention mechanism.

### Phase 6 — Adversarial fixtures + flag-flip matrix

- For each tx variant, add adversarial fixtures: insufficient balance,
  expired vesting, mid-cliff, nonce mismatch, conservation-violating
  amounts, malformed signatures, oversized payloads.
- Cross-product with governance flags:
  - `conservation_enforcement ∈ {observe, enforce}`
  - `parent_acceptance_mode ∈ {linear, mcc, mcc_full}`
  - `block_source_mode ∈ {fifo, antichain}`
  - `light_cone_state_branches_enabled ∈ {false, true}`
  - `lambda_fold_mode ∈ {hash_chain, nova}`
  - `crooks_mev_settlement_mode ∈ {observe, enforce}`
- Every flag-flip path must produce parity. This is what closes the
  bug class permanently.

## 4. Acceptance criteria for the whole arc

1. `cargo test -p evaporchain-execution --test parity_harness` runs all
   25 Transaction variants × representative + ≥3 adversarial fixtures each
   × at least the 2 settings of `conservation_enforcement`. Green.
2. Removing any single port (revert just the SimpleExecutor-arm copy
   from ParallelExecutor) causes a specific named parity test to fail.
   Tested via a once-only revert + re-test before final merge.
3. A new `Transaction` variant added without a parity fixture fails to
   compile.

## 5. Estimated arc duration

- Phase 1: this PR (today, ~half-day code).
- Phase 2: 1 PR (~half-day) — but the OverlayStateDB stake-method
  implementation may grow; budget 1 day.
- Phase 3: 3-4 PRs (one per cluster of tx types) — ~3-4 days.
- Phase 4: 1-2 PRs — ~1 day if no surprises, more if nullifier-set
  divergence surfaces.
- Phase 5: 2-3 PRs — ~2 days.
- Phase 6: 4-6 PRs (adversarial first, then per-flag matrix) — ~1 week.

Total: ~3 weeks of dedicated work, assuming no parallel-session
collisions on `parallel.rs` or `lib.rs`.

## 6. Risks

- **Parallel session on T0.1** (Layer 4 consensus surgery) may indirectly
  touch executor surfaces via consensus integration. Coordinate via
  MAINNET_READINESS lane-claim board.
- **OverlayStateDB stub explosion:** several methods are `{}` today
  (`put_stake`, `put_sentinel_*`, possibly others). Implementing them
  properly may expand into a separate STATE-DB lane.
- **Conservation-domain churn for C5** during Phase 4 — if the shielded
  pool fix lands while parity is mid-build, fixtures need updating.

## 7. Cross-references

- `AUDIT_2026_05_13.md` — findings C3, C4, H9 + Theme B (§6).
- `crates/evaporchain-execution/src/lib.rs:548` — `SimpleExecutor`.
- `crates/evaporchain-execution/src/parallel.rs:567` — `ParallelExecutor`.
- `crates/evaporchain-execution/src/parallel.rs:1535-1554` — serial-bucket
  filter (Phases 2 + 3 modify this).
- `crates/evaporchain-execution/src/lib.rs:3132-3214` — SimpleExecutor's
  arms for the 7 blackholed tx types (Phase 3 ports these).
- `crates/evaporchain-execution/src/parallel.rs:4631-4720` — T1.20 tests
  that currently codify the bug (Phase 3 inverts these).
