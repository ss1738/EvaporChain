(** * LLSA Invariant Preservation — EvaporChain Conservation Gate

    Per research/INVENTION_STACK.md §A1.2 T4 (Lambda-Locked Self-Amendment,
    Tier-0 theorem-grade):

        "Protocol upgrades require a Coq/Lean term of type
         forall s, Inv(s) -> Inv(step_new(s)).
         Pinned MetaCoq kernel + extraction-to-Rust."

    This file provides the Coq statement, the state model grounded in
    crates/evaporchain-energy-kernel/src/ (compartment.rs, conservation.rs,
    redirect.rs, lambda.rs), and proof obligations.

    All proof obligations discharged 2026-04-29. Zero [Admitted] remaining.
    Helper lemmas [zero_div_any] and [energy_at_epoch_zero_elapsed] close
    both base-case goals in §6.

    Companion files:
        research/coq/EnergyDecayMonotonicity.v   -- base decay lemma (done)
        research/tla/ConservationInvariant.tla    -- TLC-checked operational model
        research/proofs/conservation_proof_notes.md -- plain-English rationale

    Author:  Satyawan Singh
    Date:    2026-04-29
*)

Require Import Coq.Arith.Arith.
(* Coq 8.12 removed Coq.omega.Omega; the modern decision procedure is
   `lia` from `Coq.micromega.Lia`. Layer 2 of the doctrine punch list
   migrated this file from `omega` to `lia` so it can build against
   the project's pinned Coq 8.18 toolchain (research/coq/Makefile).

   2026-05-05: Coq 9.0 (Rocq) removed `Coq.Arith.Div2` entirely. The
   import was unused in this file (`pow2` is defined locally at the
   top of section 2), so the import is dropped rather than migrated. *)
Require Import Lia.
Require Import Coq.Init.Nat.

(* ================================================================
   1. Types
   ================================================================ *)

Definition Energy := nat.

Record ChainState : Type := mkState
  { accounts     : Energy
  ; stake        : Energy
  ; refresh_pool : Energy
  ; slashed_pool : Energy
  ; epoch        : nat
  }.

Definition HalfLife := nat.

(* ================================================================
   2. The Energy Decay Function
   ================================================================ *)

Fixpoint pow2 (n : nat) : nat :=
  match n with
  | O    => 1
  | S n' => 2 * pow2 n'
  end.

Definition energy_at_epoch (e : Energy) (half_life : HalfLife) (elapsed : nat) : Energy :=
  e / pow2 (elapsed / half_life).

(* ================================================================
   3. Total Energy
   ================================================================ *)

Definition TotalEnergy (s : ChainState) : Energy :=
  accounts s + stake s + refresh_pool s + slashed_pool s.

(* ================================================================
   4. The Conservation Invariant
   ================================================================ *)

Record InvParams : Type := mkParams
  { lambda_hl      : HalfLife
  ; genesis_total  : Energy
  ; prior_total    : Energy
  ; epochs_elapsed : nat
  }.

(** Inv(s, p):
    1. Total bounded above by genesis_total (no creation ever).
    2. Total non-negative (trivially true in Nat).
    3. Total does not exceed prior_total (monotone non-increasing).
    4. Total >= decay floor (no over-destruction beyond lambda). *)
Definition Inv (s : ChainState) (p : InvParams) : Prop :=
  TotalEnergy s <= genesis_total p
  /\ 0 <= TotalEnergy s
  /\ TotalEnergy s <= prior_total p
  /\ TotalEnergy s >= energy_at_epoch
                        (prior_total p)
                        (lambda_hl p)
                        (epochs_elapsed p).

(* ================================================================
   5. Transition Relation
   ================================================================ *)

Inductive RedirectStep (s s' : ChainState) : Prop :=
  | redirect_intro :
      TotalEnergy s' = TotalEnergy s ->
      accounts s' + stake s' + refresh_pool s' + slashed_pool s' =
        TotalEnergy s ->
      epoch s' = epoch s ->
      RedirectStep s s'.

Inductive DecayStep (s s' : ChainState) (p : InvParams) : Prop :=
  | decay_intro :
      TotalEnergy s' <= TotalEnergy s ->
      TotalEnergy s' >= energy_at_epoch
                          (TotalEnergy s)
                          (lambda_hl p)
                          (epochs_elapsed p) ->
      epoch s' = epoch s + epochs_elapsed p ->
      DecayStep s s' p.

Definition BlockProduceStep (s s' : ChainState) (p : InvParams) : Prop :=
  DecayStep s s' p.

(* ================================================================
   5b. Helper Lemmas
   ================================================================ *)

(** 0 / n = 0 for all n (including n = 0, where Nat.div_0_l requires n > 0). *)
Lemma zero_div_any : forall (n : nat), 0 / n = 0.
Proof.
  intro n. destruct n as [|n'].
  - (* n = 0: 0 / 0 = 0 by Nat.divmod definition *)
    reflexivity.
  - (* n = S n' > 0: use Nat.div_0_l *)
    apply Nat.div_0_l. lia.
Qed.

(** energy_at_epoch(e, hl, 0) = e — elapsed = 0 means no decay has occurred. *)
Lemma energy_at_epoch_zero_elapsed :
  forall (e : Energy) (hl : HalfLife),
    energy_at_epoch e hl 0 = e.
Proof.
  intros e hl.
  unfold energy_at_epoch.
  rewrite zero_div_any.  (* 0 / hl = 0 *)
  simpl pow2.            (* pow2 0 = 1 *)
  apply Nat.div_1_r.     (* e / 1 = e *)
Qed.

(* ================================================================
   6. LLSA Gate: forall s, Inv(s, p) -> Inv(step_new(s), p')
   ================================================================ *)

Lemma redirect_preserves_inv :
  forall (s s' : ChainState) (p : InvParams),
    Inv s p ->
    RedirectStep s s' ->
    let p' := mkParams
                (lambda_hl p)
                (genesis_total p)
                (TotalEnergy s')
                0
    in
    Inv s' p'.
Proof.
  intros s s' p Hinv Hstep p'.
  destruct Hinv as [Hbound [Hnn [Hmono Hfloor]]].
  destruct Hstep as [Htot_eq Hparts Hepoch_eq].
  unfold Inv. simpl.
  split. { rewrite Htot_eq. exact Hbound. }
  split. { apply Nat.le_0_l. }
  split. { apply Nat.le_refl. }
  rewrite energy_at_epoch_zero_elapsed. apply Nat.le_refl.
Qed.

Lemma decay_preserves_inv :
  forall (s s' : ChainState) (p : InvParams),
    Inv s p ->
    DecayStep s s' p ->
    let p' := mkParams
                (lambda_hl p)
                (genesis_total p)
                (TotalEnergy s')
                0
    in
    Inv s' p'.
Proof.
  intros s s' p Hinv Hstep p'.
  destruct Hinv as [Hbound [Hnn [Hmono Hfloor]]].
  destruct Hstep as [Hdecay_le Hdecay_ge Hepoch].
  unfold Inv. simpl.
  split.
  { apply Nat.le_trans with (TotalEnergy s).
    - exact Hdecay_le.
    - exact Hbound. }
  split. { apply Nat.le_0_l. }
  split. { apply Nat.le_refl. }
  rewrite energy_at_epoch_zero_elapsed. apply Nat.le_refl.
Qed.

Lemma block_produce_preserves_inv :
  forall (s s' : ChainState) (p : InvParams),
    Inv s p ->
    BlockProduceStep s s' p ->
    let p' := mkParams
                (lambda_hl p)
                (genesis_total p)
                (TotalEnergy s')
                0
    in
    Inv s' p'.
Proof.
  intros s s' p Hinv Hstep.
  unfold BlockProduceStep in Hstep.
  eapply decay_preserves_inv; eassumption.
Qed.

(** Main LLSA gate theorem.

    This is the type that crates/evaporchain-llsa/src/lib.rs requires of
    every proof artefact supplied to apply_amendment.

    target_invariant_id = blake3("evaporchain-conservation-invariant-v1")
    The hash of this proof's bytes is stored in LlsaProof::coq_term_hash. *)
Theorem llsa_conservation_invariant_preservation :
  forall (s s' : ChainState) (p : InvParams),
    Inv s p ->
    (  RedirectStep s s'
    \/ DecayStep s s' p
    \/ BlockProduceStep s s' p) ->
    let p' := mkParams
                (lambda_hl p)
                (genesis_total p)
                (TotalEnergy s')
                0
    in
    Inv s' p'.
Proof.
  intros s s' p Hinv Hstep p'.
  destruct Hstep as [Hr | [Hd | Hbp]].
  - eapply redirect_preserves_inv; eassumption.
  - eapply decay_preserves_inv; eassumption.
  - eapply block_produce_preserves_inv; eassumption.
Qed.

(* ================================================================
   6b. Generic Step Abstraction (parametrized LLSA gate)
   ================================================================ *)

(** The proofs in §6 above discharge invariant preservation for the
    *current* concrete step relations [RedirectStep] and [DecayStep].
    Doctrine §A1.2 T4 demands the gate be parametrized over an
    *arbitrary* `step_new` supplied by a future amendment.

    The reduction here is: every step relation that preserves [Inv]
    under the {prior_total := TotalEnergy s'; epochs_elapsed := 0}
    successor-parameters convention reduces to a single proof
    obligation — total energy is non-increasing across the step. The
    decay-floor branch of [Inv] becomes vacuous in the successor
    parameters because [energy_at_epoch_zero_elapsed] forces the floor
    to coincide with [TotalEnergy s'] itself.

    Concretely, a future amendment shipping a new step relation
    [step_new : ChainState -> ChainState -> InvParams -> Prop] need
    only:

      1. Define [step_new] as an inductive relation or predicate.
      2. Discharge [StepMonotone step_new].
      3. Apply [llsa_amendment_gate] to obtain [Inv s' p'] for free.

    The [RedirectStep] / [DecayStep] / [BlockProduceStep] relations
    are recovered as concrete instances (corollaries below). *)

(** [StepMonotone R] — the single proof obligation each amendment
    must discharge. Total energy across the step is non-increasing.
    This is the precise mechanised form of "no energy creation" that
    the §1.2 conservation gate enforces operationally. *)
Definition StepMonotone (R : ChainState -> ChainState -> InvParams -> Prop) : Prop :=
  forall s s' p, R s s' p -> TotalEnergy s' <= TotalEnergy s.

(** Generic preservation lemma: any [R] satisfying [StepMonotone]
    preserves [Inv] under the canonical successor parameters
    {prior_total := TotalEnergy s'; epochs_elapsed := 0}.

    Note: the canonical-successor convention is the SAME convention
    already used by [redirect_preserves_inv] and [decay_preserves_inv]
    in §6 — the [p'] binding in those lemmas. Future amendments that
    want a different successor convention need a separate gate. *)
Lemma step_new_preserves_inv :
  forall (R : ChainState -> ChainState -> InvParams -> Prop)
         (s s' : ChainState) (p : InvParams),
    StepMonotone R ->
    R s s' p ->
    Inv s p ->
    let p' := mkParams (lambda_hl p) (genesis_total p) (TotalEnergy s') 0 in
    Inv s' p'.
Proof.
  intros R s s' p Hmono Hstep Hinv p'.
  destruct Hinv as [Hbound [_ [_ _]]].
  pose proof (Hmono s s' p Hstep) as Hle.
  unfold Inv. simpl.
  split.
  { apply Nat.le_trans with (TotalEnergy s); [exact Hle | exact Hbound]. }
  split. { apply Nat.le_0_l. }
  split. { apply Nat.le_refl. }
  rewrite energy_at_epoch_zero_elapsed. apply Nat.le_refl.
Qed.

(** [llsa_amendment_gate] — the polymorphic gate type that
    crates/evaporchain-llsa/src/lib.rs's [apply_amendment] must
    check against any future amendment's proof artefact.

    target_invariant_id = blake3("evaporchain-conservation-invariant-v1-generic")
    bound_amendment_hash = Amendment::hash() of the proposed amendment

    A new amendment supplies (R, proof_of_StepMonotone_R, proof_of_R_holds)
    and obtains [Inv s' p'] without re-proving the conservation gate. *)
Theorem llsa_amendment_gate :
  forall (R : ChainState -> ChainState -> InvParams -> Prop)
         (s s' : ChainState) (p : InvParams),
    StepMonotone R ->
    R s s' p ->
    Inv s p ->
    let p' := mkParams (lambda_hl p) (genesis_total p) (TotalEnergy s') 0 in
    Inv s' p'.
Proof.
  exact step_new_preserves_inv.
Qed.

(* ----------------------------------------------------------------
   §6b corollaries: existing concrete steps are special cases.

   These prove the existing [RedirectStep] / [DecayStep] are valid
   amendments under the generic gate — i.e. they each discharge
   [StepMonotone] for their lifted-to-InvParams form. The §6
   bespoke proofs ([redirect_preserves_inv], [decay_preserves_inv])
   remain the canonical reference; these corollaries demonstrate
   the generic gate subsumes them.
   ---------------------------------------------------------------- *)

(** Lift [RedirectStep] (which doesn't depend on [InvParams]) into
    the [R s s' p] schema by ignoring the parameter argument. *)
Definition RedirectStepP (s s' : ChainState) (_ : InvParams) : Prop :=
  RedirectStep s s'.

Definition DecayStepP (s s' : ChainState) (p : InvParams) : Prop :=
  DecayStep s s' p.

Lemma redirect_step_monotone : StepMonotone RedirectStepP.
Proof.
  intros s s' p Hstep.
  destruct Hstep as [Heq _ _].
  rewrite Heq. apply Nat.le_refl.
Qed.

Lemma decay_step_monotone : StepMonotone DecayStepP.
Proof.
  intros s s' p Hstep.
  destruct Hstep as [Hle _ _].
  exact Hle.
Qed.

(** Witness: the existing concrete steps are valid amendments. *)
Corollary redirect_preserves_inv_via_gate :
  forall (s s' : ChainState) (p : InvParams),
    Inv s p ->
    RedirectStep s s' ->
    let p' := mkParams (lambda_hl p) (genesis_total p) (TotalEnergy s') 0 in
    Inv s' p'.
Proof.
  intros s s' p Hinv Hstep.
  exact (llsa_amendment_gate RedirectStepP s s' p
           redirect_step_monotone Hstep Hinv).
Qed.

Corollary decay_preserves_inv_via_gate :
  forall (s s' : ChainState) (p : InvParams),
    Inv s p ->
    DecayStep s s' p ->
    let p' := mkParams (lambda_hl p) (genesis_total p) (TotalEnergy s') 0 in
    Inv s' p'.
Proof.
  intros s s' p Hinv Hstep.
  exact (llsa_amendment_gate DecayStepP s s' p
           decay_step_monotone Hstep Hinv).
Qed.

(* ================================================================
   7. Proof Obligations — CLOSED
   ================================================================ *)

(**
   [ADMIT-1] and [ADMIT-2] were both discharged 2026-04-29 by:

       zero_div_any : forall n, 0 / n = 0
         Proof: destruct n; [reflexivity | Nat.div_0_l + lia]

       energy_at_epoch_zero_elapsed : forall e hl, energy_at_epoch e hl 0 = e
         Proof: unfold, rewrite zero_div_any, simpl pow2, Nat.div_1_r

   Both admit sites closed by:
       rewrite energy_at_epoch_zero_elapsed. apply Nat.le_refl.

   The main theorem [llsa_conservation_invariant_preservation] is now
   fully mechanised — zero remaining [Admitted] obligations.

   2026-05-08: §6b parametrizes the gate over an arbitrary `step_new`.
   [llsa_amendment_gate] is the polymorphic form — concrete steps
   (Redirect / Decay) are recovered as corollaries via lifted
   predicates [RedirectStepP] / [DecayStepP] and trivially-discharged
   [StepMonotone] obligations. The doctrine §A1.2 T4 demand
   ("forall s, Inv(s) -> Inv(step_new(s))") is now mechanised in its
   parametric form. Future amendments need only supply [step_new]
   plus [StepMonotone step_new] — no edit to this file required.
*)

(* ================================================================
   8. Extraction Note
   ================================================================ *)

(**
   Per crates/evaporchain-llsa/src/lib.rs (INVENTION_STACK.md §A1.2 T4):

   The hash of the proof bytes for [llsa_conservation_invariant_preservation]
   is stored in LlsaProof::coq_term_hash.

   target_invariant_id: blake3("evaporchain-conservation-invariant-v1")

   Any governance Amendment touching the energy kernel must supply an
   LlsaProof whose:
     - target_invariant_id matches the above hash
     - bound_amendment_hash matches Amendment::hash() of the proposed amendment
     - proof_bytes is accepted by the pinned MetaCoq kernel

   NOTE on Löb's theorem: the Coq kernel is an external TCB. This proof does
   NOT claim the chain has escaped Gödel. The MetaCoq kernel is trusted
   separately; document this honestly in the whitepaper (INVENTION_STACK.md §A1.9 rule 13).
*)
