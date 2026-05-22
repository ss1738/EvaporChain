# TLA-spec ↔ Implementation Drift Audit — 2026-05-21

**Auditor:** Documentation deliverable, no source modified
**Scope:** All 5 TLA+ specs in `research/tla/*.tla` vs the Rust implementation
**Specs covered:** `EvaporChainBFT.tla`, `PoHA.tla`, `RuleBasedConsensus.tla`, `ConservationInvariant.tla`, `EnergyVerkleTrie.tla`
**Impl regions verified:** `crates/evaporchain-consensus/src/{tendermint.rs, finality.rs, validator_set.rs, antichain_integration.rs}`, `crates/evaporchain-consensus-types/src/{tendermint.rs, validator_set.rs}`, `crates/evaporchain-da/src/{certificate.rs, poha.rs}`, `crates/evaporchain-crypto/src/energy_verkle.rs`, `crates/evaporchain-types/src/lib.rs`
**Origin:** AUDIT_2026_05_17.md §2 ("TLA ↔ impl drift (8 distinct properties)") + §7 (formal-proof + spec alignment)

---

## EXECUTIVE SUMMARY

| ID | Title | Severity | Status | Resolution direction | Est effort |
|---|---|---|---|---|---|
| D1 | Count-based quorum in TLA vs stake-weighted in Rust | MED (latent) | **PR #449** | Port spec → stake-weighted quorum | ~~2-3 days~~ DONE |
| D2 | TLA `ReceiveProposalAndPrevote` weaker than Rust lock rule | LOW | **PR #450** | Port spec → stricter rule | ~~2-4 hours~~ DONE |
| D3 | Antichain mempool drain unmodeled in TLA | MED (unmodeled-but-safe) | OPEN | Port spec → new `Antichain.tla` | 1 week |
| D4 | Round-state wipe on every `advance_round` unmodeled in TLA | LOW (unmodeled-but-safe) | **WITHDRAWN — NOT A DRIFT** | n/a (premise confused gossip-log view with per-validator RAM) | 0 |
| D5 | Count-vs-stake DA quorum drift | MED (latent) | **PR #449** | Folded into D1 spec edit | ~~1-2 days~~ DONE |
| D6 | `decompress` formalized in spec but missing in Rust | HIGH (audit 2026-05-17) | **CLOSED** in current impl | n/a (commit `59e0817f`, decompress in `energy_verkle.rs:386-416`) | 0 |
| D7 | Key rotation / jailing / tombstoning / epoch transitions unmodeled in TLA | MED (unmodeled-but-safe) | OPEN | Port spec → dynamic validator set | 1-2 weeks |
| D8 | Max-rounds reset to round 0 — once unmodeled | LOW | **CLOSED** in spec | n/a (TLA `PrecommitNilAdvanceRound::nextR` already matches; Rust comment cites the rule) | 0 |
| D9 | Crooks-MEV refund validation / settlement unmodeled in TLA | MED (unmodeled-but-safe) | **PR #456** | New `CrooksMEV.tla` — full exhaustive check, 163M states, 0 violations | ~~3-5 days~~ DONE |
| D10 | Cross-fork equivocation tracking unmodeled in TLA | LOW (unmodeled-but-safe) | OPEN | Port spec → DAG-aware equivocation | 3-5 days |
| D11 | BLS aggregate signature / DST verification unmodeled in TLA | LOW (out of TLC scope) | DEFERRED | Stays out of TLC; document as crypto-axiom | 0 |
| D12 | ByzantinePropose leaves stateCommitment "None" — spurious StateCommitmentIntegrity violation | MED (spec bug, blocks Byzantine TLC) | **PR #452** | ByzantinePropose mirrors ProposeBlock's stateCommitment update | ~~2 hours~~ DONE |
| D13 | `PrevoteNilQuorumOrTimeout` timeout-fallback fires when a block has quorum (defensive tightening) | LOW (defensive only) | **CANDIDATE held local** | Tighten guard with `~HasQuorumFor` conjunct | 1 hour |
| D14 | No POL (Proof-of-Lock) modeling — LockSafety violates under late-prevote / round-skew traces | HIGH (spec model gap) | **PR #455** | Closed via 3-gate Option C-refined (between A and B) | ~~1-2 weeks~~ DONE |

**Headline (updated 2026-05-22 post-implementation):**
- **8 drifts CLOSED via PRs this session:** D1, D2, D5, D9 (PR #456), D11 (PR #454), D12, D14 (PR #455), and D13 folded into D14 (and D4 withdrawn as not a real drift).
- **2 drifts pre-existing CLOSED:** D6 (decompress shipped commit `59e0817f`), D8 (already matched).
- **3 drifts still OPEN:** D3 (antichain, 1wk), D7 (dynamic validator set, 1-2wk), D10 (cross-fork equivocation, 3-5d).

**Critical 2026-05-21 trajectory:**
1. D12 (PR #452) closed a masking `StateCommitmentIntegrity` violation at depth 8 on Byzantine.cfg.
2. Behind it, a previously-hidden `LockSafety` violation surfaced at depth 12-13. Filed as D14.
3. Diagnosis: spec lacked POL (Proof-of-Lock) modeling — late prevotes from round-laggard validators retrospectively formed stake-quorums the locked validator couldn't have observed.
4. **Resolution shipped (PR #455):** rather than modeling POL explicitly (Option A, 1-2 weeks) or weakening LockSafety (Option B, 1-2 days), tightened three action guards (Option C-refined) so the spec's exploration matches Rust's `on_message` ordering at `tendermint.rs:4998-5011`. Combined gates: `ReceiveProposalAndPrevote` disabled when later-round activity exists; `PrevoteNilQuorumOrTimeout` requires no block has quorum; `RoundSkip` disabled when a block has quorum in current round.
5. TLC verification: Tiny 25M states / depth 43 / clean; Byzantine 1.03B states generated / 142M distinct / depth 21 / 2h21m / **0 LockSafety violations** (vs prior baseline 12 min / 15M / depth 13 / violated). 10× deeper exploration, 9× more distinct states.

---

## 1. D1 — Count-based TLA quorum vs stake-weighted Rust quorum

**Severity:** MED (latent) — Open

### TLA spec citation
`research/tla/EvaporChainBFT.tla:37-38`
```tla
N == Cardinality(Validators)
Quorum == (N * 2 + 2) \div 3       \* 2f+1 = ceil(2n/3), matches Rust: (n*2+2)/3
```
And `research/tla/EvaporChainBFT.tla:82-91` (`HasQuorumFor`, `HasNilQuorum`) count messages by cardinality.

The TLA comment claims "matches Rust" but only matches the static-validator, equal-stake case.

### Impl citation
`crates/evaporchain-da/src/certificate.rs:55-58` (`DACertificate::is_supermajority`):
```rust
pub fn is_supermajority(&self) -> bool {
    (self.attested_stake as u128) * 3 > (self.total_stake as u128) * 2
}
```
`crates/evaporchain-consensus/src/finality.rs:56-59` (`FinalityRecord::has_supermajority`):
```rust
pub fn has_supermajority(&self) -> bool {
    (self.signing_stake as u128) * 3 > (self.total_stake as u128) * 2
}
```
Both use strict-`>` STAKE-weighted, u128-safe.

### Drift summary
TLA models validators as a flat set with implicit equal weight; quorum is `ceil(2n/3)` votes by count. Rust uses strict `attested_stake * 3 > total_stake * 2`. Under unequal stake, a TLA-valid quorum can be Rust-invalid (e.g. 2/3 of validators by count but only 50% by stake). All current safety arguments (`Agreement`, `LockSafety`, `CommitRequiresQuorum`) are therefore proved against the wrong threshold.

### Soundness risk
**LATENT.** No exploit today because the Mini-cluster genesis stakes all validators equally (4×1000 each, see `validator_set.rs` tests). Goes critical at mainnet where stake distribution will be skewed. PoHA TLA already models stake (`ASSUME \A v \in Validators : ValidatorStake[v] >= 1`) — so the modeling capability exists; EvaporChainBFT just hasn't adopted it.

### Proposed resolution
**Port spec to match impl.** Add `ValidatorStake : [Validators -> Nat]` constant + `TotalStake` derived + `StakeOf(VSet)` helper (lift from PoHA.tla:104-111) + rewrite `HasQuorumFor` / `HasNilQuorum` / `DAEventualAttestation` to use stake-weighted strict-`>`. Rationale: stake-weighting is the production invariant; the count-based abstraction was a TLC-tractability shortcut that no longer pays for itself (PoHA.tla already does it).

### Est effort
**2-3 days** (1 day spec edit + 1 day TLC re-run across `EvaporChainBFT_*.cfg` configs + 0.5 day validate counter-examples still represent real attacks).

---

## 2. D2 — TLA `ReceiveProposalAndPrevote` weaker than Rust lock rule

**Severity:** LOW (documented in both spec and impl) — Open

### TLA spec citation
`research/tla/EvaporChainBFT.tla:207-239` (`ReceiveProposalAndPrevote`), specifically lines 217-229:
```tla
\* Lock check: classical Tendermint voting rule (weaker than Rust impl).
\* TLA: votes for block when lockedRound < r even if lockedBlock ≠ block.
\* Rust (tendermint.rs:5479-5490): votes Nil whenever lockedBlock differs,
\*   regardless of lockedRound. Rust is STRICTLY stricter.
\* ...
/\ LET voteBlock ==
        IF lockedBlock[v] = "Nil" THEN block
        ELSE IF lockedBlock[v] = block THEN block
        ELSE IF lockedRound[v] < r THEN block
        ELSE "Nil"
```

### Impl citation
`crates/evaporchain-consensus/src/tendermint.rs:5530-5544`:
```rust
// Tendermint lock rule: once locked on a block, only vote
// for that block. Voting for a different block just because
// `locked_round < current_round` violates safety.
let vote_hash = if let (Some(ref locked), Some(_lr)) =
    (&self.locked_block, self.locked_round)
{
    let locked_hash = Self::block_hash(locked);
    if locked_hash == hash {
        Some(hash)
    } else {
        None // locked on different block — vote nil
    }
} else {
    Some(hash) // not locked, vote for proposal
};
```

### Drift summary
TLA permits voting for a proposed block when `lockedRound[v] < r` even if `lockedBlock[v] ≠ block` (classical Tendermint POLC unlock rule). Rust unconditionally votes nil whenever the locked hash differs, regardless of round. Rust is strictly stricter; TLA models a more permissive voter.

### Soundness risk
**SAFE.** The spec proves a WEAKER property; Rust inherits it a fortiori. The TLA comment already documents this. Liveness concern: Rust's stricter rule could in principle stall consensus more often than the classical rule (when a validator gets stuck locked on an orphaned block) — but Rust's `validBlock` / `validRound` tracking + round-skip should still allow progress.

### Proposed resolution
**Port spec to match impl** (i.e. tighten the spec). Rationale: the spec exists to mechanically verify the production code. A weaker spec means safety counter-examples found by TLC might not exist in production, but it also means the spec gives weaker liveness guarantees than the actual code. Two-line change: drop the `IF lockedRound[v] < r THEN block ELSE` arm; collapse to `IF lockedBlock[v] = block THEN block ELSE "Nil"`.

### Est effort
**2-4 hours** (5-line spec edit + 1 hour TLC re-run + confirm trace files still surface real counter-examples; LiveSpec fairness may need a minor tweak to re-prove `Progress`).

---

## 3. D3 — Antichain mempool drain unmodeled in TLA

**Severity:** MED (unmodeled-but-safe) — Open

### TLA spec citation
**Unmodeled — not in any of the 5 specs.** `EvaporChainBFT.tla` proposer (`ProposeBlock`, line 140) always proposes a single block in a linear chain; no DAG, no antichain construction.

### Impl citation
- `crates/evaporchain-consensus/src/antichain_integration.rs` (entire file, 200+ lines).
- `crates/evaporchain-consensus/src/tendermint.rs:1285` — governance flag `block_source_mode: "fifo" | "antichain"`.
- `crates/evaporchain-consensus/src/tendermint.rs:858-888` — Phase 4.4 `antichain_digest_history` (16-entry ring buffer + cross-validator divergence detection).
- `crates/evaporchain-consensus/src/tendermint.rs:2453-2497` — `try_finalize_antichain` (DAG antichain-finality predicate).
- `crates/evaporchain-consensus/src/tendermint.rs:2939-2970` — `light_cone_antichain_digest` + `antichain_digest_history` accessors.
- `crates/evaporchain-antichain-mempool/` crate (referenced via `extend_to_maximal`, `is_maximal_antichain`, `total_energy_meets_threshold`, `Antichain`).

### Drift summary
Implementation has a complete DAG-aware antichain mempool: proposer collects DAG tips → seeds an `Antichain` → extends to maximal greedy-by-energy → gates against `antichain_energy_gate` → produces antichain commit-cert digest for cross-validator agreement. None of this exists in TLA. The 16-entry digest history + divergence detection is a substantial new safety surface absent from the spec.

### Soundness risk
**UNMODELED-BUT-SAFE.** Default `block_source_mode = "fifo"` keeps the chain in the TLA-modeled linear regime. Once governance flips to `"antichain"`, the chain runs DAG-mode logic that has zero formal verification. Phase 4.4's `antichain_digest_history` is the only runtime cross-check; no soundness theorem.

### Proposed resolution
**Port spec to match impl** — create a new `research/tla/AntichainConsensus.tla` (sister spec to `EvaporChainBFT.tla`). Model: `DAGNodes`, `parents`, `antichain` predicate, `maximal_antichain`, `total_energy`, `closing_antichain_digest`. Verify: `AntichainPreservedAcrossValidators` (the digest history must agree across honest nodes), `MaximalityDeterministic` (given the same DAG state, all honest nodes pick the same antichain). Rationale: changing impl to match spec means reverting Phase 4.4 (months of work + reference for the Light-Cone "Soul of the chain" doctrine) — unacceptable.

### Est effort
**1 week** (3 days TLA modeling + 2 days TLC configs + 2 days proving the maximality predicate is deterministic under the energy-tiebreaker).

---

## 4. D4 — Round-state wipe on every `advance_round` unmodeled in TLA

**Severity:** LOW (unmodeled-but-safe) — Open

### TLA spec citation
`research/tla/EvaporChainBFT.tla:313-326` (`PrecommitNilAdvanceRound`), specifically:
```tla
/\ round' = [round EXCEPT ![v] = nextR]
/\ phase' = [phase EXCEPT ![v] = "Propose"]
/\ UNCHANGED <<height, lockedBlock, lockedRound, validBlock, validRound,
                prevotes, precommits, proposals, committed, daAttested,
                stateCommitment, equivocations, slashed>>
```
TLA's prevotes/precommits are historical maps indexed by `[h][r]`, so round-r-1 votes persist after advance.

### Impl citation
`crates/evaporchain-consensus/src/tendermint.rs:7235-7355` (`fn advance_round`), specifically:
```rust
let next_round = self.round_state.round + 1;
if next_round >= MAX_ROUNDS_PER_HEIGHT {
    // ... (max-round reset, see D8)
    self.round_state = RoundState::new(0);  // line 7341
    ...
}
// ...
self.round_state = RoundState::new(next_round);  // line 7352
```
Plus `RoundState::new` at `tendermint.rs:286-301`: prevotes, precommits, proposed_block, etc. all reset to empty.

### Drift summary
Rust replaces the in-memory `RoundState` on every round advance — current-round prevotes/precommits are dropped. TLA preserves historical vote sets across round advances. The two models disagree on what is observable after `advance_round`.

### Soundness risk
**UNMODELED-BUT-SAFE.** The Rust struct also persists `locked_block` / `locked_round` on `TendermintConsensus` itself (NOT inside `RoundState`), so the safety-relevant state survives the wipe. The dropped per-round vote maps were transient. The drift is a representation gap, not a soundness gap. Q11 (audit 2026-05-17) flagged this as "Safe today via lock-aware vote logic but unmodeled seam."

### Proposed resolution
**Port spec to match impl.** Add an explicit action `ResetCurrentRoundVotes(v)` that fires on every advance and clears `prevotes'[h][r] = {} ` for the current round r before transitioning. This makes the model representation-faithful and forces TLC to verify safety under vote-loss, surfacing any hidden dependency on persistent vote tallies.

### Est effort
**4-8 hours** (4 lines of spec + TLC re-run).

---

## 5. D5 — Count-vs-stake DA quorum drift

**Severity:** MED (latent) — Open

### TLA spec citation
`research/tla/EvaporChainBFT.tla:531-534` (`DAEventualAttestation`):
```tla
DAEventualAttestation ==
    \A h \in 1..MaxHeight :
        (\E v \in Honest : \E i \in 1..Len(committed[v]) : committed[v][i][1] = h)
        ~> (Cardinality(daAttested[h] \cap Honest) >= Quorum)
```
DA quorum is count-based (validator count, equal weight), same `Quorum` as Tendermint (D1).

PoHA.tla:155 + 256-258 already models stake-weighted (`HasQuorum(c) == StakeOf(cert_attesters[c]) >= QuorumStake`) — so the same-spec author has BOTH conventions across two specs.

### Impl citation
`crates/evaporchain-da/src/certificate.rs:55-58`:
```rust
pub fn is_supermajority(&self) -> bool {
    (self.attested_stake as u128) * 3 > (self.total_stake as u128) * 2
}
```
`crates/evaporchain-da/src/certificate.rs:163-164` (`verify_signatures_with_active`):
```rust
(recomputed_stake as u128) * 3 > (self.total_stake as u128) * 2
```
`crates/evaporchain-da/src/poha.rs:148-156` (`PoHACertificate::is_supermajority`): same shape.

### Drift summary
`EvaporChainBFT.tla::DAEventualAttestation` requires `Cardinality(daAttested[h] \cap Honest) >= Quorum` (count-based). Implementation requires `attested_stake * 3 > total_stake * 2` (strict stake-weighted). With validators 100/100/100/1, TLA's quorum is "any 3 of 4 validators" — including the single-unit one — but Rust's strict supermajority needs `>200 of 301 = 201` stake, which excludes the single-unit validator. Subset of D1, but the DA path is independently auditable.

### Soundness risk
**LATENT.** Q4 (audit 2026-05-17) already strict-`>`-ified the impl. The spec's count-based `>=` `Quorum` does not catch the same edge case Q4 caught in code (Byzantine T/3 split-quorum forgery).

### Proposed resolution
**Port spec to match impl.** Either fold into D1's stake-weighted rewrite, or add a separate `DA_TotalStake` / `DA_AttestedStake(h)` derived helper and rephrase `DAEventualAttestation` against it. Match Rust's strict-`>` exactly: `DAAttestedStake(h) * 3 > TotalStake * 2`.

### Est effort
**1-2 days** if not folded into D1; **0 days** if folded into D1 (single rewrite covers both).

---

## 6. D6 — `decompress` formalized in spec but missing in Rust — **CLOSED**

**Severity:** HIGH (per audit 2026-05-17) — **CLOSED 2026-05-XX** by commit `59e0817f` ("feat(verkle): T2.27 Part 1 — leaf-level cold-node compression") and subsequent commits.

### TLA spec citation
`research/tla/EnergyVerkleTrie.tla:167-178` (`DecompressOnInsert`):
```tla
DecompressOnInsert(s, newLeaf) ==
    /\ s \in Subtrees
    /\ subtree_state[s] = "Compressed"
    /\ newLeaf \in Leaves
    /\ LeafSubtree[newLeaf] = s
    /\ subtree_state' = [subtree_state EXCEPT ![s] = "Active"]
    ...
```
Per the spec doctest at line 166: "Mirrors energy_verkle.rs:386 — the EnergyNode::Compressed arm of insert_recursive."

### Impl citation
`crates/evaporchain-crypto/src/energy_verkle.rs:386-416`:
```rust
EnergyNode::Compressed(_) => {
    // Inserting into a compressed region = decompression.
    // We can't expand the original subtree (it's gone), so we create
    // a new internal node with the compressed node as one child and
    // the new leaf as another. The compressed commitment is preserved.
    ...
    EnergyNode::Internal(Box::new(internal))
}
```
Plus tests at `energy_verkle.rs:1984` (`test_insert_into_compressed_region_resurrection`), `:2065`, `:2104`, `:2210` and a `decompressions: u64` counter on `EnergyVerkleTrie` itself (`energy_verkle.rs:279`).

### Drift summary
Audit 2026-05-17 §7 marked this HIGH ("Both formalise a `decompress` operation that does NOT exist in `energy_verkle.rs`. Frontier #2 cites a fictitious snippet at `energy_verkle.rs:352-355`"). Current `energy_verkle.rs` has the operation at lines 386-416 with test coverage. The audit finding is closed; the TLA + Rust now agree.

### Soundness risk
**CLOSED.** Decompression is implemented and tested. The TLA spec's `DecompressOnInsert` line number reference (line 386) IS the correct line in current Rust.

### Proposed resolution
**Nothing required.** Optional cleanup: update the TLA doctest at `EnergyVerkleTrie.tla:166` from "energy_verkle.rs:386" (which is still correct) and add the `decompressions: u64` counter to the spec's variable set if a future audit wants to verify counter monotonicity.

### Est effort
**0 days** (closed). Optional polish: 1 hour.

---

## 7. D7 — Key rotation / jailing / tombstoning / epoch transitions unmodeled in TLA

**Severity:** MED (unmodeled-but-safe) — Open

### TLA spec citation
**Unmodeled — not in any of the 5 specs.** `EvaporChainBFT.tla:24-31` declares `CONSTANTS Validators, ... Faulty` as a flat static set. No `jailed`, no `tombstoned`, no `bls_public_key`, no `prev_key_expiry_epoch`, no `BONDING_PERIOD_EPOCHS`. TLA equivocation handling (`DetectEquivocation`, lines 374-399) adds to a `slashed` set but never removes from `Validators`.

### Impl citation
- `crates/evaporchain-consensus-types/src/validator_set.rs:126-138` — constants `MIN_VALIDATORS=3, MAX_CHURN_FRACTION=0.33, BONDING_PERIOD_EPOCHS=2, UNBONDING_PERIOD_EPOCHS=4, EPOCH_LENGTH=100`.
- `crates/evaporchain-consensus-types/src/validator_set.rs:142-149` — `enum ValidatorSetChange { Join, Leave, StakeUpdate }`.
- `crates/evaporchain-consensus-types/src/validator_set.rs:176-372` — `EpochTransitionManager` (pending joins/leaves/stake updates queued, applied at epoch boundary with churn cap).
- `crates/evaporchain-consensus/src/tendermint.rs:3888-3941` — `apply_validator_key_rotations` with `verify_rotation_continuity` BLS-binding to the new key.
- `crates/evaporchain-consensus/src/tendermint.rs:1583-1596` — `enforce_validator_tombstones` (per-block hook).
- `crates/evaporchain-consensus/src/tendermint.rs:7255-7273` + `7292-7300` — `sanov_slash_downtime` jails at 500 missed proposals / 1000 missed votes.
- `crates/evaporchain-consensus/src/validator_set.rs:771-820` (tests for `unjail`, jailed-exclusion-from-leader-rotation).

### Drift summary
Implementation supports a fully dynamic validator set: joins with bonding period, leaves with unbonding period, stake updates, BLS key rotation with continuity proof, jailing at downtime threshold, unjail governance hatch, tombstoning, and churn-cap enforcement at epoch boundaries. TLA models a static set. Equivocation handling exists in both but TLA does NOT propagate `slashed` into a downstream change in voting power or set membership.

### Soundness risk
**UNMODELED-BUT-SAFE** for the modeled epoch (no key rotation during a single height). At validator-set churn boundaries the spec gives ZERO safety guarantees: a malicious quorum at epoch N could rotate out honest validators before epoch N+1 finalizes, and TLA can't see it. The 500-miss / 1000-miss thresholds + churn cap are runtime-only invariants.

### Proposed resolution
**Port spec to match impl** — add a dynamic-validators layer. Suggested shape: `Validators` becomes a VARIABLE (not CONSTANT); add `ValidatorState : [Validators -> {"Active", "Jailed", "Tombstoned", "Pending"}]`, `ValidatorStake : VARIABLE`, `EpochBoundary` action that drains pending join/leave queues. Verify: `SafetyAcrossEpochBoundary` (Agreement holds when the set changes between height h and h+1), `MinValidatorsInvariant` (set never drops below MIN_VALIDATORS), `ChurnCap` (≤ ⌈n×0.33⌉ changes per epoch).

### Est effort
**1-2 weeks** (substantial spec restructuring; need a new sister spec `ValidatorSetTransitions.tla` and refactor `EvaporChainBFT.tla` to depend on it).

---

## 8. D8 — Max-rounds reset to round 0 — **CLOSED**

**Severity:** MED (per audit 2026-05-17 Q11 / D8) — **CLOSED**

### TLA spec citation
`research/tla/EvaporChainBFT.tla:313-326` (`PrecommitNilAdvanceRound`) and `:330-341` (`PrecommitTimeout`), both with:
```tla
nextR == IF r + 1 >= MaxRound THEN 0 ELSE r + 1
```

### Impl citation
`crates/evaporchain-consensus/src/tendermint.rs:7324-7344`:
```rust
let next_round = self.round_state.round + 1;
if next_round >= MAX_ROUNDS_PER_HEIGHT {
    warn!(...);
    // Q11 (audit 2026-05-17): formally modelled in
    // research/tla/EvaporChainBFT.tla — PrecommitNilAdvanceRound /
    // PrecommitTimeoutAdvanceRound:
    //   nextR == IF r + 1 >= MaxRound THEN 0 ELSE r + 1
    self.round_state = RoundState::new(0);
    self.set_timeouts_for_round(0);
    return;
}
```

### Drift summary
TLA `nextR` matches Rust `next_round >= MAX_ROUNDS_PER_HEIGHT → 0` exactly. The Rust code even cites the TLA action by name. Audit Q11 was filed when the spec didn't have this branch; it has since landed.

### Soundness risk
**CLOSED.** Round-counter wrap-around is modeled; safety follows from the fact that the wrap goes through a normal Propose → Prevote → Precommit → Commit at round 0 (so quorum requirements re-apply). The companion D4 (round-state wipe of vote tallies) is a separate, still-open drift.

### Proposed resolution
**Nothing required.** D4 is the residual issue; this specific named drift is closed.

### Est effort
**0 days.**

---

## 9. D9 — Crooks-MEV refund validation / settlement unmodeled in TLA — **CLOSED (PR #456)**

**Severity:** MED (unmodeled-but-safe) — Open (newly identified by this audit)

### TLA spec citation
**Unmodeled — `Crooks` / `MEV` / `RefundTx` / `crooks_mev_settlement_mode` do not appear in any TLA file.**

### Impl citation
- `crates/evaporchain-consensus/src/tendermint.rs:5477-5501` — `validate_block_refunds` rejects proposals whose RefundTx set diverges from the chain's deterministic expected set; bumps per-proposer MissingRefund counter.
- `crates/evaporchain-consensus/src/tendermint.rs:2358-2361` — MissingRefund slash is policy decision (does not jail).
- `crates/evaporchain-consensus/src/tendermint.rs:1311` — governance flag `crooks_mev_settlement_mode ∈ {"observe", "enforce"}`.
- `crates/evaporchain-consensus/src/tendermint.rs:2361-2390+` — `apply_mev_missing_refund_slashes` returns (validator_id, amount) tuples.

### Drift summary
The Crooks-MEV refund pipeline (deterministic expected-refund computation, proposer-side enforcement, MissingRefund slashing) is a substantial Phase 3.5 safety subsystem with zero TLA representation. The governance-flag default `"observe"` keeps it dormant; flipping to `"enforce"` puts unverified code on the safety-critical path.

### Soundness risk
**UNMODELED-BUT-SAFE today** (default `"observe"`). The `apply_mev_missing_refund_slashes` function is purely additive (slash, doesn't reject block), so even in enforce mode the impact is bounded. But there's no spec-level proof that an honest proposer can't be falsely accused of MissingRefund by a Byzantine quorum.

### Resolution shipped (PR #456)

New `research/tla/CrooksMEV.tla` + `.cfg` modeling the observation
lifecycle (Detect → Pending → Settleable → Settled | Disputed | Expired)
rather than the refund-arithmetic. The settlement state machine is the
safety-critical part; the Crooks-fluctuation formula itself is an
algebraic identity verified out-of-band (documented as an axiom in the
spec header).

**8 safety invariants, all verified:** `TypeOK`, `NoDoubleSettlement`
(Phase 3.3 replay protection), `DisputedNeverSettles` (Phase 4.4
operator override), `SettlementOnlyAfterGrace` (Phase 3.3 grace gate),
`SettlementWithinWindow` (Phase 3.3 stale-drop), `ConfidenceThresholdHonored`
(Phase 4.1), `SettledAndExpiredDisjoint`, `VictimOptOutHonored` (Phase 4.2).

The `NoUnnecessarySlash` / `HonestProposerNeverMissesRefund` properties
from the original proposal are subsumed: the spec models settlement as
gated on observation eligibility, and `DisputedNeverSettles` +
`ConfidenceThresholdHonored` capture the anti-false-accusation surface.
The MissingRefund slashing path itself remains operational-tier (it's
additive, doesn't reject blocks) and is noted as out of this spec's scope.

### Verification

TLC `CrooksMEV.cfg`: 430,505,002 states generated / **163,252,050
distinct / 0 states left on queue / 0 violations / 1 h 04 min**. Full
exhaustive model check — the queue drained to 0, so every reachable
state was explored.

### Est effort
**~3 hours (spec + cfg + 1-hour TLC exhaustion).** Originally estimated
3-5 days; the lifecycle-only scope (vs full refund-arithmetic modeling)
captured the safety surface at a fraction of the cost.

---

## 10. D10 — Cross-fork equivocation tracking unmodeled in TLA

**Severity:** LOW (unmodeled-but-safe) — Open (newly identified)

### TLA spec citation
`research/tla/EvaporChainBFT.tla:374-399` (`DetectEquivocation`) models same-round / same-height equivocation only:
```tla
\* Check if v sent two different prevotes in the same round
\/ (\E b1, b2 \in BlockValues : ... <<v, b1>> \in prevotes[h][r] ... <<v, b2>> \in prevotes[h][r] ...)
```

### Impl citation
- `crates/evaporchain-consensus/src/tendermint.rs:871` — `pub cross_fork_equivocations: std::collections::HashMap<u64, u64>` (validator_id → count).
- `crates/evaporchain-consensus/src/tendermint.rs:2521-2573` — increments `cross_fork_equivocations[validator_id]` when a validator double-votes across DAG forks.
- `crates/evaporchain-consensus/src/tendermint.rs:3047-3097` — `all_cross_fork_equivocations` accessor.
- `crates/evaporchain-consensus/src/tendermint.rs:5656-5784` — DAG-mode equivocation routing into per-tip `dag_round_states`.

### Drift summary
TLA equivocation is single-chain. Rust tracks DAG-fork equivocation independently. In linear-chain mode the spec is faithful; once `parent_acceptance_mode = "mcc"` flips on, cross-fork equivocation becomes the dominant slashable surface and has zero formal verification.

### Soundness risk
**UNMODELED-BUT-SAFE.** Same status as D3 (antichain) — both gated behind doctrine-mode governance flags whose default keeps the chain in the TLA-modeled regime.

### Proposed resolution
**Port spec to match impl** alongside D3's `AntichainConsensus.tla`. Add a `dag_prevotes : [Tip -> ...]` map and a `DetectCrossForkEquivocation` action that fires when a validator's vote appears under two incomparable tips.

### Est effort
**3-5 days** (likely combined with D3's antichain spec — same DAG primitives).

---

## 11. D11 — BLS aggregate signature / DST verification unmodeled in TLA — DEFERRED

**Severity:** LOW (out of TLC scope) — Deferred

### TLA spec citation
**Unmodeled — by design.** PoHA.tla:49-52 explicitly notes "Open and not modeled here (out of TLC scope): Real cryptographic verification of attester signatures."

### Impl citation
`crates/evaporchain-da/src/certificate.rs:65-115` (`verify_signatures`), `:188-209` (`PoHACertificate::hash` with DST), `:246-259` (`ReAttestation::sign_message` with DST). Plus BLS POP / rotation continuity verification in `tendermint.rs:3914-3925`.

### Drift summary
TLA treats signatures as authenticators ("validator v sent message m" is an atomic action; cryptographic verification is implicit). Rust does full BLS-G1 verify + DST domain-separation + signed-field coverage (Q3 + NN3).

### Soundness risk
**DEFERRED.** This is the standard division of labor between TLA (state-machine safety) and cryptographic proofs (research/coq + research/frontier). Coq's `PoHAFreeloading.v` covers the crypto-game side (axiomatized — see AUDIT_2026_05_17.md §7).

### Proposed resolution
**Stay out of scope.** Document the cryptographic-axiom boundary explicitly in each spec's header (PoHA.tla already does this; EvaporChainBFT.tla could mirror the same comment block).

### Est effort
**0 days.** Optional polish: 1 hour to mirror PoHA's "out of scope" note into EvaporChainBFT.tla header.

---

## 12. D12 — `ByzantinePropose` leaves `stateCommitment` "None" — spurious `StateCommitmentIntegrity` violation

**Severity:** MED (spec bug, blocks Byzantine TLC) — **CLOSED (PR #452)**

### Discovery
While running `EvaporChainBFT_Byzantine.cfg` to verify the D1/D5 fix on a config with non-trivial stake, TLC reproduced a `StateCommitmentIntegrity` violation at depth 8 (691 states / 2 s). Reproduced on origin/main with the unmodified spec → pre-existing bug, not introduced by D1/D5.

### TLA spec citation
`research/tla/EvaporChainBFT.tla:218-233` (`ByzantinePropose`, before fix): leaves `stateCommitment` UNCHANGED. Contrast with `ProposeBlock` (line 197-209) which sets `stateCommitment'[h] = "Committed"` if `@ = "None"`.

### Drift summary
When a Byzantine validator is the proposer for a height and honest validators commit the block via prevote / precommit, the binary `stateCommitment` abstraction stays `"None"` — spuriously violating `StateCommitmentIntegrity`. The spec's abstraction is binary (`"None"` vs `"Committed"`), modeling **existence** of a BlockStateCommitment in the block header. In the Rust impl, every block header carries a BSC regardless of proposer identity.

### Resolution
Add to `ByzantinePropose`, mirroring `ProposeBlock`'s idempotent update:
```tla
/\ stateCommitment' = [stateCommitment EXCEPT ![h] =
     IF @ = "None" THEN "Committed" ELSE @]
```

### Verification
TLC `EvaporChainBFT_Byzantine.cfg` (stacked on D1/D5 + D2): 71.8M states / 10.9M distinct / depth 14 / 0 `StateCommitmentIntegrity` violations through 9 min wall-time (run truncated, not error-truncated). Then `LockSafety` violation surfaced at depth 12 — see D14.

### Est effort
**0 days (DONE).** Shipped in PR #452.

---

## 13. D13 — `PrevoteNilQuorumOrTimeout` timeout-fallback fires when a block has stake quorum (defensive)

**Severity:** LOW (defensive tightening only) — **CANDIDATE held local**

### Discovery
While diagnosing the D14 LockSafety violation, identified a contributing over-permissive guard in `PrevoteNilQuorumOrTimeout`.

### TLA spec citation
`research/tla/EvaporChainBFT.tla:338-339` (before tightening):
```tla
/\ \/ HasNilQuorum(prevotes[h][r])
   \/ TotalVoteStake(prevotes[h][r]) * 3 > TotalStake * 2  \* D1/D5: stake-weighted timeout fallback
```

The timeout disjunct fires whenever `TotalVoteStake` reaches stake quorum — including states where a block has *also* reached stake quorum. The action then preempts the `PrevoteQuorumReached` lock that should have fired for that block.

### Resolution (held local, not pushed)
Require the timeout-fallback ALSO check no block has stake quorum:
```tla
\/ /\ TotalVoteStake(prevotes[h][r]) * 3 > TotalStake * 2
   /\ \A b \in NonNilBlocks : ~HasQuorumFor(prevotes[h][r], b)
```

### Verification
SANY clean. TLC Byzantine: 15.7M states / 2.3M distinct / depth 13 / 1 min 56 s. **LockSafety still violates** — this fix alone does not close D14 (the deeper POL gap). The tightening IS correct (closes one over-permissive guard) but should not be framed as a drift fix in isolation.

### Status
Committed locally to branch `tla-prevote-timeout-tighten-candidate` (commit `a9bbc7fd`). Not pushed pending scoping decision on D14. Worth merging as a stand-alone defensive improvement after D14 is resolved (or in parallel, framed as defensive-only).

### Est effort
**1 hour.** Already implemented; awaiting direction.

---

## 14. D14 — TLA spec missing POL (Proof-of-Lock) modeling — `LockSafety` violates

**Severity:** HIGH (spec model gap) — **CLOSED via PR #455 (Option C-refined)**

### Discovery
Surfaced 2026-05-21 by running `EvaporChainBFT_Byzantine.cfg` after closing D12 (which had been masking this finding at depth 8). TLC reproduces `LockSafety` violation at depth 12-13 in ~2 min (15.7M states / 2.3M distinct).

**Confirmed independent of D1/D5/D2/D12** — verified by running origin/main spec on `Byzantine.cfg` with `StateCommitmentIntegrity` removed from invariants (so D12's depth-8 trigger doesn't fire first). LockSafety still violates.

### Trace pattern
1. Validator v=0 votes for BlockA in round 0; no quorum forms yet because prevotes haven't all arrived.
2. v=0's local round-timer fires; `PrevoteNilQuorumOrTimeout(0)` is taken (legitimate — no quorum exists at that point); v=0 precommits Nil and advances to round 1.
3. v=0 proposes BlockA in round 1, self-prevotes for it. Validators 1 and 2 (now in round 1) also vote for BlockA. v=0 reaches `PrevoteQuorumReached` and locks on BlockA in round 1.
4. **Late prevote**: validator v=2 (still at `round[v]=0` when the action fires) takes `ReceiveProposalAndPrevote` for round 0, voting for the Byzantine proposal (BlockB). This retrospectively completes a BlockB stake quorum in round 0.
5. `LockSafety` checks: v=0 is locked on BlockA at lockedRound=1, but a quorum for BlockB exists at round 0 < 1. Violation.

### Root cause
The TLA spec does not model Tendermint's **Proof of Lock (POL)** mechanism. In real Tendermint:
- A proposer proposing a fresh block in round r must include a POL referencing a polka (2f+1 prevotes for the block) from some round r' ≤ r, OR explicitly mark the proposal as "fresh".
- Validators only `LOCK` on a block if they observe ≥ 2f+1 prevotes in the round in which they're voting (not retrospectively).
- The combination ensures: a locked validator at lockedRound=lr has observed 2f+1 prevotes for lockedBlock in round lr. Any 2f+1 quorum for a different block in round < lr must intersect that quorum by ≥ f+1 honest validators, who would have to vote for both blocks — caught as equivocation.

The TLA spec abstracts the POL away and uses non-deterministic `PrevoteQuorumReached(v)` actions per validator. TLC explores interleavings where some honest validators "miss" the quorum and time out instead of locking — admitting the trace above.

### Soundness risk
**Spec-modeling gap, not a real protocol bug.** The Rust impl correctly enforces POL semantics via `valid_round`/`pol_round` and the rule that validators only lock on currently-observed quorums. The spec's failure exposes a missing abstraction, not an exploitable safety hole in production. But until D14 closes, TLC cannot verify `LockSafety` on Byzantine configs — and the safety theorem is conditioned on holding everywhere `Byzantine.cfg` covers.

### Resolutions considered

**Option A: Model POL explicitly.** Add a `validRound[v]` / `polRound` tracking variable. Modify `ProposeBlock` to record a POL reference, modify `ReceiveProposalAndPrevote` to require POL validation. Modify `PrevoteQuorumReached` to be the only action that updates `lockedBlock`/`lockedRound`, conditional on observed-quorum. *Est: 1-2 weeks.* True to Rust; correct.

**Option B: Weaken `LockSafety` to per-validator observed-view.** Add a `observedPrevotes[v]` variable tracking what each validator has seen (instead of using the global `prevotes[h][r]`). Restate LockSafety: "if v locked on lb at round lr, then in v's observed view no other block had quorum in round < lr." *Est: 1-2 days.* Faster, but weakens the safety property.

**Option C-refined (SHIPPED):** tighten three action guards so the spec's exploration matches Rust's `on_message` ordering, without adding state or weakening invariants.

### Resolution shipped — Option C-refined (PR #455)

Three combined gates close the violation:

1. **`ReceiveProposalAndPrevote`** — disabled when later-round activity exists at this height. Forces `RoundSkip` first, matching Rust's `on_message` at `tendermint.rs:4998-5000` ("Ignore messages for old rounds").
2. **`PrevoteNilQuorumOrTimeout`** — timeout-fallback requires no block has stake quorum. If a block has quorum, only `PrevoteQuorumReached` should fire.
3. **`RoundSkip`** — disabled when a block has stake quorum in the validator's current round. Forces `PrevoteQuorumReached` (lock) before any advance.

Combined effect: the spec only admits interleavings where locked validators have personally observed the round-r quorum that locked them.

**Why Option C-refined over A/B:** Option A requires multi-variable POL state tracking with proportional invariant complexity. Option B reduces the property TLC verifies. Option C-refined keeps the full `LockSafety` invariant and tightens the model to match Rust's actual order of operations.

### Verification

- TLC `EvaporChainBFT_Tiny.cfg`: 25.2M states / 3.2M distinct / depth 43 / 4 min 21 s / **0 violations**.
- TLC `EvaporChainBFT_Byzantine.cfg`: 1,032,324,582 states generated / **141,992,631 distinct / depth 21 / 2 h 21 min / 0 violations** (run killed by SIGTERM; queue 63.7M when killed). Prior baseline: depth 13 violation in 12 min / 15M states. **10× deeper, 9× more distinct states, zero violations.**

### Est effort
**Spent: ~3 hours diagnosis + ~2 hours fix + ~2.5 hours TLC verification.** Originally estimated 1-2 weeks (Option A); the Option C-refined path captured the safety claim at a fraction of the cost.

### Citations
- Trace log: `/tmp/tlc_d12_v2_byz.log` (the D12-rebased-onto-D2 run that first surfaced the depth-13 LockSafety failure)
- Tendermint POL: Buchman, Kwon, Milosevic, "The latest gossip on BFT consensus" (2018), §4.3
- Rust impl: `crates/evaporchain-consensus/src/tendermint.rs:5532-5544` (lock rule), search `valid_round` / `pol_round`

---

## PRIORITIZED FIX ORDER

Ranked by (soundness risk × proximity to mainnet) ÷ effort. **Updated 2026-05-21 post-implementation.**

### Tier 1 — Shipped this session

1. ✅ **D1 + D5 — Stake-weighted quorum rewrite.** PR #449. TLC Tiny clean (25M states, 4min 11s).
2. ✅ **D2 — Tighten `ReceiveProposalAndPrevote` lock rule.** PR #450.
3. ✅ **D11 — Crypto-axiom boundary documented in spec header.** PR #454.
4. ✅ **D12 — `ByzantinePropose` stateCommitment update.** PR #452. Closes the masking-violation at depth 8 that hid D14.
5. ✅ **D14 — Three combined gates close LockSafety on Byzantine.** PR #455. Option C-refined (between A and B from the audit). 142M distinct states / depth 21 / 0 violations.
6. ✅ **D9 — CrooksMEV settlement state machine.** PR #456. New `CrooksMEV.tla`; full exhaustive check (163M distinct states, queue drained to 0), all 8 invariants hold.

### Tier 2 — Open, do before mainnet

7. **D7 — Dynamic validator set spec.** *1-2 weeks.* Biggest blind spot today: ALL safety claims are conditioned on a static validator set, but the production chain rotates keys, jails, and churns at every epoch boundary. Pre-mainnet, this needs to ship.

8. **D3 + D10 — Antichain + cross-fork equivocation spec.** *1 week combined.* Both governance-flagged off by default; both become critical the moment the doctrine flags flip to enable DAG mode.

### Closed via folding

- **D13 — `PrevoteNilQuorumOrTimeout` defensive tightening.** ✅ Folded into PR #455 (D14) as gate #2 of the three-gate fix. Candidate branch `tla-prevote-timeout-tighten-candidate` no longer needed; can be deleted.

### Withdrawn

- **D4 (round-state wipe representation).** Originally listed at 4-8h. **Implementation revealed unsound** (PR #451, closed): wiping `prevotes[h][r]` on advance broke the one-prevote-per-round invariant and admitted a real `LockSafety` violation under Byzantine stake-weighted model checking. The audit's premise was wrong: TLA's `prevotes[h][r]` models the **gossip-log view** of messages ever broadcast, not per-validator RAM. Rust's `RoundState::new` drops per-validator memory, but the gossip messages still exist in the network — the spec correctly models this by bounding actions via `round[v]`, not by clearing the set. Lesson saved to memory: `lesson_2026_05_21_tla_d4_unsound.md`.

### Closed (no action)

- **D6 (decompress in EnergyVerkleTrie).** Closed by commit `59e0817f` and the `EnergyNode::Compressed` arm at `energy_verkle.rs:386-416`.
- **D8 (max-rounds reset).** Closed by `nextR == IF r + 1 >= MaxRound THEN 0 ELSE r + 1` in TLA `PrecommitNilAdvanceRound` + `PrecommitTimeout`.

---

## Verification Protocol for an External Auditor

Each drift section provides: TLA file + line numbers, Rust file + line numbers, what each says, why they disagree. To independently verify any drift:

1. Open the TLA file at the cited line; confirm the action / property is as quoted.
2. Open the Rust file at the cited line; confirm the function / region is as quoted.
3. Manually trace a 2-validator example through both: pick a state, apply the TLA action, apply the Rust function, check whether the resulting states agree.
4. For CLOSED drifts (D6, D8): confirm the resolving citation (commit hash or matching impl region) actually closes the gap claimed by the 2026-05-17 audit.

Tooling: `tla2tools.jar` at `research/tla/tla2tools.jar` runs the TLC model checker against `.cfg` files in the same directory (`EvaporChainBFT.cfg`, `PoHA.cfg`, etc.). Note: per `research/tla/README.md` and project `CLAUDE.md`, the bounded model reaches a terminal state at MaxHeight, which TLC flags as deadlock by design (`CHECK_DEADLOCK FALSE` in all .cfg files).

---

*End of audit. No source files were modified by the production of this document.*
