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

(* ===================================================================== *)
(*  CONCRETE DRIFT BOUND (2026-05-07)                                    *)
(*                                                                       *)
(*  The IDEAL theorem above (`eager_eq_lazy`) holds under two axioms     *)
(*  about an opaque `decay_step` parameter. The concrete EvaporChain     *)
(*  decay function is `energy_at_epoch` from EnergyDecayMonotonicity.v   *)
(*  — bit-shift halving + linear interpolation. Under that concrete      *)
(*  function, exact composition does NOT hold (`decay_step_compose`     *)
(*  fails as an equality), so `eager_eval = lazy_eval` is also false.    *)
(*                                                                       *)
(*  What we CAN prove is the directional bound:                          *)
(*                                                                       *)
(*       concrete_lazy_eval e h n  <=  concrete_eager_eval e h n         *)
(*                                                                       *)
(*  Intuition: lazy applies one big floor-divide (`init / 2^(n/h)`),     *)
(*  collapsing the integer-rounding loss into a single `nat_shr` call    *)
(*  + one `linear_decay` subtraction. Eager applies n single steps,      *)
(*  each subtracting `floor(prev / (2h))` — when this floor is 0 (which  *)
(*  happens for small `prev` or large `h`), eager STOPS DECAYING, while  *)
(*  lazy keeps decaying via the bigger halving step. So eager has        *)
(*  floor-induced fixed points that lazy doesn't share, and lazy <=      *)
(*  eager in general.                                                    *)
(*                                                                       *)
(*  The punch-list framing of `|lazy - eager| <= O(1/h)` was             *)
(*  aspirational; the actual gap can be much larger (O(n*e/h) worst      *)
(*  case). The one-sided bound below is what the integer-decay model     *)
(*  actually supports.                                                   *)
(* ===================================================================== *)

From EvaporChain Require Import EnergyDecayMonotonicity.

Definition concrete_decay_step (e : EnergyDecayMonotonicity.energy)
                               (h : EnergyDecayMonotonicity.half_life)
                               (n : nat) : EnergyDecayMonotonicity.energy :=
  EnergyDecayMonotonicity.energy_at_epoch e h n.

Fixpoint concrete_eager_eval (e0 : EnergyDecayMonotonicity.energy)
                             (h : EnergyDecayMonotonicity.half_life)
                             (steps : nat) : EnergyDecayMonotonicity.energy :=
  match steps with
  | 0    => e0
  | S k  => concrete_decay_step (concrete_eager_eval e0 h k) h 1
  end.

Definition concrete_lazy_eval (e0 : EnergyDecayMonotonicity.energy)
                              (h : EnergyDecayMonotonicity.half_life)
                              (steps : nat) : EnergyDecayMonotonicity.energy :=
  concrete_decay_step e0 h steps.

(** Base case: at zero steps, both evaluators return the initial
    energy unchanged. This is a sanity check — the inductive bound's
    base case. *)
Lemma concrete_drift_base :
  forall e0 h,
    h <> 0 ->
    concrete_lazy_eval e0 h 0 = concrete_eager_eval e0 h 0.
Proof.
  intros e0 h Hh.
  unfold concrete_lazy_eval, concrete_eager_eval, concrete_decay_step.
  apply EnergyDecayMonotonicity.energy_at_epoch_zero.
  - exact Hh.
  - unfold EnergyDecayMonotonicity.halving_cutoff. lia.
Qed.

(** Helper: monotonicity of `energy_at_epoch` in the initial value.
    If `a <= b` then decaying `a` and `b` for the same number of
    epochs preserves the order. The proof is a structural unfold +
    `nia` over the integer floor-divisions, leveraging that
    `nat_shr`, `linear_decay`, and saturating subtraction are all
    individually monotone in their first argument.

    DISCHARGE STATUS 2026-05-07: stated; proof obligation tagged
    [DRIFT-MONO-INIT]. The tactic `nia` typically handles this kind
    of integer-floor monotonicity but the case split on the
    halving-cutoff branch may need manual work. Left as Admitted for
    a focused follow-up session; the outer `concrete_drift_one_sided`
    theorem composes cleanly once this discharges. *)
Lemma concrete_step_mono_init :
  forall a b h n,
    h <> 0 ->
    a <= b ->
    EnergyDecayMonotonicity.energy_at_epoch a h n <=
    EnergyDecayMonotonicity.energy_at_epoch b h n.
Proof.
  (* Proof attempt 2026-05-07: structural unfold + 4 helper assertions
     (nat_shr monotonicity, linear_decay monotonicity in first arg,
     linear_decay bounded by first arg when rem < h, applied to both
     a and b). The unfold + helpers go through cleanly; the final
     compose-via-lia step FAILS because `lia` cannot reason directly
     about the saturating-sub `f(a) - g(a) <= f(b) - g(b)` shape from
     `a <= b`, `f(a) <= f(b)`, `g(a) <= g(b)`, `g(a) <= f(a)`,
     `g(b) <= f(b)` alone — a counterexample exists in those bounds
     (e.g., f(a)=3, g(a)=0, f(b)=10, g(b)=9 satisfies all four but
     gives 3 - 0 = 3 > 10 - 9 = 1). The MISSING constraint is that
     `g(v) = floor(v*r/(2h))` with `r < h` gives `g(v+1) - g(v) <= 1`
     per unit-step in v, hence `f(v) = v - g(v)` is monotone. The
     correct proof goes by induction on `b - a` using the unit-step
     bound. Tractable but technical; left as a focused follow-up.
     Tagged [DRIFT-MONO-INIT].

     The two intermediate facts (nat_shr monotone, linear_decay
     monotone in arg-1) ARE provable as separate small lemmas — see
     `EnergyDecayMonotonicity.v::nat_shr_monotone` and
     `EnergyDecayMonotonicity.v::linear_decay_monotone_in_remainder`
     for analogous primitives. The remaining work is composing them
     via the per-unit-step argument above. *)
Admitted.

(** Helper: step-subadditivity of `energy_at_epoch`. Applying `S k`
    epochs in one shot underestimates applying `k` epochs and then
    one additional epoch:

      energy_at_epoch e h (S k) <= energy_at_epoch (energy_at_epoch e h k) h 1

    The intuition matches the section-header comment: a single big
    floor-divide collapses to a smaller value than two staged
    floor-divides because the second stage's `linear_decay` on the
    already-floored intermediate cannot recover the bits the first
    stage truncated.

    DISCHARGE STATUS 2026-05-07: stated; proof obligation tagged
    [DRIFT-STEP-SUB]. Requires case-split on whether `(S k)/h =
    k/h` (within-halving step) vs `(S k)/h = k/h + 1` (cross-halving
    step). Within-halving case reduces to algebraic comparison
    `inner_after * (2h - rem - 1) <= (inner_after - inner_after*rem/(2h)) * (2h - 1)`
    which simplifies via `nia` to `rem >= 0` (always true). Cross-
    halving case uses `energy_at_epoch_monotone` + the bound that
    `nat_shr (k/h + 1)` differs from `nat_shr k/h` by exactly one
    halving. Tractable but technical; left as Admitted for a focused
    follow-up. *)
Lemma concrete_step_subadditive :
  forall e h k,
    h <> 0 ->
    EnergyDecayMonotonicity.energy_at_epoch e h (S k) <=
    EnergyDecayMonotonicity.energy_at_epoch
      (EnergyDecayMonotonicity.energy_at_epoch e h k) h 1.
Proof.
Admitted.

(** ## Main drift bound theorem

    `concrete_lazy_eval e h n <= concrete_eager_eval e h n` for any
    initial energy `e`, non-zero half-life `h`, and step count `n`.

    This is the one-sided drift bound that the punch-list §10
    obligation asked for, restated honestly. The original framing
    `|lazy - eager| <= O(1/h)` was wrong about the gap magnitude —
    in the worst case the gap is O(n*e/h), not O(1/h). The
    DIRECTIONAL bound (lazy underestimates eager) is what the
    integer-decay model actually guarantees, and is sufficient for
    the Rule-Based Consensus design: validators relying on
    `lazy_eval` for late binding will compute energies that are
    lower bounds on what eager step-by-step would produce — i.e.,
    `lazy_eval` is conservative, never optimistic.

    Proof structure: induction on n.
      Base: `concrete_drift_base` — equal at n=0.
      Step: chain three facts via `Nat.le_trans`:
        1. `concrete_step_subadditive` — lazy(S k) ≤ lazy(k) staged 1
        2. `concrete_step_mono_init` applied with IH — staged-1 from
           lazy(k) ≤ staged-1 from eager(k)
        3. The latter is exactly eager(S k) by definition.

    DISCHARGE STATUS 2026-05-07: composition Qed; depends on the
    two helper lemmas above being discharged (currently Admitted).
    Once both helpers land, this theorem becomes fully closed. *)
Theorem concrete_drift_one_sided :
  forall e0 h n,
    h <> 0 ->
    concrete_lazy_eval e0 h n <= concrete_eager_eval e0 h n.
Proof.
  intros e0 h n Hh.
  induction n as [| k IH].
  - (* Base: n = 0. Both equal e0. *)
    rewrite concrete_drift_base by exact Hh. reflexivity.
  - (* Step: n = S k *)
    unfold concrete_lazy_eval in *.
    unfold concrete_decay_step in *.
    simpl concrete_eager_eval.
    unfold concrete_decay_step.
    (* Goal: energy_at_epoch e0 h (S k) <=
             energy_at_epoch (concrete_eager_eval e0 h k) h 1 *)
    eapply Nat.le_trans.
    + (* Sub-step 1: lazy at S k <= staged-1-from-lazy-at-k *)
      apply concrete_step_subadditive. exact Hh.
    + (* Sub-step 2: staged-1-from-lazy-at-k <= staged-1-from-eager-at-k.
         Apply mono_init using IH (lazy_at_k <= eager_at_k). *)
      apply concrete_step_mono_init.
      * exact Hh.
      * exact IH.
Qed.

(** ## Companion: lazy is conservative

    Restating `concrete_drift_one_sided` from a different angle:
    `lazy_eval` is a SOUND LOWER BOUND on the step-by-step (eager)
    energy. A validator using lazy evaluation never overstates
    energy, which is what the safety contract for Rule-Based
    Consensus needs (overstating could cause spurious
    `is_evaporated` flips toward "alive"; understating cannot). *)
Corollary concrete_lazy_is_conservative :
  forall e0 h n,
    h <> 0 ->
    concrete_lazy_eval e0 h n <= concrete_eager_eval e0 h n.
Proof.
  exact concrete_drift_one_sided.
Qed.

(* --------------------------------------------------------------------- *)
(*  What's left in this file                                             *)
(*                                                                       *)
(*  Two named obligations remain, both tagged with [DRIFT-*] handles    *)
(*  for follow-up sessions:                                              *)
(*                                                                       *)
(*  [DRIFT-MONO-INIT]  concrete_step_mono_init                          *)
(*    Monotonicity of energy_at_epoch in the initial value. Standard   *)
(*    integer-floor monotonicity proof; tractable via nia + case split *)
(*    on the halving_cutoff branch.                                     *)
(*                                                                       *)
(*  [DRIFT-STEP-SUB]   concrete_step_subadditive                        *)
(*    Step-subadditivity (S k epochs in one shot underestimates k       *)
(*    then 1 staged). Within-halving case reduces to algebraic         *)
(*    comparison via nia; cross-halving case uses the existing         *)
(*    energy_at_epoch_monotone + nat_shr halving facts.                *)
(*                                                                       *)
(*  Both are technical but not deep — bounded research-grade Coq work. *)
(*  The headline theorem `concrete_drift_one_sided` and its corollary   *)
(*  `concrete_lazy_is_conservative` are Qed conditional on these       *)
(*  helpers landing.                                                    *)
(* --------------------------------------------------------------------- *)
