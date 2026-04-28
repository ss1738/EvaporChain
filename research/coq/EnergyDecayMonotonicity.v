(* ===================================================================== *)
(*  EvaporChain — Energy Decay Monotonicity                              *)
(*                                                                       *)
(*  Mechanization of the punch-list #7 obligation: prove that the        *)
(*  integer-arithmetic decay function `energy_at_epoch` is monotonically *)
(*  non-increasing in `epochs_elapsed`.                                  *)
(*                                                                       *)
(*  The Rust definition lives at:                                        *)
(*      crates/evaporchain-types/src/lib.rs:1331  energy_at_epoch        *)
(*                                                                       *)
(*  Rust shape (paraphrased — see source for exact bit operations):      *)
(*                                                                       *)
(*    fn energy_at_epoch(initial: u64, half_life: u64,                   *)
(*                      epochs_elapsed: u64) -> u64 {                    *)
(*        if half_life == 0 { return 0; }                                *)
(*        let full_halvings = epochs_elapsed / half_life;                *)
(*        let remainder      = epochs_elapsed % half_life;               *)
(*        if full_halvings >= 64 { return 0; }                           *)
(*        let after_halvings = initial >> full_halvings;                 *)
(*        let frac =                                                     *)
(*            (after_halvings * remainder) / (2 * half_life);            *)
(*        after_halvings.saturating_sub(frac)                            *)
(*    }                                                                  *)
(*                                                                       *)
(*  Why integer monotonicity is non-trivial: at the half-life boundary   *)
(*  (epoch e+1 where (e+1) mod h = 0), `full_halvings` increments and    *)
(*  `remainder` resets to 0. The value drops by exactly one halving in   *)
(*  that step. We need to prove the post-halving value at remainder=0    *)
(*  is no greater than the pre-boundary value at remainder=h-1.          *)
(*                                                                       *)
(*  Proof structure:                                                     *)
(*    - `decay_within_halving_monotone`: within a single half-life       *)
(*      window, energy decreases monotonically (covered by Lia + linear  *)
(*      interpolation bound).                                            *)
(*    - `decay_at_halving_boundary`: at the boundary, halving the value  *)
(*      is at most as large as `value - linear_decay(value, h-1, h)`.    *)
(*    - `energy_at_epoch_monotone`: the main theorem, by case split on   *)
(*      whether `e+1` crosses a half-life boundary.                      *)
(*                                                                       *)
(*  Status: spec + base cases + within-halving lemma proven.             *)
(*  Cross-boundary lemma is `Admitted` pending an arithmetic helper.     *)
(* ===================================================================== *)

From Coq Require Import Arith Lia.
From Coq Require Import Nat.
From Coq Require Import PeanoNat.

(* --------------------------------------------------------------------- *)
(*  Encoding choices                                                     *)
(*                                                                       *)
(*  We model `u64` with `nat`. This loses the 2^64 saturation but        *)
(*  simplifies the proof; we'll show monotonicity at the natural-number  *)
(*  level and argue separately that integer-overflow does not break      *)
(*  monotonicity (saturating-sub is itself monotone in its second arg).  *)
(* --------------------------------------------------------------------- *)

Definition energy := nat.
Definition half_life := nat.
Definition epoch := nat.

(* Natural-number bit-shift right is integer division by 2^k. *)
Fixpoint nat_shr (n : nat) (k : nat) : nat :=
  match k with
  | 0 => n
  | S k' => Nat.div2 (nat_shr n k')
  end.

Lemma nat_shr_zero : forall n, nat_shr n 0 = n.
Proof. intros. simpl. reflexivity. Qed.

Lemma nat_shr_succ : forall n k, nat_shr n (S k) = Nat.div2 (nat_shr n k).
Proof. intros. simpl. reflexivity. Qed.

(* Halving (Nat.div2) is monotone: this follows from the fact that
   nat_shr (S k) is at most nat_shr k. *)
Lemma nat_shr_monotone_step : forall n k,
    nat_shr n (S k) <= nat_shr n k.
Proof.
  intros n k.
  rewrite nat_shr_succ.
  (* Goal: Nat.div2 (nat_shr n k) <= nat_shr n k *)
  apply Nat.le_div2.
Qed.

Lemma nat_shr_monotone : forall n k1 k2,
    k1 <= k2 -> nat_shr n k2 <= nat_shr n k1.
Proof.
  intros n k1 k2 H.
  induction H as [| k2 Hk IH].
  - reflexivity.
  - eapply Nat.le_trans; [apply nat_shr_monotone_step | exact IH].
Qed.

(* --------------------------------------------------------------------- *)
(*  The decay function itself                                            *)
(* --------------------------------------------------------------------- *)

(* Linear interpolation within a halving window:
     frac = (value * remainder) / (2 * half_life)
   This models the Rust impl's linear approximation between halvings. *)
Definition linear_decay (value : nat) (remainder : nat) (h : half_life) : nat :=
  Nat.div (value * remainder) (2 * h).

(* The halving cutoff: at >= 64 halvings the Rust impl returns 0
   directly. We use a parameter rather than a literal so the proof
   doesn't rely on the specific cutoff value. *)
Definition halving_cutoff : nat := 64.

Definition energy_at_epoch (init : energy) (h : half_life)
                           (e : epoch) : energy :=
  if Nat.eqb h 0 then 0
  else
    let full := Nat.div e h in
    let rem  := Nat.modulo e h in
    if leb halving_cutoff full then 0
    else
      let after := nat_shr init full in
      after - linear_decay after rem h.

(* --------------------------------------------------------------------- *)
(*  Base cases (these go through cleanly)                                *)
(* --------------------------------------------------------------------- *)

(* At e = 0: energy = init (no decay yet). *)
Theorem energy_at_epoch_zero : forall init h,
    h <> 0 ->
    halving_cutoff <> 0 ->
    energy_at_epoch init h 0 = init.
Proof.
  intros init h Hh Hc.
  unfold energy_at_epoch.
  rewrite (proj2 (Nat.eqb_neq h 0) Hh).
  rewrite Nat.div_0_l by exact Hh.
  rewrite Nat.mod_0_l by exact Hh.
  (* full = 0, so leb halving_cutoff 0 is false (cutoff > 0) *)
  destruct halving_cutoff eqn:Hcut.
  - exfalso. apply Hc. reflexivity.
  - simpl.  (* leb (S n) 0 = false *)
    rewrite nat_shr_zero.
    unfold linear_decay.
    rewrite Nat.mul_0_r.  (* init * 0 = 0 *)
    rewrite Nat.div_0_l by lia.
    lia.
Qed.

(* If half_life = 0 the function returns 0. *)
Theorem energy_at_epoch_zero_halflife : forall init e,
    energy_at_epoch init 0 e = 0.
Proof.
  intros. unfold energy_at_epoch. simpl. reflexivity.
Qed.

(* If we're past the halving cutoff, energy is 0. *)
Theorem energy_at_epoch_past_cutoff : forall init h e,
    h <> 0 ->
    halving_cutoff <= Nat.div e h ->
    energy_at_epoch init h e = 0.
Proof.
  intros init h e Hh Hcut.
  unfold energy_at_epoch.
  rewrite (proj2 (Nat.eqb_neq h 0) Hh).
  apply Nat.leb_le in Hcut.
  rewrite Hcut.
  reflexivity.
Qed.

(* --------------------------------------------------------------------- *)
(*  Within-halving monotonicity (proven)                                 *)
(* --------------------------------------------------------------------- *)

(* Within a single half-life window — i.e., when stepping from epoch e
   to e+1 does NOT cross a half-life boundary — the linear-decay term
   only grows, so the energy only decreases. *)

Lemma linear_decay_monotone_in_remainder : forall v r h,
    h <> 0 ->
    linear_decay v r h <= linear_decay v (S r) h.
Proof.
  intros v r h Hh.
  unfold linear_decay.
  apply Nat.div_le_mono.
  - lia.
  - (* v * r <= v * (S r) *)
    apply Nat.mul_le_mono_l. lia.
Qed.

(* The linear-decay term never exceeds the value itself (the halving
   window is exactly one half-life wide, so the maximum linear decay at
   r = h-1 is roughly v/2, well within v). This bound is what makes the
   subtraction `after - linear_decay after rem h` always defined in N. *)
Lemma linear_decay_bounded : forall v r h,
    h <> 0 ->
    r < h ->
    linear_decay v r h <= v.
Proof.
  intros v r h Hh Hr.
  unfold linear_decay.
  (* (v * r) / (2 * h) <= v iff (v * r) <= v * (2 * h),
     which holds since r < h <= 2*h. *)
  apply Nat.div_le_upper_bound.
  - lia.
  - (* v <= 2 * h * v -- since 2*h >= 1 (because h >= 1) *)
    nia.
Qed.

(* --------------------------------------------------------------------- *)
(*  Main theorem: monotonicity                                           *)
(* --------------------------------------------------------------------- *)

(* Within-halving step: when (S e) does not cross a half-life boundary. *)
Lemma energy_step_within_halving : forall init h e,
    h <> 0 ->
    Nat.div (S e) h = Nat.div e h ->
    energy_at_epoch init h (S e) <= energy_at_epoch init h e.
Proof.
  intros init h e Hh Hdiv.
  unfold energy_at_epoch.
  rewrite (proj2 (Nat.eqb_neq h 0) Hh).
  rewrite Hdiv.
  destruct (leb halving_cutoff (Nat.div e h)) eqn:Hc.
  - reflexivity. (* both 0 *)
  - (* The remainder advances by 1 (no boundary crossing). *)
    assert (Hrem : Nat.modulo (S e) h = S (Nat.modulo e h)).
    { (* When div doesn't change, mod advances by exactly 1. *)
      (* Standard fact, discharged via Lia after rewriting. *)
      assert (Heq : S e = h * Nat.div e h + S (Nat.modulo e h)).
      { (* Use S e = e + 1 = (h * div e h + mod e h) + 1 *)
        rewrite Nat.add_1_r in *.
        rewrite (Nat.div_mod e h Hh) at 1.
        lia. }
      rewrite (Nat.div_mod (S e) h Hh) at 1.
      rewrite Hdiv in Heq.
      lia. }
    rewrite Hrem.
    set (after := nat_shr init (Nat.div e h)).
    (* Subtraction monotonicity: a - x <= a - y when y <= x.
       Use linear_decay_monotone_in_remainder. *)
    assert (Hmono : linear_decay after (Nat.modulo e h) h
                 <= linear_decay after (S (Nat.modulo e h)) h).
    { apply linear_decay_monotone_in_remainder. exact Hh. }
    lia.
Qed.

(* Cross-halving step: when (S e) crosses a half-life boundary, the
   number of halvings increments by exactly one and the remainder
   resets to 0. We need to show:
       nat_shr init (full + 1) - 0
       <= nat_shr init full - linear_decay (nat_shr init full) (h-1) h
   i.e., halving the value once is no greater than subtracting the
   end-of-window linear-decay value.
   The key fact: nat_shr v 1 = v / 2, and linear_decay v (h-1) h
   approximates v * (h-1) / (2*h) ≤ v/2 - 1 for typical v, but the
   bound depends on integer-rounding details that need a careful
   arithmetic argument. Left as Admitted for now. *)
Lemma energy_step_cross_halving : forall init h e,
    h <> 0 ->
    Nat.div (S e) h = S (Nat.div e h) ->
    halving_cutoff > S (Nat.div e h) ->
    energy_at_epoch init h (S e) <= energy_at_epoch init h e.
Proof.
  intros init h e Hh Hdiv Hcut.
  (* Proof obligation reduces to: for any value v and half-life h,
       nat_shr v 1 <= v - (v * (h-1)) / (2*h).
     This is true intuitively (halving = -50%, the linear term is
     just under -50% at remainder = h-1), but the precise bound
     requires case analysis on h's divisibility. *)
Admitted.

Theorem energy_at_epoch_monotone : forall init h e,
    h <> 0 ->
    energy_at_epoch init h (S e) <= energy_at_epoch init h e.
Proof.
  intros init h e Hh.
  destruct (Nat.eq_dec (Nat.div (S e) h) (Nat.div e h)) as [Heq | Hne].
  - apply energy_step_within_halving; assumption.
  - (* div advances. By integer-arith, it advances by exactly 1. *)
    assert (Hadv : Nat.div (S e) h = S (Nat.div e h)).
    { (* Standard: floor((e+1)/h) is either floor(e/h) or floor(e/h)+1. *)
      assert (HleS : Nat.div e h <= Nat.div (S e) h).
      { apply Nat.div_le_mono. exact Hh. lia. }
      assert (HleP : Nat.div (S e) h <= S (Nat.div e h)).
      { (* (S e)/h ≤ e/h + 1 because (S e) ≤ h * (e/h + 1) when e mod h < h. *)
        apply Nat.div_le_upper_bound. exact Hh.
        rewrite Nat.mul_succ_r.
        rewrite (Nat.div_mod e h Hh) at 1.
        assert (Hmod : Nat.modulo e h < h) by (apply Nat.mod_upper_bound; exact Hh).
        lia. }
      lia. }
    destruct (le_lt_dec halving_cutoff (S (Nat.div e h))) as [Hpc | Hpc].
    + (* Past cutoff after step: result is 0. *)
      rewrite (energy_at_epoch_past_cutoff init h (S e) Hh).
      * apply Nat.le_0_l.
      * rewrite Hadv. exact Hpc.
    + apply energy_step_cross_halving; auto.
Qed.

(* --------------------------------------------------------------------- *)
(*  Generalized monotonicity over arbitrary deltas                       *)
(* --------------------------------------------------------------------- *)

Theorem energy_at_epoch_monotone_general : forall init h e1 e2,
    h <> 0 ->
    e1 <= e2 ->
    energy_at_epoch init h e2 <= energy_at_epoch init h e1.
Proof.
  intros init h e1 e2 Hh Hle.
  induction Hle as [| e2 He IH].
  - reflexivity.
  - eapply Nat.le_trans.
    + apply energy_at_epoch_monotone. exact Hh.
    + exact IH.
Qed.

(* --------------------------------------------------------------------- *)
(*  What's left to discharge                                             *)
(*                                                                       *)
(*  `energy_step_cross_halving` is the only `Admitted` in this file.    *)
(*  Discharging it requires an arithmetic lemma of the form:             *)
(*                                                                       *)
(*    forall v h, h >= 1 ->                                              *)
(*      Nat.div v 2 <= v - Nat.div (v * (h - 1)) (2 * h).                *)
(*                                                                       *)
(*  Equivalent inequality:                                               *)
(*    Nat.div (v * (h - 1)) (2 * h) <= v - Nat.div v 2                   *)
(*    Nat.div (v * (h - 1)) (2 * h) <= Nat.div (v + 1) 2.  (*loose*)     *)
(*                                                                       *)
(*  This is provable by Lia after multiplying through, but the proof    *)
(*  needs care around the floor-divisions. Tracked as a follow-up.       *)
(* --------------------------------------------------------------------- *)
