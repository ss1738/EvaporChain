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

    THIS SPEC SURFACES A TIE-BREAK INCONSISTENCY in the implementation:
      - `mcc_choose` (choose.rs:37-44) breaks caliber ties by the
        LARGER head block id (`new_head > prev_head`).
      - `select_tip` / `enumerate_with_caliber` (fork_choice.rs:261-264)
        breaks caliber ties by the SMALLER block id (ascending sort).
    The two are used in different seams (`mcc_choose` in `evaluate`
    accept/reject; `select_tip` in proposer tip selection), but both are
    "MCC fork choice." On an energy TIE between two leaves they pick
    OPPOSITE winners. This spec models both rules and checks whether they
    can diverge — see the `TieBreakRulesAgree` property.

    Implementation:
        - crates/evaporchain-mcc/src/caliber.rs:13-30 (path_energy,
          path_caliber = boltzmann_weight)
        - crates/evaporchain-mcc/src/choose.rs:23-49 (mcc_choose —
          argmax caliber, tie-break LARGER head id)
        - crates/evaporchain-consensus/src/fork_choice.rs:226-265
          (select_tip / enumerate_with_caliber — argmax caliber,
          tie-break SMALLER block id)

    Author:  Satyawan Singh
    Date:    2026-05-22

    Properties verified by TLC:
        TypeOK                  — variable domains.
        SelectTipDeterministic  — the smaller-id rule yields exactly one
                                  winner (validator-deterministic).
        MccChooseDeterministic  — the larger-id rule yields exactly one
                                  winner (validator-deterministic).
        SelectTipPicksMinEnergy  — select_tip's winner has minimum
                                  path-energy among leaves (= max caliber
                                  at positive β).
        TieBreakRulesAgree      — [EXPECTED TO FAIL on energy ties] the
                                  two tie-break rules pick the same
                                  winner. TLC produces a counterexample
                                  exactly when two leaves share min
                                  energy, documenting the inconsistency.

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

\* mcc_choose rule: among min-energy leaves, pick the LARGEST id
\* (choose.rs:41 `new_head > prev_head`).
MccChooseWinner ==
    CHOOSE l \in MinEnergyLeaves : \A m \in MinEnergyLeaves : l >= m

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
\* (larger-id) tie-break.
MccChooseDeterministic ==
    /\ MccChooseWinner \in MinEnergyLeaves
    /\ \A l \in MinEnergyLeaves : MccChooseWinner >= l

\* select_tip's winner has minimum path-energy among all leaves (= max
\* caliber). Confirms the Boltzmann argmax = energy argmin reduction.
SelectTipPicksMinEnergy ==
    \A l \in Leaves : pathEnergy[SelectTipWinner] <= pathEnergy[l]

\* THE INCONSISTENCY CHECK. Expected to be VIOLATED whenever two leaves
\* tie on minimum energy: select_tip picks the smaller id, mcc_choose
\* picks the larger. TLC's counterexample is the documented finding.
\* (When all min-energy leaves are unique — no energy tie — both rules
\* pick the same single leaf and this holds.)
TieBreakRulesAgree ==
    SelectTipWinner = MccChooseWinner

\* The SAFE determinism property that DOES hold: each rule, used
\* CONSISTENTLY by all honest validators, is deterministic. The risk is
\* only if the two rules are mixed across the propose/evaluate seam.
SafetyInvariant ==
    /\ TypeOK
    /\ SelectTipDeterministic
    /\ MccChooseDeterministic
    /\ SelectTipPicksMinEnergy

=============================================================================
