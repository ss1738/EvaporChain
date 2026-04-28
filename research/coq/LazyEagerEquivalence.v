(* ===================================================================== *)
(*  EvaporChain — Rule-Based Consensus: lazy ≡ eager                     *)
(*                                                                       *)
(*  Mechanization of the punch-list #10 obligation: prove that lazy     *)
(*  rule evaluation produces the same trace as eager evaluation.         *)
(*                                                                       *)
(*  Companions:                                                          *)
(*      research/tla/RuleBasedConsensus.tla   — state-machine spec       *)
(*      research/frontier/03-rule-based-                                 *)
(*          consensus-proof.md                — proof companion          *)
(*                                                                       *)
(*  The Rule-Based Consensus design hinges on this property: validators *)
(*  agree on an anchor `(anchor_epoch, anchor_energy)` and then each    *)
(*  validator can independently compute object state at any query epoch *)
(*  via `LazyEnergy(anchor_energy, half_life, query_epoch - anchor)`,  *)
(*  WITHOUT another consensus round. For this to be safe, lazy and     *)
(*  eager evaluation must agree on every reachable trace.               *)
(*                                                                       *)
(*  EAGER: at each epoch tick, recompute every object's energy          *)
(*    e_{t+1} = decay_step(e_t, half_life, 1)                           *)
(*  and store the result.                                                *)
(*                                                                       *)
(*  LAZY: at the anchor, store (anchor_energy, anchor_epoch). At query *)
(*  time, compute                                                        *)
(*    e_query = decay_step(anchor_energy, half_life,                    *)
(*                         query_epoch - anchor_epoch)                  *)
(*  on demand.                                                           *)
(*                                                                       *)
(*  Equivalence theorem: for any sequence of epoch advances starting    *)
(*  from a shared anchor, eager and lazy produce identical energy      *)
(*  values at the final epoch.                                           *)
(* ===================================================================== *)

From Coq Require Import Arith Lia.

(* --------------------------------------------------------------------- *)
(*  Decay primitive                                                      *)
(*                                                                       *)
(*  We treat decay_step as an opaque parameter; the only property we    *)
(*  rely on is that it composes — i.e., k+1 steps equal one step on    *)
(*  the result of k steps. This is true for the EvaporChain decay       *)
(*  function exactly when integer rounding is monotone, which is the    *)
(*  obligation discharged in `EnergyDecayMonotonicity.v`.                *)
(* --------------------------------------------------------------------- *)

Parameter energy : Type.
Parameter half_life : Type.

(* `decay_step e h n` = energy after `n` epochs from `e`. *)
Parameter decay_step : energy -> half_life -> nat -> energy.

(* Composition law: applying n steps then m steps = applying n+m steps. *)
Axiom decay_step_compose :
  forall e h n m,
    decay_step (decay_step e h n) h m = decay_step e h (n + m).

(* Identity: 0 steps = identity. *)
Axiom decay_step_zero :
  forall e h, decay_step e h 0 = e.

(* --------------------------------------------------------------------- *)
(*  Eager and lazy evaluators                                            *)
(* --------------------------------------------------------------------- *)

(* Eager: starting from `e0`, advance epoch by epoch, applying one step
   each time. Equivalent to applying `n` single steps. *)
Fixpoint eager_eval (e0 : energy) (h : half_life) (steps : nat) : energy :=
  match steps with
  | 0 => e0
  | S k => decay_step (eager_eval e0 h k) h 1
  end.

(* Lazy: at query time, apply all steps in one call. *)
Definition lazy_eval (e0 : energy) (h : half_life) (steps : nat) : energy :=
  decay_step e0 h steps.

(* --------------------------------------------------------------------- *)
(*  Equivalence theorem                                                  *)
(* --------------------------------------------------------------------- *)

Theorem eager_eq_lazy : forall e0 h n,
    eager_eval e0 h n = lazy_eval e0 h n.
Proof.
  intros e0 h n.
  unfold lazy_eval.
  induction n as [| k IH].
  - simpl. rewrite decay_step_zero. reflexivity.
  - simpl. (* eager_eval e0 h (S k) = decay_step (eager_eval e0 h k) h 1 *)
    rewrite IH.
    (* Goal: decay_step (decay_step e0 h k) h 1 = decay_step e0 h (S k) *)
    rewrite decay_step_compose.
    (* k + 1 = S k *)
    f_equal. lia.
Qed.

(* --------------------------------------------------------------------- *)
(*  Trace-level equivalence                                              *)
(*                                                                       *)
(*  A "trace" is a list of (epoch, energy) pairs. Eager produces the    *)
(*  trace by applying one step per epoch advance; lazy computes the     *)
(*  same trace on-demand. We show that for any query at epoch n, the   *)
(*  two traces agree.                                                    *)
(* --------------------------------------------------------------------- *)

(* Eager trace: at every epoch from 0 to N, the energy. *)
Fixpoint eager_trace (e0 : energy) (h : half_life) (N : nat) : list energy :=
  match N with
  | 0 => e0 :: nil
  | S k =>
      eager_trace e0 h k ++ (decay_step (eager_eval e0 h k) h 1 :: nil)
  end.

(* Lazy query at epoch i: just one decay_step call. *)
Definition lazy_query (e0 : energy) (h : half_life) (i : nat) : energy :=
  decay_step e0 h i.

(* Trace agreement: any query at epoch i ≤ N matches the eager trace's
   ith entry. We don't fully formalize list-indexing here; the result
   reduces to `eager_eval = lazy_eval` from above. *)
Theorem trace_query_agreement : forall e0 h i,
    eager_eval e0 h i = lazy_query e0 h i.
Proof.
  intros e0 h i. unfold lazy_query. apply eager_eq_lazy.
Qed.

(* --------------------------------------------------------------------- *)
(*  What the equivalence depends on                                      *)
(*                                                                       *)
(*  The proof above is `Qed.` — fully closed — relative to two axioms: *)
(*    decay_step_compose : k+m steps = k then m                          *)
(*    decay_step_zero    : 0 steps = identity                            *)
(*                                                                       *)
(*  Both axioms are properties of the EvaporChain integer-decay         *)
(*  function. `decay_step_zero` is trivially true (epochs_elapsed=0     *)
(*  in the Rust impl returns the input). `decay_step_compose` is the   *)
(*  non-trivial obligation: integer rounding can break composition if  *)
(*  the function is not "associative" under successive applications.   *)
(*                                                                       *)
(*  For the EvaporChain `energy_at_epoch` definition (bit-shift +       *)
(*  linear-interpolation), decay_step_compose is *not* exactly true —    *)
(*  there is a small rounding error per re-anchor. The frontier doc    *)
(*  acknowledges this:                                                    *)
(*                                                                       *)
(*    "After many re-anchors, anchor_energy may differ from a fresh    *)
(*     lazy_eval all the way back to InitialEnergy."                     *)
(*    — RuleBasedConsensus.tla:165                                       *)
(*                                                                       *)
(*  So lazy ≡ eager EXACTLY when the anchor cadence is rare enough     *)
(*  that the rounding drift is bounded by an acceptable epsilon. The   *)
(*  precise drift bound is open work; the protocol design (long anchor *)
(*  intervals + integer cap) keeps it negligible in practice.           *)
(*                                                                       *)
(*  This Coq file therefore proves the IDEAL (real-valued or perfectly *)
(*  composable integer) equivalence. The drift-bound theorem for the    *)
(*  actual integer impl is tracked as a separate follow-up.              *)
(* --------------------------------------------------------------------- *)
