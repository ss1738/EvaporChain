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
(*  Status: spec + base cases + within-halving lemma + cross-halving    *)
(*  lemma all proven. The arithmetic helper is `decay_term_bound` and   *)
(*  is discharged below using `nia` over explicit `mul_div_le` and      *)
(*  `div_mod` bounds.                                                    *)
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
  rewrite Nat.div2_div.
  apply Nat.div_le_upper_bound; lia.
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

(* Arithmetic helper: the linear-decay term plus floor(v/2) never
   exceeds v, for any in-window remainder rm < h. Proven by combining
   `mul_div_le` lower bounds on the two floor-divisions with the
   constraint rm + 1 <= h. The certificate is:
       2h*Q <= v*rm                  (mul_div_le on Q)
       2*P  <= v                     (mul_div_le on P)
       rm + 1 <= h                   (from rm < h)
   ⇒  2h*(Q+P) + v <= 2*v*h          (linear combination)
   ⇒  Q + P <= v                     (since h >= 1, v - Q - P >= 0). *)
Lemma decay_term_bound : forall v h rm,
    h <> 0 -> rm < h ->
    Nat.div (v * rm) (2 * h) + Nat.div v 2 <= v.
Proof.
  intros v h rm Hh Hrm.
  pose proof (Nat.mul_div_le (v * rm) (2 * h) ltac:(lia)) as Hq.
  pose proof (Nat.mul_div_le v 2 ltac:(lia)) as Hp.
  (* Hq : 2 * h * (v * rm / (2 * h)) <= v * rm
     Hp : 2 * (v / 2) <= v *)
  (* Stage 1: bound 2*h*Q by v*h - v (since rm <= h - 1). *)
  assert (Hbound_Q : 2 * h * (Nat.div (v * rm) (2 * h)) <= v * h - v).
  { transitivity (v * (h - 1)).
    - transitivity (v * rm); [exact Hq | apply Nat.mul_le_mono_l; lia].
    - rewrite Nat.mul_sub_distr_l, Nat.mul_1_r. lia. }
  (* Stage 2: bound 2*h*P by h*v from 2*P <= v. *)
  assert (Hbound_P : 2 * h * (Nat.div v 2) <= h * v).
  { nia. }
  (* Stage 3: combine. 2*h*(Q+P) + v <= (v*h - v) + h*v + v = 2*v*h. *)
  assert (Hcombined : 2 * h * (Nat.div (v * rm) (2 * h) + Nat.div v 2) + v
                       <= 2 * v * h).
  { rewrite Nat.mul_add_distr_l. nia. }
  nia.
Qed.

(* Cross-halving step: when (S e) crosses a half-life boundary, the
   number of halvings increments by exactly one and the remainder
   resets to 0. We need to show:
       Nat.div2 (nat_shr init full)
       <= nat_shr init full - linear_decay (nat_shr init full) rm h
   for rm = mod e h < h, which is exactly `decay_term_bound`. *)
Lemma energy_step_cross_halving : forall init h e,
    h <> 0 ->
    Nat.div (S e) h = S (Nat.div e h) ->
    halving_cutoff > S (Nat.div e h) ->
    energy_at_epoch init h (S e) <= energy_at_epoch init h e.
Proof.
  intros init h e Hh Hdiv Hcut.
  unfold energy_at_epoch.
  rewrite (proj2 (Nat.eqb_neq h 0) Hh).
  rewrite Hdiv.
  assert (Hc1 : leb halving_cutoff (Nat.div e h) = false)
    by (apply Nat.leb_gt; lia).
  assert (Hc2 : leb halving_cutoff (S (Nat.div e h)) = false)
    by (apply Nat.leb_gt; lia).
  rewrite Hc1, Hc2.
  (* Boundary crossing forces (S e) to be a clean multiple of h, so
     mod (S e) h = 0. *)
  assert (Hrem : Nat.modulo (S e) h = 0).
  { pose proof (Nat.div_mod (S e) h Hh) as HSeq.
    pose proof (Nat.div_mod e h Hh)     as Heq.
    pose proof (Nat.mod_upper_bound (S e) h Hh) as Hu1.
    pose proof (Nat.mod_upper_bound e h Hh)     as Hu2.
    rewrite Hdiv in HSeq. nia. }
  rewrite Hrem.
  rewrite nat_shr_succ.
  (* LHS linear-decay collapses: rem = 0 ⇒ linear_decay _ 0 _ = 0. *)
  unfold linear_decay at 1.
  rewrite Nat.mul_0_r, Nat.div_0_l by lia.
  rewrite Nat.sub_0_r.
  unfold linear_decay.
  (* Goal: Nat.div2 (nat_shr init (Nat.div e h))
            <= nat_shr init (Nat.div e h)
               - Nat.div (nat_shr init (Nat.div e h) * Nat.modulo e h) (2 * h) *)
  rewrite Nat.div2_div.
  pose proof (Nat.mod_upper_bound e h Hh) as Hrm_lt.
  pose proof (decay_term_bound
                (nat_shr init (Nat.div e h))
                h
                (Nat.modulo e h)
                Hh Hrm_lt) as Hbound.
  lia.
Qed.

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
(*  All lemmas in this file are now `Qed` (no `Admitted`).              *)
(*                                                                       *)
(*  Closed via `decay_term_bound`, an arithmetic helper that bounds     *)
(*  the floor-divisions of the linear-decay term + halving by `nia`     *)
(*  using `Nat.mul_div_le` lower bounds. The certificate is documented  *)
(*  inline above the lemma.                                              *)
(* --------------------------------------------------------------------- *)
