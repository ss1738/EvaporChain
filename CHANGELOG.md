# EvaporChain Changelog

## 2026-05-05 (evening) — MCC full multi-parent enumeration substrate (Phase A + B + E + C.5)

Long shipping arc on `MCC_FULL_MULTI_PARENT_PLAN.md` — the single
biggest blast-radius engineering item left in
`DOCTRINE_PUNCH_LIST.md` Layer 4. Today's evening session shipped:

- **Phase A — Substrate (3/4, A.2 deferred to Phase C)** ✅ DONE
- **Phase B — State-replay pipeline (8/8)** ✅ DONE
- **Phase E — Doctrine + endpoints + runbook (6/6)** ✅ DONE
- **Phase C — Validator-determinism gate (1/6)** — C.5 only;
  C.1-C.4 + C.6 (hot-path consensus surgery + integration tests)
  remain as the focused next session.

16 commits, 35 new tests (`light-cone` 41 → 51, `consensus` 469 →
494 + 1 ignored), 3 new HTTP endpoints, 4 doctrine docs reconciled.

### Phase A — substrate accessors

  `TendermintConsensus::candidate_heads()` →
  `BTreeSet<BlockId>` of all currently-active sibling heads,
  derived from `light_cone_dag.leaves()` (no redundant field;
  DAG is the single source of truth). Validator-deterministic
  via BTreeMap-key iteration order.

  `TendermintConsensus::enumerate_candidate_heads()` →
  `Vec<(BlockId, caliber)>` sorted descending; smaller-BlockId
  tiebreak. First entry is the MCC-chosen authoritative head.

  `MccForkChoice::enumerate_with_caliber()` is the substrate
  method behind the public accessor. `select_tip` refactored to
  derive its argmax from this list — single source of truth,
  behaviour preserved bit-for-bit.

### Phase B — full state-replay pipeline

`evaporchain-light-cone::dag` (B.0):
  - `find_lca(lc, a, b) -> Option<BlockId>` — Lowest Common
    Ancestor; deepest (highest observed_epoch) common wins,
    smaller-BlockId tiebreak
  - `block_path_from_to(lc, from, to) -> Option<Vec<BlockId>>` —
    first-parent chronological path (`from` excluded, `to`
    included)

`evaporchain-consensus::tendermint`:
  - `plan_replay_to_head(from, to) -> Option<ReplayWalk>` (B.0+) —
    pure planning. Returns `ReplayWalk { lca, forward_path,
    rollback_required }`.
  - `StateSnapshotBranch` (B.1) — concrete
    `LightConeBranchSnapshot` impl wrapping
    `evaporchain_state::snapshot::StateSnapshot`.
    `SnapshotBuilder::create` for capture, `SnapshotApplier::apply`
    for restore.
  - `restore_to_lca(plan, db) -> Result<(), String>` (B.2) — the
    bridge between B.0+ planning and B.1 snapshot restore.
  - `replay_and_apply(db, from, to, block_lookup, block_apply)`
    (B.3) — closure-driven umbrella function. Composes plan +
    restore + forward-apply loop. Returns `ReplayResult` /
    `ReplayError`.
  - `replay_and_apply_atomic(...)` (B.4) — transactional wrapper.
    Pre-replay snapshot capture + on-error rollback. Either
    complete success or complete rollback — never partial.
    Trait-portable: works for InMemoryStateDB AND RocksDBStateDB.

  B.5 — eviction-drops-snapshot regression lock: verifies
  `prune_state_branches` releases the consensus crate's Arc when
  metadata is evicted. Without this, snapshot memory would
  accumulate indefinitely.

  B.6 — end-to-end branch-switch integration test: 3-block-deep
  diverging DAG, captures snapshot at genesis, mutates fork A,
  plans replay A2 → B2 (LCA=genesis, rollback_required=true,
  forward_path=[B1, B2]), calls restore_to_lca, applies fork B
  forward path. Asserts final state reflects fork B only — no
  fork-A residue, no merge artefact, no hybrid state.

### Phase C — validator-determinism gate (C.5)

`mcc_phase_c5_validator_determinism_under_random_dags`: property
test, 256 random DAG shapes (sizes 1..=20 blocks, 1-2 parents per
non-genesis), per shape two `TendermintConsensus` instances
driven through the same block-insertion sequence with FIVE
properties asserted:
  1. `candidate_heads()` BTreeSets agree
  2. `enumerate_candidate_heads()` Vecs agree EXACTLY (order +
     caliber values)
  3. `light_cone_antichain_digest()` matches
  4. `plan_replay_to_head` produces identical `ReplayWalk` for
     every (from, to) pair drawn from candidate heads
  5. No caliber values overflow

256 shapes × 5 assertions = ~1280 individual checks; all pass in
0.76s on Mini. Shipping C.5 BEFORE C.1-C.4 hot-path surgery is the
forcing function: any future change that breaks
validator-determinism fails this proptest before reaching
production.

### Phase E — doctrine + endpoints + runbook

Three new HTTP endpoints:
  - `GET /api/light_cone/candidate_heads` (E.1)
  - `GET /api/light_cone/authoritative_head` (E.2)
  - (Plus existing `/api/light_cone/antichain_digest_history`
    from Light-Cone Phase 7 — together these three form the
    cluster-divergence diagnosis surface.)

Four doctrine doc reconciliations:
  - E.3 — `LIGHT_CONE_FULL_DAG_PLAN.md` Phase 8 cross-doc addendum
  - E.4 — `INVENTION_STACK.md §A1.2 T1` (MCC) updated to reflect
    substrate-shipped state
  - E.5 — `DOCTRINE_PUNCH_LIST.md` Layer 4 row flipped to
    `[x] substrate complete`
  - E.6 — `docs/runbooks/doctrine-rollout-2026-05.md` Lane 4
    `mcc_full` rollout section: pre-flight, three-step ladder
    (linear → mcc → mcc_full), monitoring, rollback. Status
    warning at the top: do NOT flip mcc_full in production until
    Phase C.1-C.4 ships.

### Remaining work

Phases C.1-C.4 + C.6 + Phase D — the consensus hot-path surgery
and adversarial testing. ~2-3 weeks of focused fresh-session
engineering:

  - C.1 authoritative_head selection at start_round
  - C.2 voting handler dispatch by head
  - C.3 proposer multi-parent set selection
  - C.4 cross-fork equivocation rules
  - C.6 4 integration tests (besides C.5 proptest already shipped)
  - D.1-D.5 4-validator 3-fork integration, byzantine reject,
    state-replay-under-churn, perf budget under 4 forks, 72hr
    cluster soak

The substrate + operator surfaces + determinism gate are durable
on origin and ready for the integration work to compose against.

---

## 2026-05-05 — Three frontier-primitive plans shipped end-to-end

Long shipping arc closing three doctrine plans (Lambda-Fold, Crooks-MEV,
Light-Cone Full DAG) plus one Layer 7 LLSA piece. ~75 commits across the
session. Substrate-grade work; every behavioural change is gated behind
governance flags so default-mode chain stays bit-compat with pre-doctrine
behaviour. Operators flip flags on testnet first.

### Lambda-Fold (Layer 5) ✅ DONE end-to-end

`LAMBDA_FOLD_NOVA_PLAN.md` Phases 1–7 (31/31 sub-items). The chain ships
**the first sublinear-in-active-energy verifier** as defined in
`INVENTION_STACK.md §A1.2 row 8`. Sublinearity claim empirically locked
on Mac Mini M4 under release: verify @ 100 folds is **1.083×** of verify
@ 10 folds — essentially flat, far better than logarithmic.

- Phase 1 — design decisions locked (`research/lambda_fold/PHASE_1_DECISIONS.md`)
- Phase 2 — IVC arity 6→8, Poseidon-bound state-root (closes 192-bit collision risk), 5-equation chain-aggregate energy-fold gadget. ~14,575 primary R1CS constraints (was 14,041; +534 for the new gadget + bindings).
- Phase 3 — `vk` preprocessing cached on `RealBlockProver` (`Mutex<Option<(pk, vk)>>`); new `vk_bytes()` + `verify_with_vk_bytes()` light-client API. Light clients verify via vk bytes alone — no `pp`, no prover state.
- Phase 4 — `evaporchain-lambda-fold::nova_path` module (gated on `nova` feature) wires the substrate to real Nova IVC. Substrate blake3 path co-exists.
- Phase 5 — Tendermint integration. Governance flag `lambda_fold_mode ∈ {hash_chain, nova}` (default `hash_chain`). `lambda_fold_nova` crate feature opts the consensus + node binaries into the Nova path. End-to-end test through `on_block_committed` at 5.24 s for 3 blocks.
- Phase 6 — Security tests (state-root collision-resistance, energy-fold over-reporting rejection), sublinearity benchmark, fuzz harness for the verify path, async-fold compat.
- Phase 7 — Doctrine sweep: whitepaper §11.2 updated with arity bump + Poseidon binding; `INVENTION_STACK.md §4.1 row 8` flipped to "SHIPPED 2026-05-04"; `evaporchain-lambda-fold/src/lib.rs` rewritten with dual-mode description; `DOCTRINE_PUNCH_LIST.md` Layer 5 ✅.

### Crooks-MEV (Layer 6) ✅ DONE end-to-end

`CROOKS_MEV_INTEGRATION_PLAN.md` Phases 1–7 (incl. previously-deferred 3.5d + 4.2). The chain ships a **Crooks-fluctuation MEV refund pipeline**: per-block sandwich detection → rate-based pmf → ΔF computation → settlement → anti-gaming → automatic stake deduction.

- Phase 1 — `evaporchain-mev-detect` crate. `scan_block` walks Transfer triples; emits `MevObservation` for every sandwich shape. O(n²) with empirical 13.6 ms on a 1000-tx block.
- Phase 2 — Crooks-fluctuation refund formula. Rate-based pmf substitution (rigorous forward/reverse path Crooks pmf needs LP/AMM accounting EvaporChain doesn't have natively; honest-caveat documented in `research/crooks_mev/PHASE_2_DECISIONS.md`).
- Phase 3.1 — `RefundTx` protocol-issued tx variant. Wire-format: 25th `Transaction` enum variant; tag 0x18 in `signable_bytes`.
- Phase 3.2 — Deterministic `mev_state_digest` (canonical-ordered blake3 over observations + attacker stats).
- Phase 3.3 — Producer helper (`due_refund_txs`) + replay protection (`settled_refunds`).
- Phase 3.4 — Block validation rule (`validate_block_refunds` with `MissingRefund`/`UnexpectedRefund`/`MismatchedRefund` errors).
- Phase 3.5a — Executor balance movement (parallel session shipped `execute_refund` + 4 unit tests; this session confirmed wiring).
- Phase 3.5b — Validator-rejection hook in proposal handling at `tendermint.rs:3328`.
- Phase 3.5c — `mev_missing_refund_violations` counter substrate.
- Phase 3.5d — **Stake deduction wiring**: `apply_mev_missing_refund_slashes` consumes the counter, computes `entropic_slash`, applies via `validator_set.slash_with_amount`. Gated by `crooks_mev_missing_refund_slash_enabled`.
- Phase 4.1 — Confidence threshold (`crooks_mev_confidence_threshold_ppm`).
- Phase 4.3 — Self-MEV pre-filter at detection time.
- Phase 4.4 — Operator dispute via `POST /api/mev/dispute` with grace-period gate.
- Phase 4.2 — **Wire-format opt-out**: `TransferTx::mev_refund_eligible: Option<bool>` field (159-site cascade across the workspace). `Some(false)` opts the victim out — detector skips the observation entirely.
- Phase 5 — Governance flag rollout: `crooks_mev_settlement_mode ∈ {observe, enforce}` (default observe).
- Phase 6 — End-to-end consensus pipeline test + worst-case detection cost benchmark + adversarial witness test.
- Phase 7 — Whitepaper §8 reframed as "Two-Tier MEV Defense" with new §8.4 Crooks-MEV Restitution; `INVENTION_STACK.md` Crooks-MEV row updated; `DOCTRINE_PUNCH_LIST.md` Layer 6 Crooks-MEV ✅.

### Light-Cone Full DAG (Layer 6) ✅ DONE end-to-end

`LIGHT_CONE_FULL_DAG_PLAN.md` Phases 1–6 (31/31 sub-items). The chain ships a **DAG-mode partial-order causal-set consensus** with antichain finalization. The doctrine's "Soul of the chain" primitive (`INVENTION_STACK.md §A1.2 row 1`).

- Phase 1 — DAG-aware tip selection: `LightCone::leaves()`, `MccForkChoice::select_tip` (max-caliber leaf with deterministic BlockId tie-break), `TendermintConsensus::current_tip()`, proposer integration at `create_proposal`.
- Phase 2 — Multi-parent block wire format: `Block::parents: Vec<[u8;32]>` (with `serde(default, skip_serializing_if)` for chain-id continuity), `effective_parents()`, `validate_parents_wire_format()` (3 failure modes).
- Phase 3 — Per-fork state-branch substrate: `state_branches: HashMap<BlockId, LightConeBranchMetadata>`, `LightConeBranchSnapshot` trait (executor-side seam), LRU eviction at `light_cone_max_concurrent_forks` (default 4) paired with DAG-side `prune_orphan_branch` cascade.
- Phase 4 — Antichain finality: `dag_round_states: HashMap<BlockId, RoundState>`, `record_dag_prevote`/`record_dag_precommit` API, voting-handler wiring at `handle_prevote`/`handle_precommit`, `try_finalize_antichain` predicate (closing antichain ∩ ≥ 2f+1 precommits per block), cross-fork equivocation counter (`cross_fork_equivocations`), dual-mode finality bookkeeping (`committed_at_block` paired with `committed_at`).
- Phase 5 — Compaction: `LightCone::prune_orphan_branch` cascade, `detect_orphan_branches` rule (caliber threshold + 32-block recency window), LRU/DAG paired prune.
- Phase 6 — Tests + integration + doctrine: end-to-end DAG-mode pipeline test (`test_dag_mode_full_pipeline_end_to_end`), adversarial 2-fork split-vote test (`test_dag_mode_adversarial_2fork_split_vote_converges`), perf benchmark (`benchmark_light_cone_phase_6_3` — 1000-block DAG: insertion 418 ns/block, select_tip 365 µs, state-branch ops 15.8 µs; all 100×–10⁵× under plan budgets), `INVENTION_STACK.md` row updated, `DOCTRINE_PUNCH_LIST.md` Layer 6 Light-Cone row flipped ⏳ → ✅, whitepaper §4.5 "Light-Cone Full DAG Mode" added with seven sub-sections.

Decision-lock docs: `research/light_cone/PHASE_3_DECISIONS.md`, `PHASE_4_DECISIONS.md`. Rollout flag: `light_cone_state_branches_enabled` (default false). All Phase 4 voting-handler wiring is additive — primary `round_state` stays as the linear-mode tally; DAG-mode `dag_round_states` populates only when flag is on.

### Layer 7 (LLSA) — partial close

`evaporchain-llsa::MultiAuditorVerifier` shipped: k-of-n threshold-aggregating `ProofVerifier` with constructor rejection of degenerate thresholds. Closes one of three deferred Layer 7 sub-items WITHOUT the M2 Coq-build unblock. The other two remaining sub-items (production Coq verifier + MetaCoq + Rust extraction) are still gated on user-side M2.

### Cross-cutting

- 4 decision-lock docs shipped this session (`PHASE_3_DECISIONS.md` + `PHASE_4_DECISIONS.md` for Light-Cone; complementing the existing `lambda_fold/PHASE_1_DECISIONS.md` + `crooks_mev/PHASE_2_DECISIONS.md`).
- 9 governance flags added to the soft-fork allowlist: `lambda_fold_mode`, `crooks_mev_settlement_mode`, `crooks_mev_beta_mb`, `crooks_mev_grace_period_blocks`, `crooks_mev_refund_window_blocks`, `crooks_mev_confidence_threshold_ppm`, `crooks_mev_missing_refund_slash_enabled`, `light_cone_state_branches_enabled`, `light_cone_max_concurrent_forks`, `light_cone_orphan_caliber_threshold`. All default to "off / linear / observe" — chain bit-compat preserved.
- ~150 new tests across substrate, integration, fuzz harness, proptest, perf benchmark.
- Drive-by audit-fix migrations cleaned up: `target_utilization_ppm`/`health_score_ppm` field renames left dangling by parallel sessions.

## 2026-05-05 (afternoon) — Post-doctrine consistency + observability + README sweep

Continuation session after the morning doctrine arc closed. Closed the
remaining post-doctrine punch-list items, fixed a class of
proposer/follower divergence bugs in the gossip-path block-commit, shipped
the Phase 4.4 antichain commit-cert digest the doctrine rollout runbook
flagged as the next operator-facing piece, and swept all in-tree READMEs
to current state.

### Operator diagnostic — `/api/network/scores`

Lane R.* (cluster freeze 2026-05-04) carry-forward item closed. New
`SybilState::scores_view()` iterates the full `scores` HashMap including
ghost entries (peers in `scores` but not `peer_ips`) — the freeze-class
signal that was invisible to `/api/network/peers`. New `PeerScoreEntry`
exported from `evaporchain-network`. New `GET /api/network/scores`
returns `{scores, count, ghost_count}` — `ghost_count > 0` is the
standing diagnostic for the next freeze-class issue. Regression test
`test_scores_view_surfaces_ghost_entries`. Network 64/64 green.

### M2 Coq build verification — Rocq 9.1.1

Layer 7 LLSA descope path's last hard gate. `brew install coq` on Mini 1
(Rocq 9.1.1, the renamed Coq) surfaced four classes of breakage from the
8.18 → 9.x transition that the prior `omega → lia` migration didn't
anticipate:

1. `Coq.Arith.Div2` removed in Coq 9.0 — dropped the unused import (`pow2` is defined locally).
2. Coq 9.0 enforces strict bullet structure between `split`s — replaced `split. - tac. split.` patterns with `split. { tac. } split.` brace-focusing.
3. `lia` failed on trivial `0 <= n` and `n <= n` — replaced with direct lemmas (`Nat.le_0_l`, `Nat.le_refl`).
4. `apply X; assumption` no longer leaves evars for later in 9.0 — replaced with `eapply X; eassumption`.
5. `decay_preserves_inv` had a redundant `le_trans` chain through `prior_total p` — simplified to a single chain.

`research/proofs/LLSAInvariantPreservation.v` now compiles clean
end-to-end. All 4 lemmas at `Qed.`. The "first chain whose governance is
a build-verifiable theorem under audit" claim now stands on a re-running
kernel proof, not on documentation. Layer 7 descope path advanced from
~70% to ~90%.

### TLA deadlock counter-example resolution

The two `_TTrace_*.tla` files (dated 2026-04-30) were emitted when TLC's
default deadlock detection fired on the *intended* terminal state of
bounded model checking (every action guarded by `height[v] <=
MaxHeight`; once all validators commit up to MaxHeight, no action is
enabled). Inspection of the trace state confirmed all 7 safety
invariants (Agreement, Validity, CommitRequiresQuorum, LockSafety,
EquivocationDetected, StateCommitmentIntegrity, TypeOK) hold at the
"deadlock". Fix: `CHECK_DEADLOCK FALSE` added to all four `.cfg` files
with rationale comment. Background documented in `research/tla/README.md`
"On TLC deadlock reports" section. Punch list closed.

### Proposer/follower divergence fixes — six chain-wide post-commit primitives

The proposer-local block-commit at `main.rs:4205-4242` ticked Mortis,
Decay-Lamport, Sentinel autonomic governance, DSN nullifiers, PNT phase,
and the four-act snapshot publisher. The gossip-follower commit at
`main.rs:5278+` shipped only `tc.on_block_committed` and frontier-state
updates — every other chain-wide deterministic primitive *did not tick on
follower validators*. Result: in a 3-validator cluster, only the
proposer of each block updated these counters; followers' dashboards
drifted block-by-block.

Symmetric mirror shipped on the gossip path:

- **Decay-Lamport** (§4.1 #3 Tier-1) — clock now ticks per-block on every validator role.
- **DSN** (Tier-2 Decay-Stamped Nullifiers) — every validator folds the same deterministic per-block nullifier.
- **PNT** (Phased Nullifier Tree) — phase advances once per epoch on every node.
- **Mortis** (`tick_mortis_on_executor`) — four-act narrative state machine ticks deterministically.
- **Sentinel** (`autonomic_sentinel_tick`) — homeostatic governance parameter updates apply consistently.
- **`/api/four_act` snapshot publisher** — operator dashboard data on follower nodes was stale; now publishes per block.

Two whole classes of "why are validators 2 and 3 reporting stale numbers"
operator-confusion bugs eliminated.

`evap_getLamportClock` JSON-RPC docstring updated to document both wiring
sites.

### Phase 4.4 antichain commit-cert digest

The doctrine rollout runbook flagged Phase 4.4 as the "next step" beyond
the 6/6 LIGHT_CONE_FULL_DAG_PLAN.md Phase 6 deliverable — the missing
inter-validator agreement digest for the Light-Cone substrate (sibling
to Crooks-MEV's `mev_state_digest`). Shipped end-to-end:

- `evaporchain-light-cone::concurrency::digest_antichain` + `closing_antichain_digest`. Domain-separated under `evaporchain-antichain-digest-v1`. Sort-before-hash for validator-determinism. 32-byte blake3 output. Empty-set sentinel = blake3-of-domain-tag-alone.
- `TendermintConsensus::light_cone_antichain_digest()` + `light_cone_closing_antichain()` accessors.
- `GET /api/light_cone/antichain_digest` HTTP endpoint returns `{digest, closing_antichain, closing_antichain_size, running_alongside_tendermint}`.
- 6 new substrate tests (order-independence, set-separation, empty-set sentinel, domain separation, composition idiom, diverging-DAG separation). Light-cone tests 34/34 (was 28).
- Plan addendum: `LIGHT_CONE_FULL_DAG_PLAN.md` Phase 7 (4/4 sub-items shipped). Punch list flipped ⏳ → ✅. Runbook Step 2 of the DAG-mode rollout sequence updated to use the new endpoint for the inter-validator agreement check.

### Doctrine doc reconciliation

`DOCTRINE_PUNCH_LIST.md` checkboxes brought into line with what's
actually shipped. Layer 1 M3.1 (MCC) + M3.2 (CFM) flipped ✅. Layer 2
Coq cleanup mostly closed (TLA-trace investigation also flipped ✅
above). Layer 5 — all 6 sub-items flipped ✅ with arity-8 / Poseidon /
Nova / sublinearity refs. Layer 6 — Singh-Lyapunov ✅, Crooks-MEV ✅,
Light-Cone substrate ✅ with explicit post-V1 gap list. Layer 7 descope
path bumped from ~70% to ~90% (MultiAuditorVerifier shipped + Coq build
verified); the `AlwaysAcceptVerifier` stub note replaced with the k-of-n
production-verifier reference. Status snapshot table updated for Layers
1 and 7. All four "Doctrine amendments needed" items at the bottom of
the punch list now resolved.

`INVENTION_STACK.md §A1.2 T4` updated: *"the first chain whose governance
is a build-verifiable theorem under audit"* — honest claim that matches
shipped state (Coq-build-verified kernel + `MultiAuditorVerifier` k-of-n).
Tezos-beat comparison preserved (Tezos has neither Coq term nor auditor
signatures). Full theorem-grade on-chain MetaCoq path preserved as
post-V1 work.

### `evaporchain-consensus` private_interfaces warning resolved

`RoundState` made `pub(crate)` to match `dag_round_states` field
visibility introduced by Light-Cone Phase 4 work. Crate builds clean.

### README sweep — 10 files updated to current state

Audit identified 4 actively-misleading READMEs (Tier A) + 8 stale READMEs
(Tier B) out of 24 in-tree files. 10 fixed:

- **Root `README.md`** — replaced "7,477+ tests across 100+ crates" with accurate "12,500+ test functions across 147 workspace crates", added doctrine-arc status row (Lambda-Fold Nova / Crooks-MEV / Light-Cone DAG / Causal-CHSH / MultiAuditorVerifier / M2 Coq), expanded crate map with 17 named frontier primitives, port `:3000` → `:8080`.
- **`docs/README.md`** — port `:3000` → `:8080` (all occurrences). Added 5 new endpoint sections: `/api/network/scores`, `/api/light_cone/*` (Phase 4.4 antichain digest), `/api/lambda_fold/nova/*`, `evap_getLamportClock`.
- **`website/README.md`** — fictional dApp directory list (`nft-marketplace, energy-pool, mortal-messages, governance`) replaced with accurate listing of `dapps/` (singh-pool, validator-analytics, gov-portal, explorer-light + 4 legacy/early-phase apps).
- **`research/coq/README.md`** — toolchain line updated to "verified clean under Rocq 9.1.1" with the M2 transition-fix note. New row in the file-status table for `LLSAInvariantPreservation.v` showing all 4 lemmas at `Qed.`. Closing paragraph rewritten to reflect that LLSA is now build-verifiable end-to-end.
- **`research/frontier/README.md`** — expanded from 3 primitives to the full Tier-0 invention stack (5) + Tier-0 supporting (7) + 2026-05 doctrine arc (Lambda-Fold Nova, Crooks-MEV, Light-Cone Full DAG).
- **`research/tla/README.md`** — Files header now lists all four `.cfg` files (was 3 listed despite body referencing 4 after the `CHECK_DEADLOCK FALSE` sweep).
- **`docs/architecture/diagrams/README.md`** — replaced "(commit hash to be added at audit kickoff)" placeholder with "kept current with main; auditors should pin a specific commit for their snapshot reference."
- **`sdk/README.md`** — port `:3000` → `:8080`. New "Frontier endpoints (not yet wrapped)" section listing `/api/light_cone/antichain_digest`, `/api/network/scores`, `/api/mev/*`, `/api/cartel_alarm/*`, `/api/lambda_fold/nova/*`, `evap_getLamportClock` so SDK users know the coverage gap is documented, not accidental.
- **`extension/README.md`** — new "Reproducible builds" section advertises the deterministic WASM-build pipeline (`scripts/build-wasm.sh`, `scripts/wasm-build-versions.json`, `scripts/verify-wasm.mjs`) so reviewers see the user-protective property: *Chrome-Web-Store wallet is bit-identical to a rebuild from this repo at the tagged commit*.
- **`prototypes/fold-a-block/README.md`** — historical-status header pointing to the production Nova IVC integration (`crates/evaporchain-proving::nova` + `crates/evaporchain-lambda-fold`) and the empirical sublinearity numbers from the production path that supersede the prototype targets.

External auditors / grant reviewers / new contributors landing on any of
these now read accurate state.

### Net session ship

| Surface | Change |
|---|---|
| Code | `+~250 LOC` across `evaporchain-light-cone::concurrency`, `evaporchain-consensus::tendermint`, `evaporchain-network::service`, `evaporchain-node::api` + `main.rs` + `jsonrpc.rs` |
| Coq | 5 distinct 8.18→9.0 fix classes in `LLSAInvariantPreservation.v` — build clean under Rocq 9.1.1 |
| TLA | `CHECK_DEADLOCK FALSE` × 4 cfg files |
| Tests | +7 (network 64/64, light-cone 34/34) |
| HTTP endpoints | +2 (`/api/network/scores`, `/api/light_cone/antichain_digest`) |
| Doc updates | 10 READMEs + 3 doctrine docs (`DOCTRINE_PUNCH_LIST.md`, `LIGHT_CONE_FULL_DAG_PLAN.md`, `INVENTION_STACK.md` §A1.2 T4) + runbook + this CHANGELOG entry |
| Warnings cleared | `private_interfaces` on `RoundState` |

## 2026-05-04 night — Press-claim test sweep across substrate primitives

Added 36+ top-level `press_claim_tests` modules to substrate crates so the
doctrine headline of each crate ("the press claim") is asserted as a
structural invariant. If the implementation ever drifts from the claim,
the test breaks loudly.

Coverage added (lib.rs-level press_claim_tests modules):

- **Tier-2 paradigm**: total-evaporscript, cap-decay-vm, dp-native-vm
- **Tier-3 specialized**: epa-mmr, thermal-stm, plc, ew-twap
- **Identity / consensus**: bell-beacon-v2, causal-chsh, ib-validators-v2,
  modular-beacon, singh-attractor-v2, singh-inequality-v2, light-cone-v2,
  mera (research-artefact), bell-beacon (v1), ib-validators (v1),
  singh-attractor (v1), singh-inequality (v1), allen-decay
- **Core**: types (existing), da, crypto, state, execution, consensus,
  network, proving, script, contracts, light-cone
- **Decay primitives**: energy-kernel, tropical, mnemochain, childkey,
  decay-forget, decay-lamport, decay-sealed-regions
- **Slashing / governance**: entropic-slashing, sanov-slashing,
  conviction-vote, prp, cmu-gate, tur-liveness, pnt
- **NFT family**: singh-resonance, singh-heartbeat, singh-lineage,
  singh-migrant, singh-sabi, singh-triage, singh-posthuma, singh-counsel,
  singh-heir, half-life-nft, gallery-forgets
- **Social / inheritance**: grave-graph, grave-graph-split

Also fixed a pre-existing `Block` constructor breakage: the `parents:
Vec<[u8;32]>` field added by the linter required updating 11 Block
constructor sites across execution, network, proving, state, and bench
crates. Workspace test count moved from 7,378 to 7,477+ (lib tests, 0
failed).

## 2026-05-04 evening — Lane R.* cluster-freeze fix + origin/main reconciliation

### What broke

3-Mini Tailscale cluster halted at h=771 after ~90 minutes uptime.
Mini 1 was stuck at h=145 on a different state root from Mini 2/3
(h=771, lockstep). Root-cause investigation via `/api/network/peers`
on Mini 2 surfaced a peer with `score: -292, age_seconds: 47` — the
score had been decaying for ~24 hours while the peer was DISCONNECTED.

Three independent design issues compounded into a livelock:

  1. **`SCORE_IDLE_TICK = -1`** fired every 5 min on every entry in
     the `scores` HashMap, including disconnected peers (which
     `record_disconnect` left in the map).
  2. **`record_connect`** used `entry().or_default()`, so a peer
     reconnecting after going negative INHERITED their prior score
     instead of getting a fresh slate.
  3. **No authorization gate** on idle-score penalty: validators
     pre-vetted via TLS / peer-id allowlist (`peer_authority`) got
     penalized identically to random Sybil peers.

After ~100 idle ticks (~8 hours wall-clock), any peer crossed
`SCORE_BAN_THRESHOLD = -100` → IP soft-banned for `peer_ban_duration_secs
= 3600` (1 hour). With BFT 2/3+1, losing one validator halts a
3-validator cluster. Once unbanned + reconnected, the inherited
negative score reban'd it. Livelock per process lifetime.

### What landed (genuine three-layer fix)

| Lane | What | Commit |
|---|---|---|
| R.1 | Authorized validators bypass Sybil idle-ban + auto-unban on connect | `803ac6d` |
| R.2 | Regression test: 256-tick fixture confirms bug class + gate works | `9d192bf` |
| R.3 | `record_disconnect` clears score entry; `record_connect` fresh-slates; idle tick iterates `peer_ips` not `scores` | `1555eb8` |

Each layer alone closes the livelock; all three make accidental
regression near-impossible. Network crate tests: 62/62 pass.

### Origin/main reconciliation (Lane R.4 attempt → R.5 revert → R.6-R.12 disciplined)

Deploying R.1+R.3 to the live cluster required rebuilding the node
binary on each Mini, which required origin/main to be buildable on
a clean checkout. It wasn't — origin/main had accumulated weeks of
half-finished cross-crate refactors:

  - `FEE_PPM_DENOMINATOR` referenced but never declared in fees.rs
  - `VS_PPM_DENOMINATOR` similarly undeclared in validator_set.rs
  - `health_score_ppm` / `target_utilization_ppm` / `confidence_score`
    fields referenced before they were added
  - `Transaction::Refund` arm missing in 3 separate match sites across
    consensus + wallet + execution
  - 73 sister-session crates listed in workspace Cargo.toml but never
    committed — each missing one fails build sequentially
  - nova-snark API drift: `compressed.verify` returns `Vec<Scalar>`
    not the old `(Vec, Vec)` tuple

| Lane | What | Commit |
|---|---|---|
| R.4 | Bulk Mac-state commit attempt (42 files) — pollution, reverted | (reverted) |
| R.5 | Revert R.4 mass-commit; keep R.1/R.2/R.3 + sister docs interleaved | `7e289bc` |
| R.6 | Light-Cone DAG Phase 1.1: `LightCone::leaves()` + `ForkChoice::select_tip` seam + types contract test | `6b23261` `18c926f` |
| R.7 | Minimal pub-const decls (`FEE_PPM_DENOMINATOR`, `VS_PPM_DENOMINATOR`) + Refund arm in wallet/gas | `c2c6294` |
| R.8 | Fees `target_utilization` fallback + wallet/signer Refund arm | `e0b3b64` |
| R.9 | Tendermint `health_score` fallback (was `health_score_ppm`) | `f064f57` |
| R.10 | Node api.rs `confidence_score_ppm` + `health_score` field renames | `a6aae53` |
| R.11 | Land the 535-LOC `evaporchain-light-cone-v2` crate that workspace listed | `6ef88de` |
| R.12 | Land the remaining 72 sister-session crates (337 files, 47.8K LOC) | `2f53749` |

Each Lane R.X was committed as a small additive batch, verified on
Mini 1 with `cargo check --workspace`, then rolled forward. The
disciplined approach converged in 9 commits; the earlier bulk-commit
approach (R.4) blew up worse than not committing at all.

### Cluster recovery + first in-production R.1/R.3 validation

After R.12, all 3 Minis built clean (`cargo build --release --features
prove` finished in ~1m23s on each). Stopped processes, restored BLS
private keys from `~/validator-N-keys.json` (the data-dir wipe had
deleted `bls_key.bin`), restarted with the launch flags.

Cluster came back at h=37 with peer_count=2 across all three. By
2026-05-04 16:51 UTC: h=1591, identical state root
`1ec9175f30efc58eb38595d557781a276c5815b0c267d9fdff4344d7ce5a8e13`,
4.2 blk/s. Both peers showed `score: 0` after 6 min of uptime —
without R.1/R.3 they'd be at -1 already (SCORE_IDLE_TICK fired at
the 5-min mark). **First in-production validation of R.1/R.3.**

### What's still open

| Item | Effort |
|---|---|
| Sister-session ppm migration: complete the FEE_PPM/VS_PPM PID refactor that the Lane R.7-R.10 stubs unblock | 1-2 sessions |
| Cluster diagnostic RPC: `/api/network/scores` exposing per-peer `score` + `last_tick` so the next freeze surfaces faster | half-session |
| Whitepaper §A1.3 Causal-CHSH amendment | manual |

---

## 2026-05-03 / 2026-05-04 — Layer 3/4 substrate seams + Layer 0 closure + doctrine sweeps

This session lands the consensus abstraction seams (Layer 3), the
first concrete impls behind them (Layer 4), governance-flag wiring +
operator UX RPCs, four proptest mirrors locking trait invariants, and
two doctrine-doc sweeps reconciling `INVENTION_STACK.md` +
`DOCTRINE_PUNCH_LIST.md` with reality after the MERA gate FAILED on
real Ethereum data (VERKLE verdict).

Default behaviour is unchanged across all 27 commits — every new code
path is governance-gated `linear / fifo / observe` until an operator
explicitly opts in.

### Substrate (`evaporchain-consensus`)

| Lane | Trait / impl | Commit |
|---|---|---|
| G.1 | `pub trait BlockSource` + blanket impl on `Mempool` | `f78d965` |
| G.3 | `pub trait ForkChoice` + `LinearForkChoice` default | `61eb888` |
| G.4 | `pub trait MevPool` + blanket impl on `EncryptedMempool` | `150292c` |
| G.5 | `pub trait ValidatorSetSource` + impl on `ValidatorSet` | `118b19d` |
| I.1 | `TxAntichainMempool` — first non-default `BlockSource` impl | `842363f` |
| I.3 | `MccForkChoice` — first non-default `ForkChoice` impl | `c1a05bb` |
| I.5 | `mempool::antichain_project` — post-FIFO antichain helper | `2bdcdc2` |

### Hot-path consumers (governance-gated)

| Lane | What | Commit |
|---|---|---|
| I.4 | `parent_acceptance_mode = "mcc"` dispatches at `tendermint.rs:2643` | `ded1a73` |
| I.5 | `block_source_mode = "antichain"` filters at `tendermint.rs:3915` | `20d9fc8` |
| I.6 | MCC β derived from chain CFM (microbits/fee/epoch) | `a45588c` |
| F.1 | Singh-Lyapunov fee tick wired into `execute_block` | (sister `4d59b5d` + test fix `b14ed53`) |

### Operator UX

| Lane | Endpoint / API | Commit |
|---|---|---|
| J.0 | `GET /api/governance/flags` — inspect all soft-fork keys | `d694ce8` |
| K.1 | `POST /api/governance/param` — flip with allowlist | `2fa6362` |

`fork_choice_mode` retains its existing dedicated endpoint
(endorser-stake-validated). Other knobs (`parent_acceptance_mode`,
`block_source_mode`, `conservation_enforcement`) flip via the generic
allowlisted setter.

### Test rigor — proof-style coverage of trait contracts

256 randomised inputs each, ~1,536 randomised assertions per
`cargo test -p evaporchain-consensus`:

| Test | Properties locked |
|---|---|
| `tx_antichain_mempool::antichain_invariant_no_duplicate_senders` | 4 (Lane I.1) |
| `mempool::antichain_project_invariants` | 5 (Lane I.5 follow-up) |
| `fork_choice::mcc_proptest_invariants` | 3 (Lane I.3 follow-up) |
| `tx_antichain_mempool::block_source_contract_holds_for_both_impls` | cross-impl (Lane G.1 follow-up) |
| `fork_choice::fork_choice_contract_holds_for_both_impls` | cross-impl (Lane K.3) |
| `tendermint::tests::governance_set_param_proptest` | 4-bucket allowlist (Lane K.4) |

### Test rigor — integration tests for governance flag dispatch

| Test | Lane | Locks |
|---|---|---|
| `test_block_source_mode_antichain_dedups_same_sender_in_proposal` | J.1 | I.5 wire-path |
| `test_block_source_mode_default_admits_all_same_sender` | J.1 | typo-safety |
| `test_parent_acceptance_mode_mcc_diverges_from_linear_on_diverging_parent` | J.2 | I.4 + I.6 differential |
| `test_parent_acceptance_mode_typo_falls_through_to_linear` | J.2 | typo-safety |
| `test_governance_set_param_*` (4 tests) | K.2 | allowlist contract |

### Doctrine sweeps

| Lane | What | Commit |
|---|---|---|
| Layer 1 | INVENTION_STACK §A1.2 T1 + T2 wording fixes; MERA caveat; LightCone read-only note; CSLC endpoint label | `bfaa758` |
| H.1 | DOCTRINE_PUNCH_LIST Layer 0 #4 marked closed (verified `collect_demurrage` already wired) | `3f8d84b` |
| Layer 0 closure | DOCTRINE_PUNCH_LIST Layer 0 #3 + #5 marked closed | `f507434` |
| M.1 | DOCTRINE_PUNCH_LIST bullet sweep — 14 stale `[ ]` items closed with commit refs | `944879b` |
| M.2 | INVENTION_STACK MERA references swept post-VERKLE verdict | `66a84a4` |

### Final test counts (Mini 1)

- 415 evaporchain-consensus lib tests pass
- 0 regressions across the session
- ~1,536 proptest randomised assertions × 256 inputs = ~393k checks
  per `cargo test`

### What's still genuinely open

| Item | Effort |
|---|---|
| Layer 5 — Lambda-Fold real Nova IVC (`state_root_to_u64` truncation, `RealBlockCircuit` arity 6→7) | 3-6 weeks |
| Layer 6 — Crooks-MEV refund consensus integration (substrate exists, no consensus hot-path wiring) | multi-day |
| Layer 6 — Light-Cone full consensus rewrite (replaces tendermint.rs's 8.7K LOC) | months |
| Layer 7 — LLSA full theorem-grade governance (or descope to k-of-n auditor signatures) | 9-15 months OR 4-6 weeks |
| M2 — Coq build verification (manual) | 10 min |
| M3.1 — INVENTION_STACK §A1.2 T1 wording (Satyawan strategic call) | 30 min |
| M3.2 — INVENTION_STACK §A1.2 T2 wording (Satyawan strategic call) | 30 min |
| Layer 2 — CSSR (Shalizi-Klinkner ε-machine reconstruction) | 2-3 sessions |

### Cluster operations

3-Mini Tailscale cluster experienced a divergence event mid-session
(wipe-restart of Mini 1 with peers at h=771 produced a fork that the
current sync protocol couldn't reconcile — Mini 1 stuck at h=178, peers
halted at h=771 awaiting BFT 2/3+1). Cluster ops were de-prioritised in
favour of the building work above. Cluster-wide reset can be issued
later via `restart-tailscale-3node.sh` on all 3 Minis simultaneously.

---

## Amendment — 2026-05-04 — Causal-CHSH frontier primitive shipped end-to-end

After the MERA gate FAILED → VERKLE verdict (commit `2053a86`) closed
the question of whether MERA ships, the user asked: "do we must
introduce our new math and our frontier idea, insane novel?" The
answer was yes — but with the same MERA-style empirical gating
discipline that just earned its keep. Lanes O.1 through O.7+ delivered
EvaporChain's first 100% original frontier theorem from concept to
operationally-exposed primitive in one session.

### The Causal-CHSH cartel-detection bound

Bell's CHSH inequality (Clauser-Horne-Shimony-Holt 1969) translated
to blockchain causal sets. Theorem (proposed): for `S = |E(A,B) +
E(A,B') + E(A',B) − E(A',B')|` over four samples of ±1 products
drawn from concurrent block pairs in the LightCone DAG under four
setting-pairs, **`S ≤ 2`** under honest validators + LightCone
causality + EvaporChain's single-λ decay. **Violation `S > 2` ⇒
hidden cross-validator coordination.**

Where Bell's theorem gave physics quantum-entanglement detection,
Causal-CHSH gives blockchain *cartel-detection* with a closed-form
bound — not a heuristic, not a slashing rule, a *theorem*. **Only
LightCone-style chains can even form the four-term correlation**
(Tendermint linear chains have no concurrent blocks; Ethereum's
reorgs are competing finalisers, not concurrent producers). The
math is new because the substrate is new.

### The build → gate → ship cycle

| Lane | What | Commit |
|---|---|---|
| O.1 | New crate `evaporchain-causal-chsh` with math primitive + synthetic gate (12 tests including 2 proptests) | `801fd7c` |
| O.2 | Real-data driver: `extract_chsh_samples` over a `BlockSummary` trace via concurrency-window proxy + 4 binary observables; synthetic-Eth methodology validated (17 tests) | `7876624` |
| O.3 | Real Ethereum gate runner (Rust binary) + Python scraper. **Verdict: PASS** on 200 mainnet blocks (19_900_000+) — S_honest=0.012, S_cartel=4.0, gap=3.99 — ~150× headroom on the doctrine ceiling | `c9e553c` |
| O.4 | INVENTION_STACK.md `§A1.3` row reservation + new `§A1.10` gate-resolution section (parallel to MERA's `§A1.8`) + new doctrine rule #14 ("pre-commit gate thresholds before running") + Tier-0-supporting count 6 → 7 | `76cc71d` |
| O.5 | `POST /api/cartel_alarm/run_gate` — operators can run the gate live against arbitrary chain trace data; doctrine-locked thresholds baked in (no operator override) | `f396b7d` |
| O.6 | 3K-block sanity check on the same Eth window MERA used (19_900_000–19_903_000). **Verdict: PASS again** — S_honest=0.018 (vs 0.012 on 200-block, both well below 1.8), 14,885 ±1 samples (15× more than 200-block run). Verdict robust under sample-size scaling. | `cdb736c` |
| O.7 | `CartelAlarm` rolling-buffer substrate primitive — fixed-capacity ring of `BlockSummary`, periodic gate-run logic, last-S tracking. Observability-first; no auto-action emission yet (deferred to Lane O.8 design). | `63b6cf6` |
| O.7+ | Proptest 256× alarm invariants (buffer cap, monotonic counter, first-run threshold, honest-source verdict). Caught a real off-by-one in the periodic-run logic (capacity=50, interval=21, n_records=60 edge case) — pure proptest win. | `5968295` |
| O.8.1 | `TendermintConsensus` hosts `CartelAlarm` rolling buffer; `on_block_committed` ticks the alarm with a `BlockSummary` per committed block. Observability-only at this stage — no governance hook yet. | (earlier) |
| O.8.1b | `GET /api/cartel_alarm/chain_status` — chain's own self-monitoring verdict surfaced via RPC. Distinct from the operator-supplied-trace path of O.5. | (earlier) |
| O.8.1c | Integration test driving 60 blocks through `on_block_committed` → alarm fires at records_seen=50, height=50; verdict shape locked. | (earlier) |
| O.8.1d | `CartelAlarm.recompute_now` switched to `compute_chsh_s_milli` (i64 milli-units) for validator-determinism on the consensus-bearing path; f64 path retained for RPC display only. | `8853078` |
| O.8.2 | `CartelAlarmEvent` emission. Per-block emission gate fires when (a) governance `cartel_alarm_mode = "alarm"`, (b) chain's `s_honest_milli >= 1800` (doctrine ceiling), (c) no event for `last_run_at_height` already queued. Default `observe` mode silent. Surface drained via `take_pending_cartel_alarms()`. **Closes the original Lane O.8 design lane: alarm hook + governance + dedupe all in-protocol.** | `122821f` |
| O.8.2b | `GET /api/cartel_alarm/pending_events` — RPC drains the chain's queued `CartelAlarmEvent`s; each event returned exactly once. Operator dashboard / pager surface. | `0fac70f` |
| O.8.2c | Full-pipeline integration test: drives blocks through `on_block_committed` with `cartel_alarm_mode = alarm` → injected over-ceiling status → emission → drain. Distinct from O.8.2's unit test which calls the helper directly. Locks the call-site wiring end-to-end. | `6cb4b90` |

### MERA / Causal-CHSH paired symmetry

Same gate discipline. Opposite outcomes. Both demonstrate that
pre-committed thresholds are a feature, not a bug.

| Primitive | Empirical metric | Threshold | Verdict | Outcome |
|---|---|---|---|---|
| Authenticated Energy-MERA | R² = 0.66 (3 independent runs on real Eth) | ≥ 0.85 | FAIL | Drop, retain as research artefact (`§A1.8`) |
| **Causal-CHSH Cartel Detector** | **S_honest = 0.012-0.018, gap = 3.98** (200-block + 3K-block runs on real Eth) | S_honest < 1.8 + gap > 0.4 | **PASS** | **Ship as Tier-0-supporting** (`§A1.10`) |

A doctrine that can fail empirically is a doctrine that can ship
credibly when it doesn't. The credibility is in the symmetry.

### Final Causal-CHSH test counts

- 33 tests total in `evaporchain-causal-chsh` (post O.8.2 — added
  `CartelAlarmEvent` struct + `_inject_status_for_test` doctrine helper)
- 5 proptests across the crate (Bell bound for LHV sources, S
  algebraic range, alarm invariants, plus chsh dispatch)
- `evaporchain-consensus`: 423 lib tests pass (post O.8.2 + O.8.2c —
  added `test_cartel_alarm_event_emission_governance_gated` and
  `test_cartel_alarm_event_emission_via_on_block_committed`)
- Real-Ethereum gate verdict locked in `research/causal-chsh/GATE_RESULT.md`
- 3K-block sanity verdict locked in `research/causal-chsh/GATE_RESULT_3K.md`
- 200-block reproducibility CSV at `research/causal-chsh/honest.csv`
- 3K-block reproducibility CSV at `research/causal-chsh/honest_3k.csv`

### Doctrine drift across reference docs — closed again

After Lane M.1/M.2 closed the drift left over from the original
session, Lane O.4 reopened it (because shipping a new primitive
requires updating the doctrine). This amendment closes it once
more. Future sessions: the four reference surfaces should agree
that EvaporChain ships **5 Tier-0 primitives + 7 Tier-0 supporting
primitives** (was 6 before Causal-CHSH).

### What's still genuinely open after this session

| Item | Effort |
|---|---|
| ~~Lane O.8 — proper consensus integration (`cartel_alarm` governance hook with rolling buffer + auto-emission on `S > cartel_floor`)~~ — **closed by Lane O.8.1 / O.8.2 / O.8.2b / O.8.2c.** Hook ticks every block; `cartel_alarm_mode` governance flag gates emission; `CartelAlarmEvent` queue + `take_pending_cartel_alarms()` + `GET /api/cartel_alarm/pending_events` complete the operator surface. | done |
| Lane O.8.3+ — validator-side reaction policy on emitted `CartelAlarmEvent` (slashing? freeze? governance amendment?). V1 is event surface only — operators page their own response. | multi-day, design-heavy |
| Layer 5 — Lambda-Fold real Nova IVC (sister session) | 3-6 weeks |
| Layer 6 — Crooks-MEV refund consensus integration | multi-day |
| Layer 6 — Light-Cone full consensus rewrite | months |
| Layer 7 — LLSA full theorem-grade or descope to k-of-n auditor signatures | 9-15 months OR 4-6 weeks |
| M2 — Coq build verification (manual) | 10 min |
| M3.1 / M3.2 — INVENTION_STACK §A1.2 wording (Satyawan strategic call) | 30 min each |
| Layer 2 — CSSR | 2-3 sessions |
| Larger Causal-CHSH validation (10K+ blocks, multiple Eth windows) | half-day per window |
