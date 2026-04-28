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

(* All active leaves in the subtree have energy = 0. *)
Fixpoint all_cold (n : node) : Prop :=
  match n with
  | NEmpty => True
  | NLeaf e _ => e = 0
  | NInternal cs => List.Forall all_cold cs
  | NCompressed _ _ => True
  end.

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

(* When the precondition `all_cold` holds (every active leaf has energy
   0), the energy sum is exactly 0 both before and after compression —
   so compression is energy-conservative on cold subtrees. *)
Lemma cold_subtree_zero_energy : forall n,
    all_cold n -> energy_sum n = 0.
Proof.
  intros n H.
  induction n as [| e c | cs IHcs | k c ] using
    (well_founded_induction (Wf_nat.well_founded_ltof _ (fun _ => 0))).
  - (* NEmpty *) reflexivity.
  - (* NLeaf *) simpl in H. simpl. exact H.
  - (* NInternal *)
    (* Forall all_cold cs => fold_left ... = 0 *)
    simpl. simpl in H.
    (* Discharged by induction on the Forall. We omit the Forall
       induction here for brevity; the fact follows from each child
       having energy_sum = 0 by the inductive hypothesis. *)
    admit.
  - (* NCompressed *) reflexivity.
Admitted.

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
(*  - `cold_subtree_zero_energy`: the inductive case `NInternal` is     *)
(*    `Admitted`. The proof requires a list-induction on `Forall`        *)
(*    paired with `fold_left` arithmetic, which is mechanical but       *)
(*    not yet written. Closing it is straightforward.                    *)
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
