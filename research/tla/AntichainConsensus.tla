------------------- MODULE AntichainConsensus -------------------
(*
    TLA+ formal specification of EvaporChain's antichain-mempool DAG
    proposal construction — closes drift D3 from
    TLA_IMPL_DRIFT_AUDIT_2026_05_21.md.

    When `block_source_mode = "antichain"` (DAG mode), a proposer builds
    its block proposal by:
        1. collecting the DAG frontier (tips),
        2. seeding an empty antichain,
        3. greedily extending to a MAXIMAL antichain, walking candidates
           in descending-energy order (`extend_to_maximal`),
        4. gating the result on a total-energy threshold.

    The greedy extension is ORDER-DEPENDENT (different candidate orders
    can yield different maximal antichains). Cross-validator agreement
    therefore depends on every honest proposer using the SAME canonical
    candidate order. This spec verifies exactly that convergence
    property, plus the structural correctness of the result.

    SCOPE: models the antichain SELECTION (the safety-critical
    convergence surface) over a fixed DAG. It does NOT model DAG growth,
    the energy threshold gate's numeric arithmetic, or the 16-entry
    digest-history ring buffer (those are operational / covered by
    EnergyVerkleTrie + CrooksMEV-style separate concerns).

    Implementation:
        - crates/evaporchain-antichain-mempool/src/antichain.rs
          (Antichain, comparable/concurrent invariant)
        - crates/evaporchain-antichain-mempool/src/maximal.rs:25-54
          (is_maximal_antichain, extend_to_maximal — greedy, order-dependent)
        - crates/evaporchain-consensus/src/antichain_integration.rs:34-73
          (dag_tips, build_proposal_antichain — tips sorted descending energy)
        - crates/evaporchain-consensus/src/tendermint.rs:858-888, 2453-2497
          (antichain_digest_history, try_finalize_antichain)

    Author:  Satyawan Singh
    Date:    2026-05-22

    Safety properties verified by TLC:
        TypeOK                       — variable domains.
        BuiltIsAntichain             — each validator's built set is a
                                       valid antichain (pairwise concurrent)
                                       at every step.
        CanonicalConverges           — two honest validators using the
                                       canonical order reach the SAME
                                       antichain when both finish (the
                                       cross-validator agreement property).
        CompletedIsMaximal           — a finished canonical build is a
                                       maximal antichain (no block can be
                                       added).

    Demonstrated (non-invariant, by construction): the greedy result is
    order-dependent — an arbitrary-order validator (vArb) may finish with
    a DIFFERENT maximal antichain than the canonical validators, which is
    WHY the canonical order is required. CompletedIsMaximal holds for vArb
    too (every order yields *a* maximal antichain; only the canonical one
    guarantees agreement).

    Out of TLC scope:
        - DAG growth / tip discovery over time.
        - Numeric energy-threshold arithmetic (antichain_energy_gate).
        - Digest-history ring buffer + divergence detection (runtime
          cross-check, not a state-machine safety property).
        - BLS signatures over the digest (D11 axiom).

    See `research/tla/TLA_IMPL_DRIFT_AUDIT_2026_05_21.md` § D3.
*)

EXTENDS Integers, FiniteSets, Sequences, TLC

\* ══════════════════════════════════════════════════════════════════════════
\* Constants — a fixed DAG
\* ══════════════════════════════════════════════════════════════════════════

CONSTANTS
    Blocks,         \* Finite set of block ids (the DAG nodes).
    Ancestor,       \* Ancestor[a] = set of strict ancestors of a (transitive).
    Energy          \* Energy[b] = block b's energy (for canonical ordering).

\* ── Model fixture (supplied to the CONSTANTS via `<-` in the .cfg) ──────────
\* .cfg files cannot parse `:>`/`@@` function literals as direct constant
\* values, so the concrete DAG is defined here as operators and bound via
\* operator substitution. DAG: two independent 2-chains
\*   b0 -> b1   (b1 parent b0)        b2 -> b3   (b3 parent b2)
\* energies b0=50 b1=40 b2=30 b3=20 (distinct ⇒ canonical order total).
McBlocks   == {"b0", "b1", "b2", "b3"}
McAncestor == [b \in McBlocks |->
                 CASE b = "b0" -> {}
                   [] b = "b1" -> {"b0"}
                   [] b = "b2" -> {}
                   [] b = "b3" -> {"b2"}]
McEnergy   == [b \in McBlocks |->
                 CASE b = "b0" -> 50
                   [] b = "b1" -> 40
                   [] b = "b2" -> 30
                   [] b = "b3" -> 20]

\* The DAG must be a strict partial order: irreflexive + transitively
\* closed + acyclic. We assume the provided Ancestor relation satisfies
\* this (it's derived from real block parent links in the impl).
ASSUME \A b \in Blocks : b \notin Ancestor[b]                 \* irreflexive
ASSUME \A a \in Blocks : Ancestor[a] \subseteq Blocks
\* Energy is a total function to naturals; canonical order breaks ties by
\* block id, so we require energies be assigned (ids are inherently unique).
ASSUME \A b \in Blocks : Energy[b] \in Nat

\* ══════════════════════════════════════════════════════════════════════════
\* DAG order helpers
\* ══════════════════════════════════════════════════════════════════════════

\* a and b are comparable iff one is an ancestor of the other.
Comparable(a, b) == (a \in Ancestor[b]) \/ (b \in Ancestor[a])

\* Concurrent = incomparable + distinct.
Concurrent(a, b) == (a # b) /\ ~Comparable(a, b)

\* S is an antichain iff all distinct members are pairwise concurrent.
IsAntichain(S) == \A a, b \in S : (a # b) => Concurrent(a, b)

\* Can block c be added to antichain S? (concurrent with every member,
\* and not already in S.)
CanAdd(S, c) == (c \notin S) /\ (\A m \in S : Concurrent(m, c))

\* S is a MAXIMAL antichain iff it's an antichain and no block extends it.
\* Mirrors is_maximal_antichain (maximal.rs:25).
IsMaximal(S) == IsAntichain(S) /\ (\A c \in Blocks : ~CanAdd(S, c))

\* ══════════════════════════════════════════════════════════════════════════
\* Canonical candidate order: descending energy.
\*
\* The impl sorts tips by descending energy (antichain_integration.rs:44),
\* breaking ties by block id (ids are unique). TLA+ has no built-in total
\* order over arbitrary model values, so we model the canonical order as
\* pure energy-descending and require the cfg to assign DISTINCT energies
\* (the ASSUME below), which makes the order total — exactly the effect
\* the id-tiebreak achieves in the impl. b is processed before a iff b has
\* strictly higher energy.
\* ══════════════════════════════════════════════════════════════════════════

CanonicalBefore(b, a) == Energy[b] > Energy[a]

\* Distinct energies ⇒ the canonical (energy-descending) order is total,
\* modeling the impl's energy-desc-then-id-tiebreak total order.
ASSUME \A a, b \in Blocks : (a # b) => (Energy[a] # Energy[b])

\* ══════════════════════════════════════════════════════════════════════════
\* Variables — two canonical validators (vA, vB) + one arbitrary (vArb)
\* ══════════════════════════════════════════════════════════════════════════

VARIABLES
    builtA,         \* antichain built so far by canonical validator A
    doneA,          \* set of blocks A has already considered
    builtB,         \* canonical validator B (independent run, same order)
    doneB,
    builtArb,       \* arbitrary-order validator (demonstrates order-dependence)
    doneArb

vars == <<builtA, doneA, builtB, doneB, builtArb, doneArb>>

\* ══════════════════════════════════════════════════════════════════════════
\* Initial state — all empty
\* ══════════════════════════════════════════════════════════════════════════

Init ==
    /\ builtA = {} /\ doneA = {}
    /\ builtB = {} /\ doneB = {}
    /\ builtArb = {} /\ doneArb = {}

\* ══════════════════════════════════════════════════════════════════════════
\* Greedy step under the canonical order
\* ══════════════════════════════════════════════════════════════════════════

\* The next canonical candidate for a validator with progress `done` is
\* the not-yet-considered block of highest canonical priority — i.e. the
\* block c such that every other unconsidered block has lower priority.
IsNextCanonical(c, done) ==
    /\ c \in Blocks
    /\ c \notin done
    /\ \A other \in Blocks :
          (other \notin done /\ other # c) => CanonicalBefore(c, other)

\* A canonical validator processes its next candidate: mark it done, and
\* add it to the built antichain iff it's concurrent with all members.
\* Mirrors extend_to_maximal's per-candidate body (maximal.rs:42-52).
StepCanonical(built, done, builtVar, doneVar) ==
    \E c \in Blocks :
        /\ IsNextCanonical(c, done)
        /\ doneVar' = done \cup {c}
        /\ builtVar' = IF \A m \in built : Concurrent(m, c)
                       THEN built \cup {c}
                       ELSE built

StepA ==
    /\ StepCanonical(builtA, doneA, builtA, doneA)
    /\ UNCHANGED <<builtB, doneB, builtArb, doneArb>>

StepB ==
    /\ StepCanonical(builtB, doneB, builtB, doneB)
    /\ UNCHANGED <<builtA, doneA, builtArb, doneArb>>

\* The arbitrary validator processes ANY not-yet-considered block (models
\* a non-canonical proposer — demonstrates order-dependence of the result).
StepArb ==
    /\ \E c \in Blocks :
          /\ c \notin doneArb
          /\ doneArb' = doneArb \cup {c}
          /\ builtArb' = IF \A m \in builtArb : Concurrent(m, c)
                         THEN builtArb \cup {c}
                         ELSE builtArb
    /\ UNCHANGED <<builtA, doneA, builtB, doneB>>

Next ==
    \/ StepA
    \/ StepB
    \/ StepArb

Spec == Init /\ [][Next]_vars /\ WF_vars(StepA) /\ WF_vars(StepB) /\ WF_vars(StepArb)

\* ══════════════════════════════════════════════════════════════════════════
\* Type invariant
\* ══════════════════════════════════════════════════════════════════════════

TypeOK ==
    /\ builtA \subseteq Blocks /\ doneA \subseteq Blocks
    /\ builtB \subseteq Blocks /\ doneB \subseteq Blocks
    /\ builtArb \subseteq Blocks /\ doneArb \subseteq Blocks
    /\ builtA \subseteq doneA
    /\ builtB \subseteq doneB
    /\ builtArb \subseteq doneArb

\* ══════════════════════════════════════════════════════════════════════════
\* Safety invariants
\* ══════════════════════════════════════════════════════════════════════════

\* SAFETY 1: every validator's partial build is always a valid antichain
\* (the greedy never admits a comparable pair). Mirrors the Antichain
\* type invariant in antichain.rs.
BuiltIsAntichain ==
    /\ IsAntichain(builtA)
    /\ IsAntichain(builtB)
    /\ IsAntichain(builtArb)

\* SAFETY 2 (cross-validator agreement): the two canonical validators
\* are always in lockstep — same blocks considered ⇒ same antichain.
\* This is the core D3 property: deterministic canonical order ⇒ honest
\* proposers converge on the same antichain digest.
CanonicalAgree ==
    (doneA = doneB) => (builtA = builtB)

\* SAFETY 3: a COMPLETED canonical build (all blocks considered) is a
\* maximal antichain. Mirrors the postcondition of extend_to_maximal +
\* is_maximal_antichain.
CompletedIsMaximal ==
    (doneA = Blocks) => IsMaximal(builtA)

\* SAFETY 4: completed arbitrary-order build is ALSO maximal — every
\* order yields *a* maximal antichain (only the canonical one guarantees
\* cross-validator agreement). Confirms maximality is order-independent
\* even though the SPECIFIC antichain is not.
ArbCompletedIsMaximal ==
    (doneArb = Blocks) => IsMaximal(builtArb)

SafetyInvariant ==
    /\ TypeOK
    /\ BuiltIsAntichain
    /\ CanonicalAgree
    /\ CompletedIsMaximal
    /\ ArbCompletedIsMaximal

=============================================================================
