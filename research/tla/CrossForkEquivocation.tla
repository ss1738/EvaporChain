--------------------- MODULE CrossForkEquivocation ---------------------
(*
    TLA+ formal specification of EvaporChain's cross-fork (DAG-mode)
    equivocation detector — closes drift D10 from
    TLA_IMPL_DRIFT_AUDIT_2026_05_21.md.

    When `light_cone_state_branches_enabled = "true"`, consensus runs
    over a DAG of incomparable tips, each with its own per-round state.
    A validator that precommits *different* blocks under two
    incomparable tips at the SAME round is cross-fork-equivocating and
    must be detected (the operator slashing tooling reads the counter).

    This spec models the DETECTION COMPLETENESS property: every
    cross-fork double-vote is caught. It deliberately does NOT model
    the full antichain/DAG consensus protocol (that is D3's
    `AntichainConsensus.tla`, a larger effort) — only the equivocation
    detector that rides on top of whatever tips exist.

    Implementation:
        - crates/evaporchain-consensus/src/tendermint.rs:871
          (`cross_fork_equivocations: HashMap<u64, u64>` — validator → count)
        - crates/evaporchain-consensus/src/tendermint.rs:2521-2573
          (`record_dag_precommit` — scans other tips at the same round,
          increments on disagreement)
        - crates/evaporchain-consensus/src/tendermint.rs:3047-3097
          (`all_cross_fork_equivocations` accessor)
        - crates/evaporchain-consensus/src/tendermint.rs:5656-5784
          (DAG-mode routing into per-tip `dag_round_states`)

    Governance gate: light_cone_state_branches_enabled (default false →
    chain stays in the single-chain regime modeled by EvaporChainBFT.tla;
    this spec covers the regime once the flag flips true).

    Author:  Satyawan Singh
    Date:    2026-05-22

    Safety properties verified by TLC:
        TypeOK                           — variable domains.
        CrossForkEquivocationDetected    — if a validator precommitted
                                           two DIFFERENT blocks under two
                                           tips at the same round, it is
                                           in crossForkEquivocations.
                                           (Detection completeness — the
                                           slashing-soundness property.)
        NoFalsePositive                  — a validator that voted at most
                                           one distinct block per round
                                           across all tips is NEVER in
                                           crossForkEquivocations.
        DetectionStable                  — once detected, a validator
                                           stays detected (monotone; the
                                           Rust counter only increments).

    Open / out of TLC scope (same axiom boundary as EvaporChainBFT.tla):
        - The full antichain/DAG consensus protocol (tip creation,
          fork-choice, MCC Boltzmann weighting) — D3's spec.
        - BLS signature verification of the precommit (D11 axiom).
        - The numeric slash AMOUNT computed from the counter (operational).

    See `research/tla/TLA_IMPL_DRIFT_AUDIT_2026_05_21.md` § D10.
*)

EXTENDS Integers, FiniteSets, TLC

\* ══════════════════════════════════════════════════════════════════════════
\* Constants
\* ══════════════════════════════════════════════════════════════════════════

CONSTANTS
    Validators,         \* Validator set.
    Tips,               \* Set of incomparable DAG tips (model-bounded).
    Blocks,             \* Set of block hashes a validator may vote for.
    MaxRound            \* Bound on rounds explored.

ASSUME Cardinality(Validators) >= 1
ASSUME Cardinality(Tips) >= 2          \* cross-fork needs >= 2 tips
ASSUME Cardinality(Blocks) >= 2        \* equivocation needs >= 2 blocks
ASSUME MaxRound >= 0

\* "Nil" marks a tip/round where a validator hasn't precommitted.
NoVote == "Nil"
VoteValues == Blocks \cup {NoVote}

\* ══════════════════════════════════════════════════════════════════════════
\* Variables
\* ══════════════════════════════════════════════════════════════════════════

VARIABLES
    dagPrecommits,          \* [Tips -> [Validators -> [0..MaxRound -> VoteValues]]]
                            \* dagPrecommits[t][v][r] = block v precommitted
                            \* under tip t at round r ("Nil" if none).
    crossForkEquivocations  \* Subset of Validators : detected cross-fork
                            \* equivocators. Mirrors the Rust HashMap's
                            \* keyset (we abstract the count to membership;
                            \* DetectionStable captures the monotone count).

vars == <<dagPrecommits, crossForkEquivocations>>

\* ══════════════════════════════════════════════════════════════════════════
\* Helpers
\* ══════════════════════════════════════════════════════════════════════════

\* Does validator v have a recorded (non-Nil) precommit under tip t at round r?
HasPrecommit(t, v, r) == dagPrecommits[t][v][r] # NoVote

\* Has validator v cross-fork-equivocated at round r? — i.e. two tips at
\* the same round where v's non-Nil precommits disagree. This is the
\* GROUND TRUTH predicate (what SHOULD be detected); the detector
\* (RecordDagPrecommit) must keep crossForkEquivocations in sync with it.
ActuallyEquivocatedAt(v, r) ==
    \E t1, t2 \in Tips :
        /\ t1 # t2
        /\ HasPrecommit(t1, v, r)
        /\ HasPrecommit(t2, v, r)
        /\ dagPrecommits[t1][v][r] # dagPrecommits[t2][v][r]

ActuallyEquivocated(v) ==
    \E r \in 0..MaxRound : ActuallyEquivocatedAt(v, r)

\* ══════════════════════════════════════════════════════════════════════════
\* Initial state
\* ══════════════════════════════════════════════════════════════════════════

Init ==
    /\ dagPrecommits = [t \in Tips |->
                          [v \in Validators |->
                            [r \in 0..MaxRound |-> NoVote]]]
    /\ crossForkEquivocations = {}

\* ══════════════════════════════════════════════════════════════════════════
\* Actions
\* ══════════════════════════════════════════════════════════════════════════

\* RecordDagPrecommit: validator v precommits `block` under tip t at
\* round r. Mirrors Rust's record_dag_precommit: before storing, scan
\* all OTHER tips at the same round; if v's precommit there disagrees,
\* flag the cross-fork equivocation.
RecordDagPrecommit(t, v, r, block) ==
    /\ t \in Tips
    /\ v \in Validators
    /\ r \in 0..MaxRound
    /\ block \in Blocks
    /\ ~HasPrecommit(t, v, r)            \* one precommit per (tip, v, round)
    \* Detection: does any other tip at this round disagree?
    /\ LET equivocates ==
            \E ot \in Tips :
                /\ ot # t
                /\ HasPrecommit(ot, v, r)
                /\ dagPrecommits[ot][v][r] # block
       IN
       /\ dagPrecommits' = [dagPrecommits EXCEPT ![t][v][r] = block]
       /\ crossForkEquivocations' =
            IF equivocates
            THEN crossForkEquivocations \cup {v}
            ELSE crossForkEquivocations

\* ══════════════════════════════════════════════════════════════════════════
\* Next-state relation
\* ══════════════════════════════════════════════════════════════════════════

Next ==
    \E t \in Tips, v \in Validators, r \in 0..MaxRound, block \in Blocks :
        RecordDagPrecommit(t, v, r, block)

Spec == Init /\ [][Next]_vars

\* ══════════════════════════════════════════════════════════════════════════
\* Type invariant
\* ══════════════════════════════════════════════════════════════════════════

TypeOK ==
    /\ dagPrecommits \in [Tips -> [Validators -> [0..MaxRound -> VoteValues]]]
    /\ crossForkEquivocations \subseteq Validators

\* ══════════════════════════════════════════════════════════════════════════
\* Safety invariants
\* ══════════════════════════════════════════════════════════════════════════

\* SAFETY 1 (detection completeness): every validator that actually
\* cross-fork-equivocated is detected. This is the slashing-soundness
\* property — no double-voter escapes the counter.
CrossForkEquivocationDetected ==
    \A v \in Validators :
        ActuallyEquivocated(v) => v \in crossForkEquivocations

\* SAFETY 2 (no false positive): a validator that never voted two
\* different blocks at the same round across tips is never flagged.
\* Honest validators (consistent per round) are never slashed.
NoFalsePositive ==
    \A v \in Validators :
        v \in crossForkEquivocations => ActuallyEquivocated(v)

\* Combined safety
SafetyInvariant ==
    /\ TypeOK
    /\ CrossForkEquivocationDetected
    /\ NoFalsePositive

\* ══════════════════════════════════════════════════════════════════════════
\* Monotonicity (the Rust counter only increments — detection is sticky)
\* ══════════════════════════════════════════════════════════════════════════

\* Once a validator is detected, it stays detected. Expressed as an
\* action property checked by TLC via the [Next] step.
DetectionStable ==
    [][\A v \in Validators :
        v \in crossForkEquivocations => v \in crossForkEquivocations']_vars

=============================================================================
