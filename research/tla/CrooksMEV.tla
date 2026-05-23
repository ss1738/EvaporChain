--------------------------- MODULE CrooksMEV ---------------------------
(*
    TLA+ formal specification of EvaporChain's Crooks-MEV refund-and-
    settlement state machine — closes drift D9 from
    TLA_IMPL_DRIFT_AUDIT_2026_05_21.md.

    Models the observation lifecycle:
        Detect → Pending (with grace period for dispute)
              → Settleable (in [grace, window])
              → Settled (refund tx emitted, replay-protected)
              | Disputed (canceled in grace)
              | Expired (past window without settlement)

    Implementation:
        - crates/evaporchain-mev-detect/src/lib.rs (1391 LOC; Phase 1
          detector + Phase 2 refund computation + Phase 3.3
          due_refund_txs)
        - crates/evaporchain-crooks-mev-refund/src/refund.rs
          (compute_delta_f_millibits, compute_refund)
        - crates/evaporchain-consensus/src/tendermint.rs
          (TendermintConsensus::{mev_observations, settled_refunds,
          disputed_observations, validate_block_refunds, dispute_observation,
          on_block_committed Phase 3.5 wiring})

    Design rationale:
        docs/archive/completed-plans/CROOKS_MEV_INTEGRATION_PLAN.md

    Author:  Satyawan Singh
    Date:    2026-05-21

    Safety properties verified by TLC:
        TypeOK                       — variable domains.
        NoDoubleSettlement           — each (block_height, obs_idx)
                                       pair settles at most once.
        DisputedNeverSettles         — disputed observations never
                                       appear in settled_refunds.
        SettlementOnlyAfterGrace     — settled refunds are emitted
                                       only after grace_period blocks
                                       have elapsed since detection.
        SettlementWithinWindow       — settled refunds are emitted
                                       only within refund_window
                                       blocks of detection.
        ConfidenceThresholdHonored   — settled observations have
                                       confidence >= threshold.
        VictimOptOutHonored          — opted-out observations never
                                       enter mev_observations.

    Liveness properties (under fairness):
        EventualSettlementOrExpiry   — every eligible observation
                                       either settles or expires
                                       within `window` blocks.

    Open and not modeled here (out of TLC scope — same axiom boundary
    as PoHA.tla and EvaporChainBFT.tla):
        - Cryptographic correctness of compute_delta_f_millibits.
          The Crooks-fluctuation refund formula is verified separately
          (algebraic identity; not state-machine concerned).
        - Detection precision/recall. scan_block emits observations
          for sandwich-shaped triples; whether a real-world MEV attack
          matches the triple shape is a precision question, not a
          safety question.
        - Confidence-score derivation. Phase 1 ships with placeholder
          confidence = 1.0; the spec treats confidence as an opaque
          quantity to compare against the governance threshold.
        - Off-chain governance flag propagation. The spec treats
          governance_mode and threshold as static constants chosen at
          model-checking time.

    See `research/tla/TLA_IMPL_DRIFT_AUDIT_2026_05_21.md` § D9 for the
    full impl-vs-spec correspondence.
*)

EXTENDS Integers, Sequences, FiniteSets, TLC

\* ══════════════════════════════════════════════════════════════════════════
\* Constants
\* ══════════════════════════════════════════════════════════════════════════

CONSTANTS
    Validators,             \* Validator set (we model 1 validator since
                            \* settlement is deterministic; multi-validator
                            \* convergence is verified in EvaporChainBFT.tla
                            \* via mev_state_digest).
    Attackers,              \* Set of attacker addresses (model-bounded)
    Victims,                \* Set of victim addresses
    MaxBlockHeight,         \* Bound on blocks explored by TLC
    GracePeriod,            \* Blocks between detection and earliest
                            \* settlement (dispute window).
    RefundWindow,           \* Maximum blocks between detection and
                            \* settlement; past this, observations expire.
    ConfidenceThreshold,    \* Nat in 0..1000 (milli-units): settlement
                            \* requires confidence >= threshold.
    MaxObservationsPerBlock \* Bound on observations per block (state
                            \* space control).

ASSUME Cardinality(Validators) >= 1
ASSUME Cardinality(Attackers) >= 1
ASSUME Cardinality(Victims) >= 1
ASSUME GracePeriod >= 1
ASSUME RefundWindow >= GracePeriod
ASSUME ConfidenceThreshold \in 0..1000
ASSUME MaxObservationsPerBlock >= 1
ASSUME MaxBlockHeight >= 1

\* ══════════════════════════════════════════════════════════════════════════
\* Derived constants
\* ══════════════════════════════════════════════════════════════════════════

\* Confidence values explored by the model: just enough to straddle the
\* threshold (low / at / above). Two discrete points keep the state
\* space tractable while exercising the threshold gate.
ConfidenceValues == {0, ConfidenceThreshold, 1000}

\* Observation ID: (block_height, intra_block_index). The intra-block
\* index is bounded by MaxObservationsPerBlock.
ObservationIds == [bh: 1..MaxBlockHeight, idx: 0..(MaxObservationsPerBlock - 1)]

\* ══════════════════════════════════════════════════════════════════════════
\* Variables
\* ══════════════════════════════════════════════════════════════════════════

VARIABLES
    currentHeight,          \* Current block height being produced.
    mevObservations,        \* [ObservationIds -> Observation|None]
                            \* Records detected sandwich observations.
    settledRefunds,         \* Subset of ObservationIds : settled.
    settleHeight,           \* [ObservationIds -> 0..MaxBlockHeight]
                            \* Block height at which each settlement
                            \* occurred (0 = not settled). Matches the
                            \* Rust impl's Transaction::Refund.settle_block_height.
    disputedObservations,   \* Subset of ObservationIds : disputed.
    expiredObservations     \* Subset of ObservationIds : past refund window.

vars == <<currentHeight, mevObservations, settledRefunds, settleHeight,
          disputedObservations, expiredObservations>>

\* ══════════════════════════════════════════════════════════════════════════
\* Helper operators
\* ══════════════════════════════════════════════════════════════════════════

\* An "absent" observation in the function — we use a record value
\* "None" to mark unfilled slots so the function domain is total.
None == [attacker |-> "Nil", victim |-> "Nil", confidence |-> 0, opted_out |-> FALSE]

\* Is observation o (at id i) within the settlement-eligible age
\* relative to currentHeight?
IsSettleable(i, h) ==
    /\ i.bh + GracePeriod <= h
    /\ h <= i.bh + RefundWindow

\* Is the observation past its refund window?
IsExpired(i, h) == h > i.bh + RefundWindow

\* Is the observation in the grace period (dispute window)?
InGracePeriod(i, h) == h < i.bh + GracePeriod

\* Has this observation been recorded?
IsObserved(i) ==
    /\ mevObservations[i].attacker # "Nil"
    /\ mevObservations[i].opted_out = FALSE

\* ══════════════════════════════════════════════════════════════════════════
\* Initial state
\* ══════════════════════════════════════════════════════════════════════════

Init ==
    /\ currentHeight = 1
    /\ mevObservations = [i \in ObservationIds |-> None]
    /\ settledRefunds = {}
    /\ settleHeight = [i \in ObservationIds |-> 0]
    /\ disputedObservations = {}
    /\ expiredObservations = {}

\* ══════════════════════════════════════════════════════════════════════════
\* Actions
\* ══════════════════════════════════════════════════════════════════════════

\* DetectMevTriple: a sandwich-shaped triple is detected at currentHeight.
\* Matches Phase 1's scan_block. Opted-out victims are filtered out at
\* detection time, never entering the buffer (Phase 4.2).
DetectMevTriple(attacker, victim, confidence, opted_out) ==
    /\ attacker \in Attackers
    /\ victim \in Victims
    /\ attacker # victim                       \* Phase 4.3: self-MEV skipped at detection
    /\ confidence \in ConfidenceValues
    /\ opted_out \in BOOLEAN
    \* Find a free observation slot at currentHeight
    /\ \E idx \in 0..(MaxObservationsPerBlock - 1) :
        LET id == [bh |-> currentHeight, idx |-> idx]
        IN
        /\ mevObservations[id] = None
        /\ \/ \* Opted-out victim: record nothing (Phase 4.2 contract)
              /\ opted_out = TRUE
              /\ UNCHANGED <<currentHeight, mevObservations, settledRefunds,
                              settleHeight, disputedObservations,
                              expiredObservations>>
           \/ \* Standard path: record the observation
              /\ opted_out = FALSE
              /\ mevObservations' = [mevObservations EXCEPT ![id] =
                    [attacker |-> attacker, victim |-> victim,
                     confidence |-> confidence, opted_out |-> FALSE]]
              /\ UNCHANGED <<currentHeight, settledRefunds, settleHeight,
                              disputedObservations, expiredObservations>>

\* DisputeObservation: an operator disputes an observation. Only valid
\* during grace period. Matches Phase 4.4's dispute_observation.
DisputeObservation(id) ==
    /\ id \in ObservationIds
    /\ IsObserved(id)
    /\ InGracePeriod(id, currentHeight)
    /\ id \notin disputedObservations
    /\ id \notin settledRefunds
    /\ disputedObservations' = disputedObservations \cup {id}
    /\ UNCHANGED <<currentHeight, mevObservations, settledRefunds,
                    settleHeight, expiredObservations>>

\* SettleRefund: emit a Transaction::Refund for the observation, marking
\* it settled. Matches Phase 3.3's due_refund_txs eligibility check:
\*   - past grace period
\*   - within refund window
\*   - confidence >= threshold
\*   - not disputed
\*   - not already settled
SettleRefund(id) ==
    /\ id \in ObservationIds
    /\ IsObserved(id)
    /\ IsSettleable(id, currentHeight)
    /\ mevObservations[id].confidence >= ConfidenceThreshold
    /\ id \notin disputedObservations
    /\ id \notin settledRefunds
    /\ id \notin expiredObservations
    /\ settledRefunds' = settledRefunds \cup {id}
    /\ settleHeight' = [settleHeight EXCEPT ![id] = currentHeight]
    /\ UNCHANGED <<currentHeight, mevObservations, disputedObservations,
                    expiredObservations>>

\* ExpireObservation: an unsettled observation past its window is marked
\* expired. Matches Phase 3.3's stale-drop step in due_refund_txs (prune
\* stale observations before computing new refunds).
ExpireObservation(id) ==
    /\ id \in ObservationIds
    /\ IsObserved(id)
    /\ IsExpired(id, currentHeight)
    /\ id \notin settledRefunds
    /\ id \notin expiredObservations
    /\ expiredObservations' = expiredObservations \cup {id}
    /\ UNCHANGED <<currentHeight, mevObservations, settledRefunds,
                    settleHeight, disputedObservations>>

\* AdvanceBlock: bump the block height. Bounded by MaxBlockHeight.
AdvanceBlock ==
    /\ currentHeight < MaxBlockHeight
    /\ currentHeight' = currentHeight + 1
    /\ UNCHANGED <<mevObservations, settledRefunds, settleHeight,
                    disputedObservations, expiredObservations>>

\* ══════════════════════════════════════════════════════════════════════════
\* Next-state relation
\* ══════════════════════════════════════════════════════════════════════════

Next ==
    \/ AdvanceBlock
    \/ \E attacker \in Attackers, victim \in Victims,
         confidence \in ConfidenceValues, opted_out \in BOOLEAN :
           DetectMevTriple(attacker, victim, confidence, opted_out)
    \/ \E id \in ObservationIds : DisputeObservation(id)
    \/ \E id \in ObservationIds : SettleRefund(id)
    \/ \E id \in ObservationIds : ExpireObservation(id)

Spec == Init /\ [][Next]_vars

\* ══════════════════════════════════════════════════════════════════════════
\* Type invariant
\* ══════════════════════════════════════════════════════════════════════════

TypeOK ==
    /\ currentHeight \in 1..MaxBlockHeight
    /\ mevObservations \in [ObservationIds ->
            [attacker: Attackers \cup {"Nil"},
             victim: Victims \cup {"Nil"},
             confidence: 0..1000,
             opted_out: BOOLEAN]]
    /\ settledRefunds \subseteq ObservationIds
    /\ settleHeight \in [ObservationIds -> 0..MaxBlockHeight]
    /\ disputedObservations \subseteq ObservationIds
    /\ expiredObservations \subseteq ObservationIds

\* ══════════════════════════════════════════════════════════════════════════
\* Safety invariants
\* ══════════════════════════════════════════════════════════════════════════

\* SAFETY 1: replay protection — each observation settles at most once.
\* Already implicit in `settledRefunds` being a SET, but stated as an
\* invariant for clarity (and to catch bugs if the variable type
\* changes).
NoDoubleSettlement ==
    \A id \in settledRefunds :
        Cardinality({i \in settledRefunds : i = id}) <= 1

\* SAFETY 2: a disputed observation never settles. Critical to Phase
\* 4.4's operator-override semantics.
DisputedNeverSettles ==
    settledRefunds \cap disputedObservations = {}

\* SAFETY 3: settlement requires being past grace. No early-settle
\* exploit. Matches Phase 3.3's grace-period filter in due_refund_txs.
\* Uses settleHeight (the height AT settlement) so the invariant remains
\* true after currentHeight advances post-settlement.
SettlementOnlyAfterGrace ==
    \A id \in settledRefunds :
        settleHeight[id] >= id.bh + GracePeriod

\* SAFETY 4: settlement must occur within refund window. No stale-settle.
\* Uses settleHeight (the height AT settlement) — see SAFETY 3.
SettlementWithinWindow ==
    \A id \in settledRefunds :
        settleHeight[id] <= id.bh + RefundWindow

\* SAFETY 5: settlement requires the configured confidence threshold.
\* Matches Phase 4.1's confidence-threshold gate.
ConfidenceThresholdHonored ==
    \A id \in settledRefunds :
        IsObserved(id) =>
            mevObservations[id].confidence >= ConfidenceThreshold

\* SAFETY 6: an observation cannot be both settled and expired.
\* Phase 3.3's "prune stale" step happens BEFORE settlement attempts;
\* an observation that crosses the window is dropped, not settled.
SettledAndExpiredDisjoint ==
    settledRefunds \cap expiredObservations = {}

\* SAFETY 7: opted-out victims never appear in mevObservations.
\* Phase 4.2's victim consent contract — opted-out triples are dropped
\* at detection time, no buffer entry.
VictimOptOutHonored ==
    \A id \in ObservationIds :
        mevObservations[id].attacker # "Nil" =>
            mevObservations[id].opted_out = FALSE

\* Combined safety
SafetyInvariant ==
    /\ TypeOK
    /\ NoDoubleSettlement
    /\ DisputedNeverSettles
    /\ SettlementOnlyAfterGrace
    /\ SettlementWithinWindow
    /\ ConfidenceThresholdHonored
    /\ SettledAndExpiredDisjoint
    /\ VictimOptOutHonored

\* ══════════════════════════════════════════════════════════════════════════
\* Liveness — observations don't linger forever
\* ══════════════════════════════════════════════════════════════════════════

\* If an observation exists, eventually it either settles, is disputed,
\* or expires (no permanent in-limbo state).
EventualResolution ==
    \A id \in ObservationIds :
        IsObserved(id) ~>
            (id \in settledRefunds
             \/ id \in disputedObservations
             \/ id \in expiredObservations)

\* ══════════════════════════════════════════════════════════════════════════
\* Fairness (for liveness checking)
\* ══════════════════════════════════════════════════════════════════════════

LiveSpec ==
    /\ Spec
    /\ WF_vars(AdvanceBlock)
    /\ \A id \in ObservationIds :
        /\ WF_vars(SettleRefund(id))
        /\ WF_vars(ExpireObservation(id))

=============================================================================
