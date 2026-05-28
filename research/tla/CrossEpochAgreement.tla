------------------- MODULE CrossEpochAgreement -------------------
(*
    TLA+ formal specification of EvaporChain's cross-epoch agreement
    safety — drift D7-Part2 from TLA_IMPL_DRIFT_AUDIT_2026_05_21.md.

    D7-Part1 (ValidatorSetTransition.tla, PR #458) verified the epoch-
    transition MANAGER in isolation (min-validators, bounded churn,
    bonding/unbonding). D7-Part2 — this spec — verifies the SAFETY
    consequence: a block committed by epoch N's validator set is not
    reorged by epoch N+1's (different) validator set.

    The classical dynamic-BFT safety mechanism is QUORUM INTERSECTION
    ACROSS RECONFIGURATION: any quorum in epoch N and any quorum in
    epoch N+1 must share at least one HONEST validator. That shared
    honest validator cannot vote for two conflicting blocks, so the two
    epochs cannot commit conflicting blocks — committed history is
    final across the boundary.

    Ratification context (impl): a validator-set change is a committed
    transaction (Transaction::ValidatorStake = Join, ValidatorExit =
    Leave), scanned from committed blocks in
    `tendermint.rs:on_block_committed` (lines 6279-6309) and applied at
    the boundary via `EpochTransitionManager::apply_epoch_transition`.
    So the OLD set ratifies the change by committing it; bonding /
    unbonding delays + the churn cap then gate when it takes effect.

    THE QUESTION THIS SPEC TESTS: EvaporChain's churn cap is COUNT-based
    (`max_churn = ceil(active_count * MAX_CHURN_FRACTION=0.33)`,
    validator_set.rs:241-243) but quorums are STAKE-weighted
    (`attested_stake * 3 > total_stake * 2`). Does a COUNT-bounded churn
    guarantee STAKE-quorum intersection across epochs? This spec checks
    it. If count-bounded churn permits a stake-quorum-intersection
    failure (e.g. a few high-stake validators rotate out), that is a
    real finding: the count-based cap would be insufficient for
    stake-weighted cross-epoch safety, and the transition should bound
    STAKE churn (or require old-set ratification of the new stake
    distribution).

    Author:  Satyawan Singh
    Date:    2026-05-22

    Properties verified by TLC:
        TypeOK                       — variable domains.
        ByzantineBoundBothEpochs     — faulty stake < 1/3 in both epochs
                                       (the standing assumption).
        HonestQuorumIntersection     — for EVERY epoch-N quorum and
                                       EVERY epoch-(N+1) quorum, the two
                                       share an honest validator. This is
                                       the cross-epoch agreement guarantee:
                                       if it holds, no conflicting commits
                                       across the boundary are possible.

    Out of TLC scope:
        - The full BFT round protocol (EvaporChainBFT.tla); this spec
          assumes a commit = some stake-quorum and reasons about quorum
          intersection, the safety-relevant abstraction.
        - Long-range / weak-subjectivity attacks beyond one boundary.
        - The numeric bonding/unbonding epoch arithmetic (D7-Part1).

    See `research/tla/TLA_IMPL_DRIFT_AUDIT_2026_05_21.md` § D7.
*)

EXTENDS Integers, FiniteSets, TLC

\* ══════════════════════════════════════════════════════════════════════════
\* Constants
\* ══════════════════════════════════════════════════════════════════════════

CONSTANTS
    AllValidators,          \* Universe of validator ids.
    StakeValues,            \* Set of per-validator stake values explored (e.g. {1,2,3}).
    MinValidators,          \* Min active set size (impl MIN_VALIDATORS = 3).
    EnforceStakeOverlap,    \* BOOLEAN: TRUE applies the candidate D7-Part2
                            \* remediation (stake-overlap requirement);
                            \* FALSE keeps only the impl's count-churn cap.
    EnforceC5,              \* BOOLEAN: TRUE applies the ENFORCEABLE C5 rule —
                            \* delayed stake activation (frozen stayers) + the
                            \* stayers holding >2/3 stake in both epochs.
    ByzDen                  \* Byzantine bound denominator: faulty stake < 1/ByzDen
                            \* of total. ByzDen=3 -> standard f<1/3; ByzDen=4 ->
                            \* tightened f<1/4 (the margin C5 needs for churn).

ASSUME Cardinality(AllValidators) >= MinValidators
ASSUME StakeValues \subseteq (Nat \ {0})   \* positive stakes
ASSUME MinValidators >= 1
ASSUME EnforceStakeOverlap \in BOOLEAN
ASSUME EnforceC5 \in BOOLEAN
ASSUME ByzDen \in (Nat \ {0})

\* ══════════════════════════════════════════════════════════════════════════
\* Variables — two consecutive epochs' validator sets + stakes + faulty set
\* ══════════════════════════════════════════════════════════════════════════

VARIABLES
    setN,           \* SUBSET AllValidators : epoch-N active set
    setN1,          \* SUBSET AllValidators : epoch-(N+1) active set
    stakeN,         \* [AllValidators -> StakeValues] : stakes in epoch N
    stakeN1,        \* [AllValidators -> StakeValues] : stakes in epoch N+1
    faulty          \* SUBSET AllValidators : Byzantine validators

vars == <<setN, setN1, stakeN, stakeN1, faulty>>

\* ══════════════════════════════════════════════════════════════════════════
\* Stake helpers
\* ══════════════════════════════════════════════════════════════════════════

RECURSIVE SumStake(_, _)
SumStake(S, stakeFn) ==
    IF S = {} THEN 0
    ELSE LET x == CHOOSE x \in S : TRUE
         IN stakeFn[x] + SumStake(S \ {x}, stakeFn)

StakeOfN(S)  == SumStake(S, stakeN)
StakeOfN1(S) == SumStake(S, stakeN1)
TotalN  == StakeOfN(setN)
TotalN1 == StakeOfN1(setN1)

\* A stake-weighted quorum in epoch N: subset of setN with > 2/3 of TotalN.
\* Matches `attested_stake * 3 > total_stake * 2`.
IsQuorumN(Q)  == /\ Q \subseteq setN  /\ StakeOfN(Q)  * 3 > TotalN  * 2
IsQuorumN1(Q) == /\ Q \subseteq setN1 /\ StakeOfN1(Q) * 3 > TotalN1 * 2

\* Count-based churn (impl): symmetric difference size. The impl caps
\* applied changes at ceil(|setN| * 1/3); we model the bound as
\* |setN △ setN1| * 3 <= |setN| (i.e. churn count <= 1/3 of the set).
ChurnCount == Cardinality((setN \ setN1) \cup (setN1 \ setN))
ChurnWithinCap == ChurnCount * 3 <= Cardinality(setN)

\* ── Candidate remediation — TLC-REJECTED (insufficient) ────────────────────
\* CANDIDATE: require the honest validators present in BOTH epochs
\* ("honest stayers") to hold a stake supermajority (> 2/3) in EACH epoch.
\* Intuition: if the honest carry-over is a supermajority by both stake
\* distributions, every quorum must touch it.
\*
\* TLC VERDICT: INSUFFICIENT. With EnforceStakeOverlap = TRUE, TLC still
\* finds a HonestQuorumIntersection violation (CrossEpochAgreement_
\* CandidateFix.cfg). Counterexample: a stayer (v4) triples its stake
\* 1->3 across the boundary, so an epoch-N+1 quorum can lean on
\* {faulty, new-joiner, the-fattened-stayer} and route around the OTHER
\* honest stayers, while an epoch-N quorum used those other stayers —
\* the two quorums then intersect only in the Byzantine validator.
\*
\* CONCLUSION: bounding SET overlap (even honest-stayer-supermajority) is
\* NOT enough. The stake REDISTRIBUTION among the post-transition set
\* must also be bounded. The real remediation is a design decision
\* (candidates: bound per-validator stake deltas across a boundary;
\* delay stake-update activation by an extra epoch so the boundary
\* quorum uses the OLD distribution; or require the new set+stakes to be
\* ratified by a >2/3 OLD-STAKE quorum, Tendermint-style). Left to the
\* owner — this spec PROVES the gap is real and the obvious fix fails.
HonestStayers == (setN \cap setN1) \ faulty
StayerSupermajorityFix ==          \* candidate C1 — TLC-REJECTED (see above)
    /\ StakeOfN(HonestStayers)  * 3 > TotalN  * 2
    /\ StakeOfN1(HonestStayers) * 3 > TotalN1 * 2

\* ── Candidate C2 — bounded TOTAL stake churn — ALSO TLC-REJECTED ──────────
\* "Moved" stake across the boundary = stake of leavers (in N terms) +
\* stake of joiners (in N+1 terms) + sum of |stake delta| over stayers.
\* Require moved stake <= 1/3 of the total in BOTH epochs.
\*
\* TLC VERDICT: STILL INSUFFICIENT. Counterexample (no churn at all,
\* just stake redistribution): setN = setN1 = all 5; stakeN v4=3 others
\* 1 (total 7); stakeN1 v3=2,v4=2 others 1 (total 7); faulty={v0,v1}
\* (stake 2/7, just under 1/3). MovedStake = |v3:1->2| + |v4:3->2| = 2,
\* and 2*3=6 <= 7, so C2's bound HOLDS — yet epoch-N quorum {v0,v1,v4}
\* (stake 5) and epoch-N+1 quorum {v0,v1,v2,v3} (stake 5) intersect only
\* in the faulty pair {v0,v1}.
\*
\* CHARACTERIZATION (why simple bounds fail): a violation exists iff the
\* honest stayers can be split into A,B with StakeOfN(A) < 1/3 TotalN and
\* StakeOfN1(B) < 1/3 TotalN1 — i.e. honest stake that is "cheap in N"
\* can be disjoint from honest stake "cheap in N+1" when stakes
\* redistribute, AND the faulty set (near 1/3) fills the intersection.
\* No simple per-epoch churn/overlap bound rules this out; it needs the
\* two epochs' quorums to be COUPLED. The robust remediation is
\* Tendermint-style: the OLD set signs the EXACT new (set, stakes) with
\* a >2/3 OLD-STAKE quorum, making the boundary atomic so an N+1 quorum
\* provably descends from an N quorum. Owner design decision; this spec
\* maps the failure space (C1 and C2 both rejected) but does not pick
\* the remediation.
AbsDiff(a, b) == IF a >= b THEN a - b ELSE b - a
RECURSIVE SumDelta(_)
SumDelta(S) ==
    IF S = {} THEN 0
    ELSE LET x == CHOOSE x \in S : TRUE
         IN AbsDiff(stakeN[x], stakeN1[x]) + SumDelta(S \ {x})
MovedStake ==
    StakeOfN(setN \ setN1)            \* leavers, N-stake
  + StakeOfN1(setN1 \ setN)           \* joiners, N+1-stake
  + SumDelta(setN \cap setN1)         \* stayer stake deltas
StakeChurnBoundedFix ==
    /\ MovedStake * 3 <= TotalN
    /\ MovedStake * 3 <= TotalN1

\* ── Candidate C3 — delayed stake activation (FREEZE stayer stakes) — ALSO REJECTED ─
\* Stake-updates take effect one epoch LATER, so a continuing validator's
\* stake is identical across the boundary; only count-capped joins/leaves
\* change the set.
\*
\* TLC VERDICT: STILL INSUFFICIENT. Counterexample: setN={v0,v1,v2,v3}
\* all stake 1 (total 4); v4 JOINS in N+1 with stake 3 (stayers frozen,
\* churn count 1 within cap); total N+1 = 7; faulty={v0}. The high-stake
\* JOINER reshapes N+1 quorums: N-quorum {v0,v1,v2} and N+1-quorum
\* {v0,v3,v4} (stake 5) intersect only in faulty v0. Freezing STAYER
\* stakes does nothing about JOINER stake.
StakeFreezeFix == \A v \in (setN \cap setN1) : stakeN1[v] = stakeN[v]

\* ── CONCLUSION: no lightweight per-epoch bound is sufficient ───────────────
\* C1 (stayer supermajority), C2 (bounded total stake churn), and C3
\* (delayed/frozen stayer stakes) are ALL TLC-rejected — each defeated by
\* an independent stake redistribution (stayer-fattening / pure
\* redistribution / high-stake joiner respectively) that a near-1/3
\* Byzantine set exploits. Because the two epochs' stake distributions
\* are INDEPENDENT, no local per-epoch constraint couples their quorums.
\*
\* The robust remediation REQUIRES coupling: the OLD validator set must
\* ratify the EXACT new (set, stakes) with a >2/3 OLD-STAKE quorum
\* (Tendermint-style atomic validator-set change), so every epoch-N+1
\* quorum provably descends from an epoch-N quorum. Modeling/implementing
\* that is a substantial consensus change (validator-set-change signing +
\* verification) and an OWNER DESIGN DECISION — this spec's contribution
\* is the conclusive proof that the cheap alternatives do not work.
\*
\* ── Candidate C4 — frozen HONEST stayers form a supermajority — TLC-VERIFIED ─
\* Combines the lessons of C1/C2/C3: the validators that (a) are in BOTH
\* epochs, (b) are honest, AND (c) have UNCHANGED stake across the
\* boundary must hold > 2/3 stake in each epoch. Because these "stable
\* honest" validators have identical stake in both epochs, they anchor a
\* CONSISTENT supermajority that neither stayer-fattening (C1),
\* redistribution (C2), nor high-stake joiners (C3) can route around.
\*
\* TLC VERDICT: VERIFIED SUFFICIENT. With EnforceStakeOverlap = TRUE and
\* StakeOverlapFix = this condition, HonestQuorumIntersection HOLDS over
\* the full model — 850,288 distinct states, queue drained to 0, 0
\* violations (14 min). This is the first candidate that closes the gap.
\*
\* IMPLEMENTABILITY: this is enforced by making each epoch boundary
\* preserve a >2/3 stake mass of honest validators whose membership AND
\* stake are unchanged across the boundary. Concretely: bound the
\* COMBINED voting-power change (leaves + joins + stake-updates), measured
\* against BOTH the old and new totals, so that the carried-over,
\* stake-unchanged set keeps a supermajority — i.e. cap the total
\* voting-power churn well below 1/3, AND apply stake-updates with a
\* one-epoch activation delay (so "stayers" really are frozen for the
\* boundary quorum). This is the stake-weighted analogue of Tendermint's
\* bounded per-block voting-power change; the count-based cap in
\* validator_set.rs:241-243 must become a STAKE-based cap on this
\* combined quantity. NOTE: the exact numeric cap that GUARANTEES the
\* >2/3 frozen-honest supermajority depends on the max Byzantine stake
\* fraction; deriving it for the production parameters is the remaining
\* impl-design step (owner).
StableHonest == {v \in (setN \cap setN1) : (v \notin faulty) /\ (stakeN[v] = stakeN1[v])}
StableHonestSupermajorityFix ==
    /\ StakeOfN(StableHonest)  * 3 > TotalN  * 2
    /\ StakeOfN1(StableHonest) * 3 > TotalN1 * 2

\* The fix selected by EnforceStakeOverlap. C1/C2/C3 are all TLC-rejected
\* (see above); C4 is TLC-VERIFIED sufficient.
StakeOverlapFix == StableHonestSupermajorityFix

\* ── C5 — the ENFORCEABLE rule the protocol can actually check ───────────────
\* C4 requires honest-stayers>2/3 but honesty is adversary-hidden. C5 is what
\* the EpochTransitionManager can enforce without knowing honesty:
\*   (a) delayed stake activation -> continuing validators' stakes are FROZEN
\*       across a boundary (StakeFreezeFix), and
\*   (b) the STAYERS (all of them) hold > 2/3 stake in both epochs,
\* under a tightened Byzantine bound f < 1/ByzDen (ByzDen >= 4). Arithmetic:
\* stayers > 2/3 of total, minus Byzantine < 1/ByzDen, leaves honest stayers
\* > 2/3 - 1/ByzDen of total; with the churn budget chosen so the implied
\* margin holds, this recovers C4. We test ByzDen=4 (f<1/4) here. Enforcing
\* (b) as a STAKE-churn cap: leaving stake <= c*TotalN and joining <= c*TotalN1
\* with c < 1/3 - f keeps stayers above the 2/3 line.
StakersHoldSupermajority ==
    /\ StakeOfN(setN \cap setN1)  * 3 > TotalN  * 2
    /\ StakeOfN1(setN \cap setN1) * 3 > TotalN1 * 2
C5Fix == StakeFreezeFix /\ StakersHoldSupermajority
\* TLC VERDICT (CrossEpochAgreement_C5.cfg, ByzDen=4 i.e. f<1/4):
\* HonestQuorumIntersection HOLDS — "Model checking completed. No error has
\* been found." over 483,553 distinct states. So C5 is the VERIFIED ENFORCEABLE
\* rule. Implemented in evaporchain-consensus EpochTransitionManager as:
\* (a) one-epoch stake-update activation delay (freezes stayers across a
\*     boundary), and (b) a STAKE-churn cap MAX_STAKE_CHURN_FRACTION on
\* (leaving + joining) stake, with the cap < 1/3 - f. At f<1/4 -> cap < 1/12.

\* ══════════════════════════════════════════════════════════════════════════
\* Initial state — nondeterministically pick two epochs satisfying the
\* impl's constraints, so TLC explores ALL legal reconfigurations.
\* ══════════════════════════════════════════════════════════════════════════

Init ==
    /\ stakeN  \in [AllValidators -> StakeValues]
    /\ stakeN1 \in [AllValidators -> StakeValues]
    /\ setN  \in SUBSET AllValidators
    /\ setN1 \in SUBSET AllValidators
    /\ faulty \in SUBSET AllValidators
    \* Both sets meet the minimum-validators floor.
    /\ Cardinality(setN)  >= MinValidators
    /\ Cardinality(setN1) >= MinValidators
    \* Count-based churn cap (the impl's actual constraint).
    /\ ChurnWithinCap
    \* Byzantine assumption (tightened by ByzDen): faulty stake < 1/ByzDen
    \* of total in BOTH epochs. ByzDen=3 = standard f<1/3; ByzDen=4 = f<1/4.
    /\ StakeOfN(faulty \cap setN)   * ByzDen < TotalN
    /\ StakeOfN1(faulty \cap setN1) * ByzDen < TotalN1
    \* Candidate fix: when enabled, require the stake-overlap remediation.
    /\ (EnforceStakeOverlap => StakeOverlapFix)
    \* Enforceable C5 rule: delayed activation (frozen stayers) + stayers >2/3.
    /\ (EnforceC5 => C5Fix)

\* Static model: no transitions; TLC enumerates all initial states.
Next == UNCHANGED vars
Spec == Init /\ [][Next]_vars

\* ══════════════════════════════════════════════════════════════════════════
\* Invariants
\* ══════════════════════════════════════════════════════════════════════════

TypeOK ==
    /\ setN  \subseteq AllValidators
    /\ setN1 \subseteq AllValidators
    /\ stakeN  \in [AllValidators -> StakeValues]
    /\ stakeN1 \in [AllValidators -> StakeValues]
    /\ faulty \subseteq AllValidators

\* The standing Byzantine assumption, restated as an invariant so the
\* model only ever reasons about legal configurations.
ByzantineBoundBothEpochs ==
    /\ StakeOfN(faulty \cap setN)   * 3 < TotalN
    /\ StakeOfN1(faulty \cap setN1) * 3 < TotalN1

\* THE cross-epoch agreement guarantee: every epoch-N quorum and every
\* epoch-(N+1) quorum share an HONEST validator. If this holds, the two
\* epochs cannot commit conflicting blocks (the shared honest validator
\* would have to vote for both — equivocation). A counterexample means
\* count-bounded churn is insufficient for stake-weighted cross-epoch
\* safety.
HonestQuorumIntersection ==
    \A QN \in SUBSET setN :
        \A QN1 \in SUBSET setN1 :
            (IsQuorumN(QN) /\ IsQuorumN1(QN1)) =>
                \E v \in (QN \cap QN1) : v \notin faulty

SafetyInvariant ==
    /\ TypeOK
    /\ ByzantineBoundBothEpochs
    /\ HonestQuorumIntersection

=============================================================================
