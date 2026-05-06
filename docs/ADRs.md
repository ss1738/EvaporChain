# Architecture Decision Records — EvaporChain

This document captures non-obvious design decisions made during the
2026-04-27 hardening session. Each ADR records:

- **Context** — what was the problem.
- **Decision** — what we chose.
- **Alternatives considered** — what we rejected and why.
- **Consequences** — what this enables and what it costs.

ADRs are immutable historical records — when a decision is later revised,
add a new ADR that supersedes the old one. Don't edit existing ADRs to
match new realities; that loses the rationale for *why* the original
decision was made.

---

## ADR-001: DecayingDAO contract + execution-layer floor bounds

**Date:** 2026-04-27
**Status:** Accepted, implemented

### Context

The original `execute_governance` `put_governance_param` path accepted
arbitrary `(key, value)` strings with no validation. Combined with vote
weight = balance, no quorum, and a Foundation Treasury holding 35% of
genesis supply, a single Foundation vote could pass any proposal solo,
including setting `block_gas_limit = u64::MAX` (which halts the chain).

### Decision

Two-layer governance:

- **Constitutional layer (execution)**: `validate_governance_param` enforces
  immutable floor bounds for security-critical keys. Only changeable by
  hard fork.
- **Statutory layer (contract)**: `DecayingDAO` contract template enforces
  per-deployment `param_bounds`, vote-weight cap (`min(balance, stake)`),
  quorum, and timelock. Tunable by deploying a new DAO instance with
  different parameters.
- **Bridge**: `apply_dao_governance` reads `ReadyToApply` proposals from
  a DAO contract and applies them to execution-layer governance state,
  validating against the floor bounds along the way.

### Alternatives considered

1. **Pure execution-layer hard-coded bounds.** Simple but inflexible —
   every new tunable parameter needs a node release. Rejected: this is
   what makes Ethereum governance painful.
2. **Pure contract-layer governance.** More flexible but unsafe — a
   buggy or compromised DAO contract would be the only barrier.
   Rejected: no defense-in-depth.
3. **Snapshot-style stake-weighted vote without timelock.** Faster
   pass-to-apply but rug-pullable. Rejected: timelock is cheap; no-rug
   is non-negotiable for governance.

### Consequences

- Two-layer is the standard "constitutional + statutory" pattern — bounds
  can be tightened by governance but never widened past the constitution.
- Adds the `apply_dao_governance` bridge as a new public surface on the
  executor; operator-invoked, not transaction-driven (avoids cascading
  changes to the Transaction enum and dispatchers).
- `DecayingDAO` proposals decay if nobody refreshes them — genuinely
  decay-native; no other production DAO has this property.

---

## ADR-002: Logged-exit for persistence failures (not Result propagation)

**Date:** 2026-04-27
**Status:** Accepted, implemented

### Context

Persistence operations (`persist_object` / `persist_account` / `persist_ghost`
in `evaporchain-state/src/rocksdb_backend.rs`) called `.expect("write X
to RocksDB")` on the underlying RocksDB write, which panics with a
generic message and exit status 101 on disk-full / permission-revoked /
other I/O failures.

### Decision

Replace `.expect(...)` with a `fatal_persistence_error(op, error)` helper
that:
1. Logs `tracing::error!` with structured fields (operation name, underlying error)
2. Sleeps 100ms for log flush
3. Calls `std::process::exit(2)` to halt the node cleanly

### Alternatives considered

1. **Propagate `Result<(), PersistError>` up through the call stack.**
   The right thing in principle, but the public `StateDB` trait methods
   (`put_object`, `put_account`, etc.) return unit. Adapting the trait
   surface cascades through every caller across the workspace
   (execution, consensus, oracle, sharding, every test fixture).
   Rejected as too invasive for the security benefit.
2. **Catch the panic via `std::panic::catch_unwind`.** Recoverable but
   leaves the node in an inconsistent in-memory state (the panic
   happened mid-block-apply). Rejected: the node should halt, not
   continue.

### Consequences

- Operators see a structured log line instead of a generic panic message.
- Exit status 2 distinguishes persistence failures from programmer-error
  panics (status 101).
- On restart, the in-flight block is replayed by consensus; no
  inconsistent state survives because the failed write never persisted.
- The audit-named-sites and the broader sweep (8 + 13 = 21 total
  `.expect()` sites) all use the same helper.

---

## ADR-003: Auto-derive `storage_bytes_charged` in deploy methods (not pass-through)

**Date:** 2026-04-27
**Status:** Accepted, implemented

### Context

Storage rent accounting needs to know how many bytes to credit back when
a contract or script evaporates. The credit-back path runs in the
execution layer (`credit_back_evaporated_contracts` /
`credit_back_evaporated_scripts`). It needs the exact byte count
charged at deploy time.

### Decision

Add `pub storage_bytes_charged: u64` to `ContractInstance` and
`ScriptContract` with `#[serde(default)]`. Set it inside the deploy
method:

- `ContractEngine::deploy`: `serde_json::to_string(&params).len()` —
  approximate (whitespace differs from the original `tx.init_args` string).
- `ScriptEngine::deploy`: `source.len()` — exact match to what the
  execution layer credits.

### Alternatives considered

1. **Add a `bytes_charged: u64` parameter to `deploy()`.** Exact
   accounting via caller-side knowledge. Rejected: cascades through 3
   production callers + tests; signature change is invasive.
2. **Add a public `set_storage_bytes_charged(id, bytes)` method.** Caller
   would call it after `deploy()`. Rejected: race-prone (deploy could be
   queried between the call and the set), more API surface.
3. **Compute at evaporation time from current state.** What the prior
   approximation did. Rejected: state can drift from init_args
   significantly (state mutations during the contract's lifetime).

### Consequences

- For scripts: exact accounting (deploy charge = `source.len()` =
  evaporation credit-back).
- For contracts: small bounded drift between deploy charge
  (`tx.init_args.len()`) and credit (`serde_json::to_string(parsed_params).len()`)
  — typically a few bytes of whitespace difference. `saturating_sub`
  absorbs any underflow.
- `#[serde(default)]` keeps legacy on-disk contracts deserializable;
  their `storage_bytes_charged = 0` so credit-back is a no-op (correct
  — they were never charged either).

---

## ADR-004: Cross-key validator separate from single-key validator

**Date:** 2026-04-27
**Status:** Accepted, implemented

### Context

`validate_governance_param(key, value)` enforces single-key floor bounds
(e.g., `block_gas_limit ∈ [10_000, 100_000_000]`). But the cross-key
invariant `base_fee_floor < base_fee_ceiling` requires reading the OTHER
side from state — which the single-key signature can't do.

### Decision

Add a separate function `validate_governance_param_against_state(db, key, value)`
that:

1. Calls the single-key `validate_governance_param(key, value)` first.
2. Then, for cross-key invariants, reads the other side from `db.get_governance_param`.
3. Skips the cross-key check when the other side is unset (executor falls
   back to its compiled-in default).

The 4 production callers of the validator switched to the cross-key
version. Tests for the single-key version remained, since they don't
need a `db` argument.

### Alternatives considered

1. **Add a `db` parameter to `validate_governance_param`.** Breaks the
   pure-function shape and forces all callers (including tests) to pass
   a db. Rejected: tests should be able to validate without a state
   harness.
2. **Validate cross-key invariants only at `apply_governance_params`.**
   Defense-only at the read-back time. Rejected: bad pairs would still
   be stored in db (just not applied). Better to reject at write time.

### Consequences

- Two validator functions, but with a clear hierarchy: cross-key wraps
  single-key.
- The cross-key version is the "production" path; the single-key remains
  for unit tests and forward-compat (additional cross-key constraints
  can be added by extending the wrapper without touching the single-key
  table).
- A defense-in-depth check in `apply_governance_params` skips the apply
  if the floor/ceiling pair is somehow inconsistent — covering legacy
  state and genesis-time inconsistencies that bypassed the gates.

---

## ADR-005: Canonicalize-at-init for owner addresses (not args-only)

**Date:** 2026-04-27
**Status:** Accepted, implemented

### Context

Multiple contract templates store an owner address as `String`
(`TokenState.owner`, `TemporalState.owner`, `EscrowState.sender`/`receiver`,
`AuctionState.seller`, `NftInfo.owner`). Comparisons against
`hex::encode(caller)` (always lowercase, no prefix) failed if the stored
string had different case or a `0x`/`0X` prefix.

### Decision

Two-pronged fix:

1. **At init time**, parse each owner-address arg through
   `canonicalize_address_hex(s)`: trim whitespace, strip `0x`/`0X`
   prefix, lowercase, validate length 64 + hex chars. The stored string
   is always canonical post-this-fix.
2. **At comparison time**, use `eq_ignore_ascii_case` as defense-in-depth
   for any pre-fix instance with a non-canonical stored owner.

Args-supplied address strings (per-call user input) also canonicalize at
the start of the exec method.

### Alternatives considered

1. **Change `owner` field type from `String` to `AccountAddress` (32-byte
   array).** More type-safe but breaks JSON serialization compatibility
   with existing on-disk state. Rejected as too invasive.
2. **Comparison-only `eq_ignore_ascii_case` (skip init canonicalization).**
   Handles case but not `0x` prefix. Rejected: deployer passing
   `"0xABC..."` would still mismatch `caller_hex = "abc..."`.
3. **One-shot migration script** to canonicalize all stored owner
   strings. Useful for legacy data but doesn't help going forward.
   Rejected as out of scope; legacy data is on the testnet only.

### Consequences

- Every owner-string field is canonical post-deploy/-mint/-transfer.
- Malformed addresses (wrong length, non-hex chars) are rejected at the
  API boundary instead of silently propagating.
- For DecayingToken specifically, the `balances` HashMap uses the
  canonical owner as key — depositing under one casing and withdrawing
  with another now consistently uses the same map entry. **This is a
  behavior change** for any pre-fix testnet state that relied on
  case-sensitive map lookups.

---

## ADR-006: Block-STM optimistic path serializes deploys (no parallel deploy)

**Date:** 2026-04-27
**Status:** Accepted (pre-existing, formalized in this session)

### Context

Block-STM's optimistic execution path partitions transactions by access
keys. Every contract / script deploy touches a globally shared
`ContractEngine` / `ScriptEngine`, which would force every deploy into
the same partition — defeating parallelization.

### Decision

`Transaction::DeployContract`, `Transaction::DeployScript`,
`Transaction::CallContract`, `Transaction::CallScript`, and the privacy
transactions all explicitly return `Err("contract/script/...txs execute
in serial phase")` from the optimistic path. They route to the serial
fallback executor unconditionally.

### Alternatives considered

1. **Add a per-contract-engine partition.** Would still serialize
   internally; saves no parallelism. Rejected.
2. **Use a sharded contract engine.** Real research direction but bigger
   redesign. Rejected for v0.1.

### Consequences

- Storage rent charging via the serial-fallback path covers all deploys —
  the optimistic path never executes them.
- The "Block-STM optimistic deploy charging" gap that initially looked
  like a real refactor turns out to be closed-by-design.
- For high-throughput deployments at scale, parallelizing contract
  execution remains an open research direction.

---

## ADR-007: TLA+ as the formal-spec target (not Coq/Lean for v0.1)

**Date:** 2026-04-27
**Status:** Accepted

### Context

Three frontier primitives (PoHA, Energy-Verkle Trie, Rule-Based Consensus)
plus the existing Tendermint BFT consensus need formal correctness
arguments before external audit. Auditors weight formal verification
heavily; the question is which formal-method tool to use.

### Decision

TLA+ for state-machine-level invariants (lifecycle correctness, quorum
preservation, energy monotonicity, etc.). Coq / Lean as the open follow-up
for unbounded statements that TLC's bounded checking can't cover.

### Alternatives considered

1. **Coq / Lean for everything.** More rigorous but multi-week to
   multi-month per spec. Rejected for v0.1: too slow, too many tools.
2. **No formal verification at all, rely on tests + audit.** Standard
   practice for many L1s. Rejected: EvaporChain's INEVITABILITY_STRATEGY
   names formal verification as a key credibility lever.
3. **Property-based testing only (proptest, fuzzing).** Already in
   place in the codebase. Useful but doesn't capture state-machine
   invariants the same way.

### Consequences

- Four TLA+ modules in `research/tla/`: existing `EvaporChainBFT.tla`
  plus this session's `RuleBasedConsensus.tla`, `EnergyVerkleTrie.tla`,
  `PoHA.tla`. Each has a `.cfg` and a proof-companion `.md`.
- Each proof companion explicitly notes what's bounded-checked vs what's
  open (for the open theorems, including the algebraic-commitment
  theorem for Energy-Verkle and the freeloading-resistance theorem for
  PoHA).
- Auditors get a clean reading order: design rationale → TLA+ spec →
  proof companion → audit-readiness pack with known issues.

---

## How this catalog evolves

When you make a non-obvious design decision in a future session, add an
ADR (next ADR-NNN). Don't edit existing ADRs; if a decision is revised,
write a new ADR that explicitly **supersedes** the old one and link both
ways.

Keep ADRs short. The goal is searchable rationale, not exhaustive
documentation.
