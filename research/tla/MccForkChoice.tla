--------------------- MODULE MccForkChoice ---------------------
(*
    TLA+ formal specification of EvaporChain's MCC (Maximum-Caliber
    Consensus) Boltzmann fork-choice determinism — the D3-MCC follow-up
    sub-piece from TLA_IMPL_DRIFT_AUDIT_2026_05_21.md § D3.

    When `parent_acceptance_mode = "mcc"`, the proposer/validator picks
    among competing DAG tips by Boltzmann caliber: each candidate tip's
    first-parent trajectory is scored by `path_caliber = Boltzmann(path_
    energy, β)`, and the maximum-caliber tip wins. Because Boltzmann is
    monotone-DECREASING in energy at positive β, "max caliber" = "min
    path-energy" — the closest-to-equilibrium / least-dissipation fork.

    The SAFETY-CRITICAL property is DETERMINISM: every honest validator,
    given the same DAG and the same β, must pick the SAME tip — otherwise
    they fork. Determinism reduces to the tie-break being a TOTAL order.

    HISTORY — this spec was the #460 diagnostic that surfaced a tie-break
    INCONSISTENCY: `mcc_choose` broke caliber ties by the LARGER head id
    while `select_tip` used the SMALLER, so the propose/accept seams could
    pick OPPOSITE winners on a tie (a liveness hazard). #461 (8855d434 +
    the energy-first follow-up) fixed the impl: both seams now use the
    SAME total order — caliber, then LOWER path-energy (the #461 fix for
    caliber saturation, where distinct energies quantise to equal
    caliber), then SMALLER head id. This spec is updated to match, so
    `TieBreakRulesAgree` now HOLDS and is part of `SafetyInvariant`.

    Implementation:
        - crates/evaporchain-mcc/src/caliber.rs:13-30 (path_energy,
          path_caliber = boltzmann_weight)
        - crates/evaporchain-mcc/src/choose.rs (mcc_choose — argmax
          caliber, then lower path-energy, then smaller head id)
        - crates/evaporchain-consensus/src/fork_choice.rs
          (select_tip / enumerate_with_caliber — same total order)

    Author:  Satyawan Singh
    Date:    2026-05-22

    Properties verified by TLC:
        TypeOK                  — variable domains.
        SelectTipDeterministic  — the smaller-id rule yields exactly one
                                  winner (validator-deterministic).
        MccChooseDeterministic  — the smaller-id rule yields exactly one
                                  winner (validator-deterministic).
        SelectTipPicksMinEnergy  — select_tip's winner has minimum
                                  path-energy among leaves (= max caliber
                                  at positive β).
        TieBreakRulesAgree      — [HOLDS since #461] both seams pick the
                                  same winner on every energy assignment,
                                  including ties (both use smaller-id).

    Modeling choices / out of scope:
        - Boltzmann arithmetic is abstracted: at positive β, argmax-
          caliber = argmin-path-energy (monotone), so we compare path
          energies directly. The β=0 all-tie degenerate case is covered
          by allowing equal energies.
        - First-parent trajectory construction is abstracted to a given
          per-leaf path-energy (the trajectory walk itself is a
          deterministic DAG fold, verified structurally elsewhere).
        - Multi-parent DAG trajectory enumeration (Lane I.4) is future
          work; V1 is first-parent-only (deterministic single path).

    See `research/tla/TLA_IMPL_DRIFT_AUDIT_2026_05_21.md` § D3 (MCC follow-up).
*)

EXTENDS Integers, FiniteSets, TLC

\* ══════════════════════════════════════════════════════════════════════════
\* Constants
\* ══════════════════════════════════════════════════════════════════════════

CONSTANTS
    Leaves,         \* Finite set of leaf block ids (competing DAG tips).
    MaxEnergy       \* Path-energies range over 0..MaxEnergy.

ASSUME Cardinality(Leaves) >= 1
ASSUME MaxEnergy >= 0

\* Total order on leaf ids. TLA+ has no built-in order over arbitrary
\* model values, so we require the cfg to use naturals as leaf ids and
\* compare them directly. (The impl uses 32-byte block ids with the
\* natural lexicographic order; modeling them as naturals preserves the
\* "total order exists" property that determinism depends on.)
ASSUME Leaves \subseteq Nat

\* ══════════════════════════════════════════════════════════════════════════
\* Variables
\* ══════════════════════════════════════════════════════════════════════════

\* The only state is the per-leaf path-energy assignment. We let it be
\* chosen nondeterministically at Init (within 0..MaxEnergy) so TLC
\* explores ALL energy configurations — including ties.
VARIABLES
    pathEnergy      \* [Leaves -> 0..MaxEnergy] : first-parent trajectory energy.

vars == <<pathEnergy>>

\* ══════════════════════════════════════════════════════════════════════════
\* Selection rules
\* ══════════════════════════════════════════════════════════════════════════

\* Min path-energy among leaves (= max Boltzmann caliber at positive β).
MinEnergy == CHOOSE e \in {pathEnergy[l] : l \in Leaves} :
                \A l \in Leaves : e <= pathEnergy[l]

\* The leaves achieving the minimum energy (the caliber-tie set).
MinEnergyLeaves == {l \in Leaves : pathEnergy[l] = MinEnergy}

\* select_tip rule: among min-energy leaves, pick the SMALLEST id
\* (fork_choice.rs:263 ascending sort).
SelectTipWinner ==
    CHOOSE l \in MinEnergyLeaves : \A m \in MinEnergyLeaves : l <= m

\* mcc_choose rule (post-#461): among min-energy leaves, pick the
\* SMALLEST id — same as select_tip (choose.rs `new_head < prev_head`,
\* applied after the energy-first compare). The energy-first step is
\* already captured by restricting the tie-break to MinEnergyLeaves.
MccChooseWinner ==
    CHOOSE l \in MinEnergyLeaves : \A m \in MinEnergyLeaves : l <= m

\* ══════════════════════════════════════════════════════════════════════════
\* Init / Next — pathEnergy is fixed per behavior; no transitions needed.
\* TLC enumerates all energy assignments as distinct initial states.
\* ══════════════════════════════════════════════════════════════════════════

Init == pathEnergy \in [Leaves -> 0..MaxEnergy]

\* Stuttering-only: the model is a pure function of the (nondeterministic)
\* initial energy assignment. We check invariants over all initial states.
Next == UNCHANGED vars

Spec == Init /\ [][Next]_vars

\* ══════════════════════════════════════════════════════════════════════════
\* Invariants
\* ══════════════════════════════════════════════════════════════════════════

TypeOK == pathEnergy \in [Leaves -> 0..MaxEnergy]

\* select_tip yields exactly one winner — determinism via the total
\* (smaller-id) tie-break. (CHOOSE is deterministic; we assert the
\* winner is genuinely minimal-and-unique among the min-energy set.)
SelectTipDeterministic ==
    /\ SelectTipWinner \in MinEnergyLeaves
    /\ \A l \in MinEnergyLeaves : SelectTipWinner <= l

\* mcc_choose yields exactly one winner — determinism via the total
\* (smaller-id) tie-break (post-#461, matching select_tip).
MccChooseDeterministic ==
    /\ MccChooseWinner \in MinEnergyLeaves
    /\ \A l \in MinEnergyLeaves : MccChooseWinner <= l

\* select_tip's winner has minimum path-energy among all leaves (= max
\* caliber). Confirms the Boltzmann argmax = energy argmin reduction.
SelectTipPicksMinEnergy ==
    \A l \in Leaves : pathEnergy[SelectTipWinner] <= pathEnergy[l]

\* CONSISTENCY CHECK. Post-#461 both seams use the SAME total order
\* (caliber, then lower path-energy, then smaller id), so the two rules
\* pick the same winner on EVERY energy assignment — including ties.
\* This now HOLDS (it was the #460 diagnostic that surfaced the bug)
\* and is checked as part of SafetyInvariant below.
TieBreakRulesAgree ==
    SelectTipWinner = MccChooseWinner

\* The determinism + consistency properties that hold post-#461: each
\* rule is deterministic AND the two seams agree (so propose/evaluate
\* can't fork on a tie).
SafetyInvariant ==
    /\ TypeOK
    /\ SelectTipDeterministic
    /\ MccChooseDeterministic
    /\ SelectTipPicksMinEnergy
    /\ TieBreakRulesAgree

=============================================================================
