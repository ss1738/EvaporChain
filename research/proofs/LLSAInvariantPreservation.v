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
