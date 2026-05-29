# Leaderless block production — V1.5 design draft

**Doctrine punch list:** Layer 6 ⏳ "Block-production protocol that emits parent sets without a leader (post-V1)"

**Status:** design draft — **components pre-staged 2026-05-29** (all default-off behind the `doctrine_v1_5` feature; see §7). V1 still ships with leader-rotation Tendermint; the hot-path voting integration (§7 Phase 4b) is deliberately deferred to Q4 2026.

**Target:** V1.5 protocol upgrade behind `--cfg doctrine_v1_5`.  Must coexist with V1 hot path so the chain can run either consensus mode at flag time.

---

## 1.  Problem with leader-rotation

Tendermint-style consensus picks a single proposer per `(height, round)` via a deterministic schedule (today: round-robin weighted by stake).  Three failure modes:

1. **Censorship under MEV pressure.** A proposer can selectively drop transactions they don't like; the next round's proposer can do the same.  Censoring f+1 of 2f+1 validators is sufficient to suppress a tx indefinitely.
2. **Liveness-on-leader-failure.**  When the proposer is offline or slow, the entire round burns timeouts before rotation.
3. **Doctrine impedance mismatch.**  EvaporChain's Light-Cone DAG already supports multi-parent blocks; the leader-bottleneck in proposal-time is artificial.

V1 lives with these because the chain is BFT-correct and the audit gates close the worst MEV/censorship vectors at the validation layer (Crooks-MEV refund, encrypted mempool).  But the doctrine ambition is leaderless block emission, where any validator can propose a block at any time and the network's antichain rule + MCC fork-choice converges on the canonical tip.

## 2.  Three sub-problems

### 2.1  Leaderless block emission

**Goal.**  Any validator may produce a block at any height whose `parents` field references a valid antichain of recent heads.

**Mechanism (sketch).**
- Each validator maintains a per-tip view of pending txs (already in place via `dag_round_states`).
- A validator becomes eligible to propose when:
  - Their VRF output for `(height, chain_id, validator_id)` falls below an adaptive threshold tied to their stake weight and the network's recent block rate.
  - Their proposed `parents` set is a valid antichain in their local Light-Cone DAG.
- Proposed blocks gossip with the existing BlockProposal channel.  Recipients that see multiple competing proposals at the same height feed all of them to MCC fork-choice; the chain converges on the highest-caliber proposal.

**Validator-determinism gate.**  Every validator must arrive at the same `parents` antichain set when they're the proposer for any tip.  Today's `enumerate_candidate_heads()` already provides this; the leaderless emission just lifts the "single proposer" constraint.

### 2.2  Sorkin BD-action / interval-cardinality invariant

The Sorkin BD-action  S = (#elements) − (#causal-relations / 2)  is the doctrine-purity invariant for a Light-Cone DAG.  Today it's measured post-hoc; V1.5 should enforce it at INSERT time:

- A block `b` may only be inserted into the local DAG if its `parents` antichain doesn't violate the BD-action gradient at the proposed insertion point.
- The current `LightCone::insert` enforces `MAX_PARENTS_PER_BLOCK = 16` (SUB-N6) and `is_antichain` (SUB-N7); add a BD-action threshold check on top.
- The threshold is a chain parameter: stricter values prefer linear-chain doctrine; looser values let the DAG grow wider.

### 2.3  Network-level causal delivery

Today's gossipsub doesn't guarantee that block `b` is delivered AFTER its `parents` arrive.  Honest validators handle this with a "future block buffer" that holds out-of-order blocks until parents arrive.  V1.5 should formalize this:

- Gossipsub message envelope carries the block's `parents` set in the metadata.
- Subscribers prioritise delivery in causal order: when block `b` arrives, peers that haven't seen all of `b.parents` request them first.
- Out-of-order arrivals are buffered with a TTL (drop blocks whose parents haven't arrived within N rounds — protects against memory-amplification spam).

---

## 3.  Why this is V1.5 and not V1

- V1's BFT properties don't require leaderless emission; the leader-rotation + Crooks-MEV refunds + encrypted mempool already close the worst MEV/censorship vectors.
- Leaderless emission needs:
  - A new soundness theorem (multi-proposer-per-height + MCC fork-choice convergence under f<n/3 Byzantine).
  - A migration path from leader-rotation to leaderless (governance flag + at-fork-epoch flip).
  - Adversarial test suite covering split-brain proposals, BD-action grinding, network-level causal-delivery TTL gaming.
- Estimated effort: 2-4 weeks deep design + impl + adversarial-test harness.  Not in the May–Oct 2026 V1 sprint window.

---

## 4.  Acceptance gates (for V1.5)

1. **Theorem.**  Written argument that under f<n/3 Byzantine validators, the leaderless-emission + MCC fork-choice + antichain mempool together preserve safety (no two conflicting chains finalize) and liveness (every honest tx gets included within `O(stake_weighted_block_rate)` rounds).
2. **Code.**  Trait `LeaderlessProposer` behind `--cfg doctrine_v1_5`; default `--cfg doctrine_v1` keeps current Tendermint hot path.
3. **Tests.**  Cross-impl proptest comparing leader-rotation vs. leaderless outputs on shared input streams.  Adversarial harness for split-brain, BD-action grinding, causal-delivery TTL.
4. **Soak.**  72-hour devnet running V1.5 alongside V1 (different chain_ids) with zero finality regressions.

---

## 5.  Open questions

- Should leaderless emission keep stake-weighted VRF eligibility, or move to a stake-stratified Poisson process for smoother emission rates?
- Does V1.5 inherit V1's encrypted mempool, or does the multi-proposer setup demand a different MEV-defense model (e.g. threshold-encrypted blocks)?
- Network-level causal-delivery TTL — pick a value or make it a chain parameter?

---

## 6.  Status

**Draft.**  Not blocking V1.  Ship V1.0 mainnet first; revisit V1.5 design in Q4 2026 after audit closure and operator stability.

## 7. Implementation status (updated 2026-05-29)

All leaderless **components** are built, tested, and merged — every one
default-off behind the `doctrine_v1_5` Cargo feature (or an off-by-default
runtime flag), so the V1 leader-rotation hot path is unaffected.

| Phase | What | Status |
|---|---|---|
| 1 | `LightCone::insert` antichain-parents enforcement (§2.2), `set_enforce_antichain_parents`, default off | ✅ PR #476 |
| 0 | `doctrine_v1_5` feature + `trait LeaderlessProposer` + `EligibilityContext` seam (consensus) | ✅ PR #477 |
| 2 | `VrfLeaderlessProposer` — stake-weighted VRF eligibility (§2.1) + liveness floor | ✅ PR #478 |
| 3 | `FutureBlockBuffer` — causal-delivery buffer (§2.3) in `evaporchain-network` | ✅ PR #479 |
| 4a | `research/tla/LeaderlessConsensus.tla` — safety/liveness theorem (gate #1), TLC-green | ✅ PR #480 |

**Deferred to Q4 2026 (the BFT-voting surgery — owner-steered):**
- **4b** — wire the above into `tendermint.rs`: emission gate (`tick:4449`
  `am_i_proposer` → `is_eligible`), relax the single-leader check
  (`on_message:5056`, verify VRF ticket), route competing proposals to MCC
  fork-choice instead of first-accepted-wins (`:5555`), wire the buffer at
  ingest (`:5146`). Needs: a `recent_block_rate` helper (from `committed_at`),
  a validator_id-scoped VRF input variant, a `block_production_mode`
  governance key. Multi-session, high-risk; paused before mainnet by design.
- **4c** — adversarial harness (split-brain, BD-action grinding, TTL gaming). Depends on 4b.
- **4d** — 72h V1.5-alongside-V1 soak (gate #4). Gated on the cluster (T3.1).

**BD-action (§2.2) — resolved 2026-05-29 as measure-only (PR #483).**
Doctrine decision: ship the Sorkin/BD-action as an *observability metric*
(`LightCone::bd_action_doubled` = `2N − R`, link-count reading), NOT an
insert gate. Rationale: the simple-proxy insert-gate reduces to a
duplicate of the SUB-N6 fan-in cap, and the faithful interval-cardinality
action's antichain theorem is unproven (`research/IMPOSSIBLE_RESEARCH_STACK.md`).
A real enforcement gate stays deferred pending a proven theorem — not a
threshold guess. (The antichain-at-insert rule from §2.2 *is* enforced,
opt-in, via Phase 1 / PR #476.)

---

*Drafted 2026-05-16 as part of the post-audit "what's heavy and parallel-able" sweep.  Sister docs: `HEAVY_WORK_QUEUE.md`, `MAINNET_SPRINT_PLAN_2026_05_11.md`, `DOCTRINE_PUNCH_LIST.md` Layer 6.*
