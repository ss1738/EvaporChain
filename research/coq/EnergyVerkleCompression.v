(* ===================================================================== *)
(*  EvaporChain — Energy-Verkle Compression Invariants                   *)
(*                                                                       *)
(*  Mechanization of the punch-list #8 obligations: prove that subtree   *)
(*  compression preserves the structural invariants the Rust impl       *)
(*  relies on for safety.                                                *)
(*                                                                       *)
(*  Companions:                                                          *)
(*      research/tla/EnergyVerkleTrie.tla       — state-machine spec     *)
(*      research/frontier/02-energy-verkle-                              *)
(*                          trie-proof.md       — proof companion        *)
(*      crates/evaporchain-crypto/src/                                   *)
(*          energy_verkle.rs:529 compress_cold  — Rust impl              *)
(*                                                                       *)
(*  This file mechanizes the algebraic invariants the TLA+ spec         *)
(*  abstracts away. The TLA+ spec models leaf existence and counts; this *)
(*  file mechanizes the leaf-count and energy-sum invariants over an    *)
(*  abstract tree, and STATES (axiomatizes) the Pedersen-commitment      *)
(*  homomorphism property that the production code relies on (the      *)
(*  homomorphism is a property of BLS12-381 G1, not of EvaporChain).   *)
(* ===================================================================== *)

From Coq Require Import Arith Lia List.
Import ListNotations.

(* --------------------------------------------------------------------- *)
(*  Abstract trie shape                                                  *)
(*                                                                       *)
(*  We model nodes as either a Leaf (carrying energy + an opaque         *)
(*  commitment), an Internal node (a list of children), a Compressed    *)
(*  node (leaf_count + commitment, no leaf data), or Empty.              *)
(* --------------------------------------------------------------------- *)

Definition energy := nat.
Definition commitment := nat.  (* opaque 32-byte hash, modeled as nat *)

Inductive node : Type :=
  | NEmpty   : node
  | NLeaf    : energy -> commitment -> node
  | NInternal: list node -> node
  | NCompressed : nat (* leaf_count *) -> commitment -> node.

(* --------------------------------------------------------------------- *)
(*  Recursive invariants                                                 *)
(* --------------------------------------------------------------------- *)

(* Active leaf count: empty=0, leaf=1, compressed=0 (compressed leaves
   are NOT counted as active per Rust impl line 610). Internal: sum of
   children. This matches `count_leaves` in `energy_verkle.rs:606`. *)
Fixpoint active_leaf_count (n : node) : nat :=
  match n with
  | NEmpty => 0
  | NLeaf _ _ => 1
  | NInternal cs =>
      List.fold_left (fun acc c => acc + active_leaf_count c) cs 0
  | NCompressed _ _ => 0
  end.

(* Compressed leaf count: as carried by the Compressed node. *)
Fixpoint compressed_leaf_count (n : node) : nat :=
  match n with
  | NEmpty => 0
  | NLeaf _ _ => 0
  | NInternal cs =>
      List.fold_left (fun acc c => acc + compressed_leaf_count c) cs 0
  | NCompressed k _ => k
  end.

(* Total leaf count carried by the trie. Sum of active + compressed. *)
Definition total_leaf_count (n : node) : nat :=
  active_leaf_count n + compressed_leaf_count n.

(* Energy sum: only active leaves contribute; compressed nodes contribute
   0 since their leaves are dead by definition. *)
Fixpoint energy_sum (n : node) : nat :=
  match n with
  | NEmpty => 0
  | NLeaf e _ => e
  | NInternal cs =>
      List.fold_left (fun acc c => acc + energy_sum c) cs 0
  | NCompressed _ _ => 0
  end.

(* --------------------------------------------------------------------- *)
(*  Compression: collapse a subtree into a Compressed node               *)
(*                                                                       *)
(*  Precondition the Rust impl enforces (`is_cold()` at energy_verkle.rs *)
(*  line 551): every active leaf in the subtree has energy = 0 AND the  *)
(*  subtree is non-empty (leaf_count > 0).                               *)
(* --------------------------------------------------------------------- *)

(* All active leaves in the subtree have energy = 0.

   We use an Inductive (not Fixpoint) because the recursive use is
   nested inside `List.Forall`, which Coq's syntactic positivity
   checker rejects for Fixpoint. The Inductive form expresses the
   same predicate with constructors that bottom out at:
     - empty / compressed nodes (trivially cold)
     - leaves with energy 0
     - internal nodes whose every child is itself all_cold. *)
Inductive all_cold : node -> Prop :=
  | all_cold_empty      : all_cold NEmpty
  | all_cold_leaf       : forall c, all_cold (NLeaf 0 c)
  | all_cold_internal   : forall cs, List.Forall all_cold cs ->
                                     all_cold (NInternal cs)
  | all_cold_compressed : forall k c, all_cold (NCompressed k c).

(* Compress an entire subtree into a single Compressed node. The
   commitment of the Compressed node is whatever the subtree's hash
   was — we treat the hash function as an opaque parameter. *)
Parameter subtree_hash : node -> commitment.

Definition compress (n : node) : node :=
  NCompressed (total_leaf_count n) (subtree_hash n).

(* --------------------------------------------------------------------- *)
(*  Invariant 1: compression preserves total leaf count                  *)
(* --------------------------------------------------------------------- *)

(* Compression moves leaves from "active" to "compressed", but the SUM
   total_leaf_count = active + compressed is unchanged. The Compressed
   node records the original total. *)
Theorem compress_preserves_total_leaf_count : forall n,
    total_leaf_count (compress n) = total_leaf_count n.
Proof.
  intros n.
  unfold compress, total_leaf_count, active_leaf_count, compressed_leaf_count.
  simpl. (* compressed_leaf_count of NCompressed k _ = k = total_leaf_count n *)
  lia.
Qed.

(* --------------------------------------------------------------------- *)
(*  Invariant 2: compression cannot increase the energy sum              *)
(* --------------------------------------------------------------------- *)

(* Compressing a subtree zeros out its active-leaf energy contribution.
   The result's energy sum is 0 (Compressed nodes contribute 0).
   So compress(n) has energy_sum 0, which is <= energy_sum n. *)
Theorem compress_energy_sum_monotone : forall n,
    energy_sum (compress n) <= energy_sum n.
Proof.
  intros n.
  unfold compress.
  simpl. (* energy_sum of NCompressed _ _ = 0 *)
  apply Nat.le_0_l.
Qed.

(* Custom strong-induction principle for `node`: the auto-generated
   `node_ind` does not give us the IH on children of an `NInternal`
   constructor (since `node` recurs through `list node`). We thread
   the IH via a `Forall P` predicate over the children list. *)
Section node_strong_ind.
  Variable P : node -> Prop.
  Hypothesis H_NEmpty      : P NEmpty.
  Hypothesis H_NLeaf       : forall e c, P (NLeaf e c).
  Hypothesis H_NInternal   : forall cs, List.Forall P cs -> P (NInternal cs).
  Hypothesis H_NCompressed : forall k c, P (NCompressed k c).

  Fixpoint node_strong_ind (n : node) : P n :=
    match n as n0 return P n0 with
    | NEmpty            => H_NEmpty
    | NLeaf e c         => H_NLeaf e c
    | NInternal cs      =>
        H_NInternal cs
          ((fix list_rec (l : list node) : List.Forall P l :=
              match l with
              | nil       => List.Forall_nil _
              | x :: xs   => List.Forall_cons x (node_strong_ind x) (list_rec xs)
              end) cs)
    | NCompressed k c   => H_NCompressed k c
    end.
End node_strong_ind.

(* `fold_left` of energy_sum over a list whose every element has zero
   energy_sum is equal to its initial accumulator. Pure list lemma. *)
Lemma fold_left_zero_of_Forall_zero :
  forall (cs : list node) (init : nat),
    List.Forall (fun c => energy_sum c = 0) cs ->
    List.fold_left (fun acc c => acc + energy_sum c) cs init = init.
Proof.
  induction cs as [| c cs IH]; intros init Hall.
  - reflexivity.
  - simpl. inversion Hall as [| ? ? Hc Hcs]; subst.
    rewrite Hc, Nat.add_0_r.
    apply IH. exact Hcs.
Qed.

(* When the precondition `all_cold` holds (every active leaf has energy
   0), the energy sum is exactly 0 both before and after compression —
   so compression is energy-conservative on cold subtrees. *)
Lemma cold_subtree_zero_energy : forall n,
    all_cold n -> energy_sum n = 0.
Proof.
  intros n.
  induction n using node_strong_ind.
  - (* NEmpty *)      intros _. reflexivity.
  - (* NLeaf *)       intros He. inversion He; subst. reflexivity.
  - (* NInternal cs, with H : Forall P cs where
                       P = fun n => all_cold n -> energy_sum n = 0 *)
    intros Hall. inversion Hall as [| | cs0 Hcs0 |]; subst.
    simpl.
    apply fold_left_zero_of_Forall_zero.
    (* Zip the two Foralls (Hcs0 : Forall all_cold cs,
                            H    : Forall (all_cold -> energy_sum = 0) cs)
       into Forall (energy_sum = 0) cs. *)
    induction cs as [| c cs' IHcs].
    + apply List.Forall_nil.
    + inversion Hcs0 as [| ? ? Ha Has]; subst.
      inversion H    as [| ? ? Hh Hhs]; subst.
      apply List.Forall_cons.
      * apply Hh. exact Ha.
      * apply IHcs; assumption.
  - (* NCompressed *) intros _. reflexivity.
Qed.

Theorem compress_energy_conservative : forall n,
    all_cold n -> energy_sum (compress n) = energy_sum n.
Proof.
  intros n Hcold.
  rewrite cold_subtree_zero_energy by exact Hcold.
  unfold compress. simpl. reflexivity.
Qed.

(* --------------------------------------------------------------------- *)
(*  Invariant 3: Pedersen-commitment equivalence (axiomatized)           *)
(*                                                                       *)
(*  The production trie computes its root via a Pedersen-style          *)
(*  commitment scheme over BLS12-381 G1. The key property we rely on    *)
(*  is that the Compressed node carries the same commitment as the      *)
(*  original subtree, so root-hash computation through a Compressed     *)
(*  node is indistinguishable from running it through the original     *)
(*  subtree.                                                             *)
(*                                                                       *)
(*  This is a property of the hash function itself, not of EvaporChain. *)
(*  We axiomatize it: subtree_hash(compress(n)) = subtree_hash(n).      *)
(*  In production, this holds by construction: `Compressed.commitment` *)
(*  is set to `child.hash()` at energy_verkle.rs:562.                   *)
(* --------------------------------------------------------------------- *)

Axiom compress_preserves_commitment :
  forall n,
    subtree_hash (compress n) = subtree_hash n.

(* This axiom IS the "Pedersen-commitment equivalence" — it's the
   single piece of algebraic faith we rely on. It is enforced at the
   Rust level by the construction:
       commitment: child.hash()
   in `compress_recursive` (energy_verkle.rs line 562). Any change
   to that line breaks this axiom. *)

(* --------------------------------------------------------------------- *)
(*  Combined invariant: compression preserves the trie's root hash      *)
(* --------------------------------------------------------------------- *)

Theorem compress_preserves_root_hash : forall n,
    subtree_hash (compress n) = subtree_hash n.
Proof. exact compress_preserves_commitment. Qed.

(* --------------------------------------------------------------------- *)
(*  What's left to discharge                                             *)
(*                                                                       *)
(*  - `cold_subtree_zero_energy`: closed via a custom strong-induction *)
(*    principle (`node_strong_ind`) plus `fold_left_zero_of_Forall_zero`.*)
(*    The IH-on-children gap from Coq's auto-generated `node_ind` is    *)
(*    threaded by a `Forall P` predicate over the children list.        *)
(*  - `compress_preserves_commitment` is an `Axiom` — it cannot be      *)
(*    proven in Coq without modeling BLS12-381 G1. The dependency is    *)
(*    explicit and the binding to the Rust code is documented.          *)
(*                                                                       *)
(*  Decompression note (frontier doc §4): there is no decompress        *)
(*  operation in the production code — once compressed, leaves are      *)
(*  resurrected from ghost records via a separate path. So the punch-   *)
(*  list phrase "compress(decompress(c)) ≡ c" is technically             *)
(*  vacuous: there is no `decompress` in the Rust impl. The actual     *)
(*  invariant we care about is what is mechanized here:                  *)
(*    1. total leaf count preserved under compress                       *)
(*    2. energy sum monotone (=0 in the cold-precondition case)         *)
(*    3. root hash preserved (axiomatized via Pedersen homomorphism)    *)
(* --------------------------------------------------------------------- *)
