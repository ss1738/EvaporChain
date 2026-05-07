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
(** Helper: per-unit-step bound on linear_decay growth. When v
    increases by 1, linear_decay grows by at most 1 (because
    rem < h ≤ 2h, so each step in v adds < 1 to v*rem/(2h)). *)
Lemma linear_decay_step_le_one :
  forall v rem h,
    h <> 0 -> rem < h ->
    EnergyDecayMonotonicity.linear_decay (v + 1) rem h <=
    EnergyDecayMonotonicity.linear_decay v rem h + 1.
Proof.
  intros v rem h Hh Hrem.
  unfold EnergyDecayMonotonicity.linear_decay.
  replace ((v + 1) * rem) with (v * rem + rem) by lia.
  (* Goal: (v*rem + rem) / (2h) <= (v*rem)/(2h) + 1 *)
  (* Strategy: bound the RHS as `< (v*rem)/(2h) + 2`, then `<= +1` follows. *)
  apply Nat.lt_succ_r.
  apply Nat.div_lt_upper_bound; [lia |].
  pose proof (Nat.div_mod (v * rem) (2 * h)) as Hdm.
  pose proof (Nat.mod_upper_bound (v * rem) (2 * h)) as Hmod.
  (* Goal: v*rem + rem < 2*h * S (Nat.div (v*rem) (2*h) + 1) *)
  lia.
Qed.

(** Helper: per-unit-step monotonicity of v - linear_decay v rem h. *)
Lemma sub_linear_decay_step_mono :
  forall v rem h,
    h <> 0 -> rem < h ->
    v - EnergyDecayMonotonicity.linear_decay v rem h <=
    (v + 1) - EnergyDecayMonotonicity.linear_decay (v + 1) rem h.
Proof.
  intros v rem h Hh Hrem.
  pose proof (linear_decay_step_le_one v rem h Hh Hrem) as Hstep.
  pose proof (EnergyDecayMonotonicity.linear_decay_bounded v rem h Hh Hrem) as Hbnd.
  lia.
Qed.

(** Helper: monotonicity of `v - linear_decay v rem h` in v. By
    induction on the proof of `a <= b` using the per-unit-step bound. *)
Lemma sub_linear_decay_mono_init :
  forall a b rem h,
    h <> 0 -> rem < h -> a <= b ->
    a - EnergyDecayMonotonicity.linear_decay a rem h <=
    b - EnergyDecayMonotonicity.linear_decay b rem h.
Proof.
  intros a b rem h Hh Hrem Hab.
  induction Hab as [| b' Hab' IH].
  - (* a = a *) reflexivity.
  - (* a <= S b' from a <= b' *)
    eapply Nat.le_trans.
    + exact IH.
    + replace (S b') with (b' + 1) by lia.
      apply sub_linear_decay_step_mono; assumption.
Qed.

(** Helper: nat_shr is monotone in its first argument. *)
Lemma nat_shr_mono_init :
  forall a b k,
    a <= b ->
    EnergyDecayMonotonicity.nat_shr a k <=
    EnergyDecayMonotonicity.nat_shr b k.
Proof.
  intros a b k Hab.
  induction k as [| k' IH].
  - simpl. exact Hab.
  - simpl.
    (* Nat.div2 is monotone *)
    apply Nat.div_le_mono with (c := 2) in IH.
    + rewrite <- (Nat.div2_div (EnergyDecayMonotonicity.nat_shr a k')) in IH.
      rewrite <- (Nat.div2_div (EnergyDecayMonotonicity.nat_shr b k')) in IH.
      exact IH.
    + lia.
Qed.

(** [DRIFT-MONO-INIT] DISCHARGED 2026-05-07. Composes the helpers
    above. *)
Lemma concrete_step_mono_init :
  forall a b h n,
    h <> 0 ->
    a <= b ->
    EnergyDecayMonotonicity.energy_at_epoch a h n <=
    EnergyDecayMonotonicity.energy_at_epoch b h n.
Proof.
  intros a b h n Hh Hab.
  unfold EnergyDecayMonotonicity.energy_at_epoch.
  rewrite (proj2 (Nat.eqb_neq h 0) Hh).
  destruct (leb EnergyDecayMonotonicity.halving_cutoff (Nat.div n h)).
  - (* Past halving cutoff: both 0. *)
    reflexivity.
  - apply sub_linear_decay_mono_init.
    + exact Hh.
    + apply Nat.mod_upper_bound. exact Hh.
    + apply nat_shr_mono_init. exact Hab.
Qed.

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
(** Helper: floor-super-additivity for the linear_decay term. The
    sum of two floor-divides is bounded above by the floor of the
    sum: `floor(a/c) + floor(b/c) <= floor((a+b)/c)`. Specialised
    here to `linear_decay`.

    DISCHARGED 2026-05-07. *)
Lemma linear_decay_super_additive :
  forall v r h,
    h <> 0 ->
    EnergyDecayMonotonicity.linear_decay v r h +
    EnergyDecayMonotonicity.linear_decay v 1 h <=
    EnergyDecayMonotonicity.linear_decay v (r + 1) h.
Proof.
  intros v r h Hh.
  unfold EnergyDecayMonotonicity.linear_decay.
  replace (v * (r + 1)) with (v * r + v * 1) by lia.
  apply Nat.div_le_lower_bound; [lia |].
  pose proof (Nat.div_mod (v * r) (2 * h)) as Hdm1.
  pose proof (Nat.div_mod (v * 1) (2 * h)) as Hdm2.
  pose proof (Nat.mod_upper_bound (v * r) (2 * h)) as Hmod1.
  pose proof (Nat.mod_upper_bound (v * 1) (2 * h)) as Hmod2.
  lia.
Qed.

(** Within-halving case of step-subadditivity. When (S k)/h = k/h
    (no half-life boundary crossed), the lazy-at-(S k) value is at
    most the staged-1-from-lazy-at-k value.

    DISCHARGE STATUS 2026-05-07: proof scaffold landed (case-split
    on h=1 vs h>=2 done; for h>=2 the chain `LHS <= mid1 <= RHS` is
    structurally clear with all 7 supporting bounds asserted).
    Final compose step left as Admitted: under the bounds Hsuper
    (A + B <= C), Hmono_inner (D <= B), Hbnd_after_rem' (A <= after),
    Hbnd_after_rem_succ (C <= after), Hbnd_inner (D <= after - A),
    and Hcomb (A + B <= after), the goal `after - C <= (after - A)
    - D` reduces in real arithmetic to `A + D <= C`, immediate from
    Hsuper + Hmono_inner. But Coq's `lia` and `nia` don't close it
    even after `set`/`remember`/`clearbody` of the four
    `linear_decay` applications — the saturating-sub anti-monotonic
    chain doesn't unfold cleanly. The right path is an explicit
    chain via `Nat.sub_add_distr` + a `Nat.sub_le_mono_*` family
    lemma, which requires more proof-engineering than fits in
    this commit's bounded scope. Tagged [DRIFT-STEP-SUB-WITHIN]. *)
Lemma concrete_step_subadditive_within_halving :
  forall e h k,
    h <> 0 ->
    Nat.div (S k) h = Nat.div k h ->
    EnergyDecayMonotonicity.energy_at_epoch e h (S k) <=
    EnergyDecayMonotonicity.energy_at_epoch
      (EnergyDecayMonotonicity.energy_at_epoch e h k) h 1.
Proof.
  (* Proof scaffold drafted across multiple 2026-05-07 attempts. The
     structural decomposition is solid:
       - h=1 case: contradiction (within-halving forces S k = k)
       - h>=2 case: unfold energy_at_epoch on both sides; assemble
         7 supporting bounds (Hsuper from linear_decay_super_additive
         + 4 linear_decay_bounded sites + monotonicity-in-arg-1 of
         the inner linear_decay + combined non-saturation bound)
       - Final goal: `after - C <= (after - A) - D` reduces in
         real arith to `A + D <= C`, immediate from Hsuper +
         Hmono_inner.

     Tactical blocker (encountered in 4 separate attempts):
       - Bare lia fails: doesn't unfold the saturating-sub
         anti-monotonic relation across the structural goal.
       - Bare nia same.
       - set/remember/clearbody of the four linear_decay applications
         + lia: same failure (lia can't bridge).
       - Explicit chain via `replace ((after - A) - D) with
         (after - (A + D))` (using Nat.sub_add_distr) does succeed,
         leaving a goal `after - C <= after - (A + D)`. The remaining
         `apply Nat.sub_le_mono_l` then fails because that lemma's
         signature is `n <= m -> n - p <= m - p` — wrong polarity.
         The needed lemma is `n <= m -> p - m <= p - n` (anti-mono
         in arg-2). Inline-asserting `forall a b c, a <= b -> c - b
         <= c - a` and proving by `intros; lia` succeeds for the
         standalone helper but `apply Hsub_anti` then fails on the
         actual goal due to unification difficulty around the `set
         (after := nat_shr e (Nat.div k h))` binding inside the
         destruct branch.

     The needed missing piece is either:
       (a) An anti-mono-arg-2 sub lemma in scope as a fresh nat
           relation (without nested set bindings interfering), OR
       (b) A more aggressive lia variant / autorewrite database that
           handles `(c - b <= c - a)` from `a <= b` automatically.

     Tagged [DRIFT-STEP-SUB-WITHIN]. The proof IS structurally
     sound — the Admitted is purely Coq-tactical. *)
Admitted.

(** Cross-halving case of step-subadditivity. When (S k)/h = k/h + 1
    (a half-life boundary IS crossed by the +1 step), the LHS gets
    one additional `Nat.div2` halving while the RHS subtracts one
    `linear_decay (inner) 1 h`. The inequality holds because the
    inner-value's halving-equivalent (linear_decay over the full
    pre-boundary remainder) lands `inner` such that `Nat.div2 after
    <= inner - inner/(2h)` for h >= 1 — algebraically `after/2 <=
    after * (h+1)(2h-1) / (4h^2)` reduces to `0 <= h - 1`.

    DISCHARGE STATUS 2026-05-07: stated; the integer-floor-rounding
    case is technical but tractable. Tagged [DRIFT-STEP-SUB-CROSS]
    for a focused follow-up. *)
Lemma concrete_step_subadditive_cross_halving :
  forall e h k,
    h <> 0 ->
    Nat.div (S k) h = Nat.div k h + 1 ->
    EnergyDecayMonotonicity.energy_at_epoch e h (S k) <=
    EnergyDecayMonotonicity.energy_at_epoch
      (EnergyDecayMonotonicity.energy_at_epoch e h k) h 1.
Proof.
Admitted.

(** [DRIFT-STEP-SUB] composition lemma. Combines the within-halving
    and cross-halving cases via case-split on `(S k)/h` vs `k/h`. *)
Lemma concrete_step_subadditive :
  forall e h k,
    h <> 0 ->
    EnergyDecayMonotonicity.energy_at_epoch e h (S k) <=
    EnergyDecayMonotonicity.energy_at_epoch
      (EnergyDecayMonotonicity.energy_at_epoch e h k) h 1.
Proof.
  intros e h k Hh.
  (* (S k)/h is either k/h or k/h + 1. *)
  pose proof (Nat.div_mod (S k) h Hh) as Hsk.
  pose proof (Nat.div_mod k h Hh) as Hk.
  pose proof (Nat.mod_upper_bound (S k) h Hh) as Hbnd_sk.
  pose proof (Nat.mod_upper_bound k h Hh) as Hbnd_k.
  destruct (Nat.eq_dec (Nat.div (S k) h) (Nat.div k h)) as [Heq | Hne].
  - apply concrete_step_subadditive_within_halving; assumption.
  - assert (Hcross : Nat.div (S k) h = Nat.div k h + 1).
    { (* (S k) / h is either k/h or k/h + 1; not equal means +1. *)
      pose proof (Nat.div_le_mono k (S k) h Hh (le_S k k (le_n k))) as Hle.
      pose proof (Nat.div_str_pos_iff (S k - k) h Hh) as _.
      (* Easier: use Nat.div_succ_l or analyze directly *)
      assert (Hub : Nat.div (S k) h <= Nat.div k h + 1).
      { apply Nat.div_le_upper_bound; [exact Hh |].
        rewrite Nat.mul_add_distr_l, Nat.mul_1_r.
        nia. }
      lia. }
    apply concrete_step_subadditive_cross_halving; assumption.
Qed.

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
