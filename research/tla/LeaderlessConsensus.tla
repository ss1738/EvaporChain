-------------------- MODULE LeaderlessConsensus --------------------
(*
    Leaderless block-production safety/liveness — V1.5 acceptance gate #1
    (docs/proposals/leaderless-block-production-v15.md §4.1).

    V1 picks a single leader per (height, round). V1.5 lets ANY eligible
    validator emit a block (VRF sortition), so a single height can carry
    MULTIPLE competing candidate blocks. This spec checks that the BFT
    voting rule still preserves agreement under that multiplicity.

    Model (one height; the multi-height chain follows by the usual
    induction, as in MccForkChoice.tla):
      - `Blocks` = the competing candidates emitted this height by the
        eligible validators (>= 1; models leaderless multi-proposal).
      - Honest validators run the DETERMINISTIC MCC fork-choice +
        antichain rule and therefore all vote for the SAME canonical
        block (that determinism is what MccForkChoice.tla proves).
      - Byzantine validators (< n/3) vote for an arbitrary candidate or
        abstain — TLC explores every such assignment.
      - A block FINALIZES when a BFT quorum of 2f+1 validators vote it.

    Verified invariants (over ALL Byzantine vote assignments):
      - SafetyNoConflictingFinalize: at most one block per height
        finalizes. Quorum intersection: a non-canonical block can draw
        votes only from the <= f Byzantine, and f < 2f+1, so it can
        never reach the quorum the honest already give the canonical one.
      - LivenessCanonicalFinalizes: the honest-canonical block always
        finalizes — the n-f honest validators alone meet 2f+1 when
        n >= 3f+1.

    Together with MccForkChoice.tla (deterministic fork-choice tie-break)
    this discharges gate #1: multi-proposer emission + MCC fork-choice +
    antichain mempool preserve safety & liveness under f < n/3.
*)

EXTENDS Integers, FiniteSets, TLC

CONSTANTS
    Validators,   \* set of validator ids (naturals)
    Faulty,       \* Byzantine subset
    Blocks        \* candidate block ids emitted this height (>= 1)

ASSUME Faulty \subseteq Validators
ASSUME Cardinality(Blocks) >= 1
\* `0` is the abstain sentinel and must not collide with a real block id.
ASSUME 0 \notin Blocks
\* BFT bound: strictly fewer than a third of validators are Byzantine.
ASSUME 3 * Cardinality(Faulty) < Cardinality(Validators)

Honest == Validators \ Faulty
NoVote == 0
\* BFT quorum = 2f+1 (<= n whenever n >= 3f+1, guaranteed by the ASSUME).
Quorum == 2 * Cardinality(Faulty) + 1

VARIABLES
    canonical,  \* the block honest validators converge on (the MCC tip)
    vote        \* [Validators -> Blocks \cup {NoVote}]

vars == <<canonical, vote>>

\* One-shot voting. TLC enumerates every initial state: each choice of
\* canonical block, each Byzantine vote assignment. Honest validators are
\* pinned to `canonical` (deterministic MCC fork-choice).
Init ==
    /\ canonical \in Blocks
    /\ vote \in [Validators -> (Blocks \cup {NoVote})]
    /\ \A h \in Honest : vote[h] = canonical

Next == UNCHANGED vars

Spec == Init /\ [][Next]_vars

VotersFor(b) == { v \in Validators : vote[v] = b }
Finalized(b) == Cardinality(VotersFor(b)) >= Quorum

TypeOK ==
    /\ canonical \in Blocks
    /\ vote \in [Validators -> (Blocks \cup {NoVote})]

\* SAFETY — no two distinct blocks finalize at the same height.
SafetyNoConflictingFinalize ==
    \A b1, b2 \in Blocks :
        (b1 # b2 /\ Finalized(b1)) => ~Finalized(b2)

\* LIVENESS — the honest-canonical block always reaches a quorum.
LivenessCanonicalFinalizes == Finalized(canonical)

\* A non-canonical block draws votes only from the Byzantine set
\* (honest validators all voted canonical), so it stays below quorum.
NonCanonicalStaysSubQuorum ==
    \A b \in Blocks : b # canonical => ~Finalized(b)

SafetyInvariant ==
    /\ TypeOK
    /\ SafetyNoConflictingFinalize
    /\ LivenessCanonicalFinalizes
    /\ NonCanonicalStaysSubQuorum

=============================================================================
