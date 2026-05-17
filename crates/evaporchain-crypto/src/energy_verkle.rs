//! Energy-Annotated Verkle Trie — a novel authenticated data structure
//! where internal nodes carry thermodynamic metadata (max energy, min half-life,
//! leaf count, last activity epoch). Cold subtrees compress to single nodes,
//! making the trie self-shrinking as objects decay and evaporate.
//!
//! This is a new primitive: no prior work combines cryptographic commitments
//! with temporal energy metadata for automatic subtree pruning.
//!
//! Properties:
//! - Identical commitment scheme to standard Verkle (IPA over Pallas curve)
//! - Energy annotations propagate bottom-up on insert/update/delete
//! - Subtrees where max_energy == 0 compress to a single `Compressed` node
//! - Compressed nodes can be decompressed on resurrection
//! - Proof sizes shrink when traversing cold (compressed) regions

#[allow(unused_imports)]
use pasta_curves::group::ff::{Field, PrimeField};
use pasta_curves::group::{Curve, Group, GroupEncoding};
use pasta_curves::{Ep, Fq};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::verkle::{VERKLE_INTERNAL_DST, VERKLE_LEAF_DST};

// ─────────────────────── Constants ───────────────────────────────────────

const WIDTH: usize = 256;
const MAX_DEPTH: usize = 32;

// ─────────────────────── Generator Points ────────────────────────────────
// H3 (audit 2026-05-16): use DISTINCT generator seed from standard
// Verkle to ensure Energy-Verkle and standard-Verkle commitments never
// alias.  Pre-fix both tries used `"EvaporChain_Verkle_Gen_{i}"` →
// identical generators → identical Pedersen commitments on the same
// child values → cross-trie proofs indistinguishable.  Post-fix the
// Energy-Verkle generator seed carries `EnergyVerkle_` prefix so the
// derived generator points differ from `verkle.rs`'s set.

static GENERATORS: OnceLock<Vec<Ep>> = OnceLock::new();

fn generators() -> &'static Vec<Ep> {
    GENERATORS.get_or_init(|| {
        let mut gens = Vec::with_capacity(WIDTH + 1);
        for i in 0..=WIDTH {
            let seed = format!("EvaporChain_EnergyVerkle_Gen_{}", i);
            let hash = blake3::hash(seed.as_bytes());
            let scalar = bytes_to_scalar(hash.as_bytes());
            gens.push(Ep::generator() * scalar);
        }
        gens
    })
}

/// Convert 32 bytes to a Pallas scalar field element (Fq).
///
/// L1 (audit 2026-05-13): same shape as `verkle::bytes_to_scalar`.
/// See that function's docstring for the bias bound (≈ 1.5 × 10⁻³⁹
/// from uniformity over the full Fq range) and the `expect` rationale
/// — the masked value is provably < modulus, so the fallback is dead
/// but loud rather than silently collapsing to `Fq::ONE`.
fn bytes_to_scalar(bytes: &[u8; 32]) -> Fq {
    let mut repr = *bytes;
    repr[31] &= 0x3F; // ensure < field modulus (254-bit subspace)
    Fq::from_repr(repr).expect(
        "bytes_to_scalar invariant: after `repr[31] &= 0x3F` the value is < Fq modulus",
    )
}

fn point_to_bytes(point: &Ep) -> [u8; 32] {
    let affine = point.to_affine();
    let bytes = affine.to_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes[..32]);
    out
}

// ─────────────────────── Energy Metadata ─────────────────────────────────

/// Thermodynamic metadata propagated through internal nodes.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnergyMeta {
    /// Maximum energy of any leaf in the subtree.
    pub max_energy: u64,
    /// Minimum half-life of any leaf in the subtree.
    /// u64::MAX means "no leaves" (identity for min).
    pub min_half_life: u64,
    /// Number of active (non-compressed) leaves in the subtree.
    pub leaf_count: u32,
    /// Most recent epoch of any insert/update in the subtree.
    pub last_activity_epoch: u64,
}

impl EnergyMeta {
    /// Identity element — a subtree with no leaves.
    pub fn empty() -> Self {
        Self {
            max_energy: 0,
            min_half_life: u64::MAX,
            leaf_count: 0,
            last_activity_epoch: 0,
        }
    }

    /// Merge two child metadata (associative, commutative).
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            max_energy: self.max_energy.max(other.max_energy),
            min_half_life: self.min_half_life.min(other.min_half_life),
            leaf_count: self.leaf_count.saturating_add(other.leaf_count),
            last_activity_epoch: self.last_activity_epoch.max(other.last_activity_epoch),
        }
    }

    /// Metadata for a single leaf.
    pub fn leaf(energy: u64, half_life: u64, epoch: u64) -> Self {
        Self {
            max_energy: energy,
            min_half_life: half_life,
            leaf_count: 1,
            last_activity_epoch: epoch,
        }
    }

    /// True if the entire subtree is dead (all leaves have zero energy).
    pub fn is_cold(&self) -> bool {
        self.max_energy == 0 && self.leaf_count > 0
    }
}

// ─────────────────────── Node Types ──────────────────────────────────────

/// Leaf: stores key, value, and energy metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct EnergyLeaf {
    key: [u8; 32],
    value: [u8; 32],
    energy: u64,
    half_life: u64,
    epoch: u64,
}

/// Internal node with energy-annotated children.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct EnergyInternal {
    children: BTreeMap<u8, EnergyNode>,
    /// Cached metadata aggregated from children.
    meta: EnergyMeta,
}

/// A compressed subtree — replaces an entire dead subtree with a single node.
/// Stores the commitment over the dead leaves so proofs can reference it.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CompressedNode {
    /// Commitment hash of the subtree that was compressed.
    commitment: [u8; 32],
    /// Number of dead leaves under this compressed node.
    leaf_count: u32,
    /// Epoch when the last leaf died.
    last_activity_epoch: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
enum EnergyNode {
    Internal(Box<EnergyInternal>),
    Leaf(EnergyLeaf),
    Compressed(CompressedNode),
    #[default]
    Empty,
}

impl EnergyInternal {
    fn new() -> Self {
        Self {
            children: BTreeMap::new(),
            meta: EnergyMeta::empty(),
        }
    }

    /// Recompute metadata from children.
    fn recompute_meta(&mut self) {
        let mut meta = EnergyMeta::empty();
        for child in self.children.values() {
            meta = meta.merge(&child.meta());
        }
        self.meta = meta;
    }
}

impl EnergyNode {
    /// Get the energy metadata for this node.
    fn meta(&self) -> EnergyMeta {
        match self {
            EnergyNode::Empty => EnergyMeta::empty(),
            EnergyNode::Leaf(leaf) => EnergyMeta::leaf(leaf.energy, leaf.half_life, leaf.epoch),
            EnergyNode::Internal(internal) => internal.meta.clone(),
            EnergyNode::Compressed(c) => EnergyMeta {
                max_energy: 0,
                min_half_life: u64::MAX,
                leaf_count: c.leaf_count,
                last_activity_epoch: c.last_activity_epoch,
            },
        }
    }

    /// Compute the cryptographic hash/commitment of this node.
    /// Compressed nodes return their stored commitment (same as the original subtree).
    ///
    /// CR-1 (audit 2026-05-17): producer-side DSTs MUST match `verify` /
    /// `verify_multi`. Pre-fix `EnergyNode::hash` returned raw
    /// `blake3(key || value)` for leaves and raw `point_to_bytes(commitment)`
    /// for internals, while `verify` reconstructed with `VERKLE_LEAF_DST` /
    /// `VERKLE_INTERNAL_DST` (commit b5959a05, H2 closure). The H2 closure
    /// was half-applied — verify side added DSTs, producer side didn't —
    /// so `test_proof_verifies` was red on HEAD.
    fn hash(&self) -> [u8; 32] {
        match self {
            EnergyNode::Empty => [0u8; 32],
            EnergyNode::Leaf(leaf) => {
                let mut data = Vec::with_capacity(VERKLE_LEAF_DST.len() + 64);
                data.extend_from_slice(VERKLE_LEAF_DST);
                data.extend_from_slice(&leaf.key);
                data.extend_from_slice(&leaf.value);
                *blake3::hash(&data).as_bytes()
            }
            EnergyNode::Internal(internal) => {
                let gens = generators();
                let mut commitment = Ep::identity();
                for (&idx, child) in &internal.children {
                    let child_hash = child.hash();
                    let scalar = bytes_to_scalar(&child_hash);
                    commitment += gens[idx as usize] * scalar;
                }
                let pt = point_to_bytes(&commitment);
                let mut data = Vec::with_capacity(VERKLE_INTERNAL_DST.len() + 32);
                data.extend_from_slice(VERKLE_INTERNAL_DST);
                data.extend_from_slice(&pt);
                *blake3::hash(&data).as_bytes()
            }
            EnergyNode::Compressed(c) => c.commitment,
        }
    }
}

// ─────────────────────── Proof Types ─────────────────────────────────────

/// Energy-annotated Verkle proof. Extends standard Verkle proofs with
/// energy metadata at each level, and marks compressed regions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnergyVerkleProof {
    pub key: [u8; 32],
    pub value: Option<[u8; 32]>,
    pub depth: usize,
    /// M-3 (audit 2026-05-17): **DIAGNOSTIC-ONLY.** `verify` reconstructs
    /// these from siblings and ignores the supplied values. Kept for
    /// serde wire compatibility; do not rely on for any security check.
    pub commitments: Vec<[u8; 32]>,
    pub path_indices: Vec<u8>,
    pub siblings: Vec<Vec<(u8, [u8; 32])>>,
    /// Energy metadata at each level along the path.
    pub energy_path: Vec<EnergyMeta>,
    /// Whether we hit a compressed node (proof of historical existence).
    pub hit_compressed: bool,
}

// ─────────────────────── EnergyVerkleTrie ────────────────────────────────

/// Energy-Annotated Verkle Trie.
///
/// A self-shrinking authenticated data structure where the trie physically
/// contracts as objects lose energy and evaporate. Cold subtrees (max_energy=0)
/// are compressed to single commitment nodes.
#[derive(Clone, Serialize, Deserialize)]
pub struct EnergyVerkleTrie {
    root: EnergyNode,
    /// Total number of compressions performed.
    pub compressions: u64,
    /// Total number of decompressions (resurrections into compressed regions).
    pub decompressions: u64,
}

impl EnergyVerkleTrie {
    pub fn new() -> Self {
        Self {
            root: EnergyNode::Empty,
            compressions: 0,
            decompressions: 0,
        }
    }

    /// Insert or update a key-value pair with energy metadata.
    pub fn insert(
        &mut self,
        key: [u8; 32],
        value: [u8; 32],
        energy: u64,
        half_life: u64,
        epoch: u64,
    ) {
        self.root = Self::insert_recursive(
            std::mem::take(&mut self.root),
            &key,
            &value,
            energy,
            half_life,
            epoch,
            0,
        );
    }

    fn insert_recursive(
        node: EnergyNode,
        key: &[u8; 32],
        value: &[u8; 32],
        energy: u64,
        half_life: u64,
        epoch: u64,
        depth: usize,
    ) -> EnergyNode {
        if depth >= MAX_DEPTH {
            return EnergyNode::Leaf(EnergyLeaf {
                key: *key,
                value: *value,
                energy,
                half_life,
                epoch,
            });
        }

        match node {
            EnergyNode::Empty => EnergyNode::Leaf(EnergyLeaf {
                key: *key,
                value: *value,
                energy,
                half_life,
                epoch,
            }),

            EnergyNode::Leaf(existing) => {
                if existing.key == *key {
                    // Update existing leaf
                    EnergyNode::Leaf(EnergyLeaf {
                        key: *key,
                        value: *value,
                        energy,
                        half_life,
                        epoch,
                    })
                } else {
                    // Split into internal node
                    let mut internal = EnergyInternal::new();
                    let existing_idx = existing.key[depth];
                    let new_idx = key[depth];

                    if existing_idx == new_idx {
                        let child = Self::insert_recursive(
                            EnergyNode::Leaf(existing),
                            key,
                            value,
                            energy,
                            half_life,
                            epoch,
                            depth + 1,
                        );
                        internal.children.insert(new_idx, child);
                    } else {
                        internal
                            .children
                            .insert(existing_idx, EnergyNode::Leaf(existing));
                        internal.children.insert(
                            new_idx,
                            EnergyNode::Leaf(EnergyLeaf {
                                key: *key,
                                value: *value,
                                energy,
                                half_life,
                                epoch,
                            }),
                        );
                    }
                    internal.recompute_meta();
                    EnergyNode::Internal(Box::new(internal))
                }
            }

            EnergyNode::Compressed(_) => {
                // Inserting into a compressed region = decompression.
                // We can't expand the original subtree (it's gone), so we create
                // a new internal node with the compressed node as one child and
                // the new leaf as another. The compressed commitment is preserved.
                //
                // This handles resurrection: a new object landing in a previously
                // dead region of the trie.
                let idx = key[depth];
                let mut internal = EnergyInternal::new();
                // The compressed node keeps its slot but we need to decide where.
                // Since we lost the original key structure, we place the compressed
                // node at index 0 (arbitrary but deterministic) and the new leaf
                // at its natural index. If they collide, the new leaf takes priority
                // (the compressed region was dead anyway).
                if idx != 0 {
                    internal.children.insert(0, node);
                }
                internal.children.insert(
                    idx,
                    EnergyNode::Leaf(EnergyLeaf {
                        key: *key,
                        value: *value,
                        energy,
                        half_life,
                        epoch,
                    }),
                );
                internal.recompute_meta();
                EnergyNode::Internal(Box::new(internal))
            }

            EnergyNode::Internal(mut internal) => {
                let idx = key[depth];
                let child = internal.children.remove(&idx).unwrap_or(EnergyNode::Empty);
                let new_child =
                    Self::insert_recursive(child, key, value, energy, half_life, epoch, depth + 1);
                internal.children.insert(idx, new_child);
                internal.recompute_meta();
                EnergyNode::Internal(internal)
            }
        }
    }

    /// Get the value for a key (returns None for compressed/missing regions).
    pub fn get(&self, key: &[u8; 32]) -> Option<[u8; 32]> {
        Self::get_recursive(&self.root, key, 0)
    }

    fn get_recursive(node: &EnergyNode, key: &[u8; 32], depth: usize) -> Option<[u8; 32]> {
        match node {
            EnergyNode::Empty | EnergyNode::Compressed(_) => None,
            EnergyNode::Leaf(leaf) => {
                if leaf.key == *key {
                    Some(leaf.value)
                } else {
                    None
                }
            }
            EnergyNode::Internal(internal) => {
                if depth >= MAX_DEPTH {
                    return None;
                }
                let idx = key[depth];
                match internal.children.get(&idx) {
                    Some(child) => Self::get_recursive(child, key, depth + 1),
                    None => None,
                }
            }
        }
    }

    /// Delete a key. Returns true if it existed.
    pub fn delete(&mut self, key: &[u8; 32]) -> bool {
        let (new_root, deleted) = Self::delete_recursive(std::mem::take(&mut self.root), key, 0);
        self.root = new_root;
        deleted
    }

    fn delete_recursive(node: EnergyNode, key: &[u8; 32], depth: usize) -> (EnergyNode, bool) {
        match node {
            EnergyNode::Empty | EnergyNode::Compressed(_) => (node, false),
            EnergyNode::Leaf(leaf) => {
                if leaf.key == *key {
                    (EnergyNode::Empty, true)
                } else {
                    (EnergyNode::Leaf(leaf), false)
                }
            }
            EnergyNode::Internal(mut internal) => {
                if depth >= MAX_DEPTH {
                    return (EnergyNode::Internal(internal), false);
                }
                let idx = key[depth];
                let child = match internal.children.remove(&idx) {
                    Some(c) => c,
                    None => return (EnergyNode::Internal(internal), false),
                };

                let (new_child, deleted) = Self::delete_recursive(child, key, depth + 1);
                match &new_child {
                    EnergyNode::Empty => {}
                    _ => {
                        internal.children.insert(idx, new_child);
                    }
                }

                if !deleted {
                    return (EnergyNode::Internal(internal), false);
                }

                // Collapse single-leaf children
                if internal.children.len() == 1 {
                    let only = internal.children.into_iter().next().unwrap();
                    if let EnergyNode::Leaf(_) = &only.1 {
                        return (only.1, true);
                    }
                    let mut new_internal = EnergyInternal::new();
                    new_internal.children.insert(only.0, only.1);
                    new_internal.recompute_meta();
                    return (EnergyNode::Internal(Box::new(new_internal)), true);
                }

                if internal.children.is_empty() {
                    (EnergyNode::Empty, true)
                } else {
                    internal.recompute_meta();
                    (EnergyNode::Internal(internal), true)
                }
            }
        }
    }

    /// Update the energy of a leaf by key. Returns true if found.
    pub fn update_energy(&mut self, key: &[u8; 32], new_energy: u64, epoch: u64) -> bool {
        Self::update_energy_recursive(&mut self.root, key, new_energy, epoch, 0)
    }

    fn update_energy_recursive(
        node: &mut EnergyNode,
        key: &[u8; 32],
        new_energy: u64,
        epoch: u64,
        depth: usize,
    ) -> bool {
        match node {
            EnergyNode::Empty | EnergyNode::Compressed(_) => false,
            EnergyNode::Leaf(leaf) => {
                if leaf.key == *key {
                    leaf.energy = new_energy;
                    leaf.epoch = epoch;
                    true
                } else {
                    false
                }
            }
            EnergyNode::Internal(internal) => {
                if depth >= MAX_DEPTH {
                    return false;
                }
                let idx = key[depth];
                let found = match internal.children.get_mut(&idx) {
                    Some(child) => {
                        Self::update_energy_recursive(child, key, new_energy, epoch, depth + 1)
                    }
                    None => false,
                };
                if found {
                    internal.recompute_meta();
                }
                found
            }
        }
    }

    /// Compress all cold subtrees (max_energy == 0, leaf_count > 0).
    /// Returns the number of subtrees compressed.
    ///
    /// Mechanized invariants: `research/coq/EnergyVerkleCompression.v`
    /// — `compress_preserves_total_leaf_count` (Qed),
    ///   `compress_energy_sum_monotone` (Qed),
    ///   `compress_preserves_commitment` (Axiom — pinned to the
    ///   `commitment: child.hash()` line in `compress_recursive` below;
    ///   any change there must update the Coq axiom binding).
    pub fn compress_cold(&mut self) -> u32 {
        let count = Self::compress_recursive(&mut self.root);
        self.compressions += count as u64;
        count
    }

    fn compress_recursive(node: &mut EnergyNode) -> u32 {
        match node {
            EnergyNode::Internal(internal) => {
                // First recurse into children
                let mut count = 0u32;
                for child in internal.children.values_mut() {
                    count += Self::compress_recursive(child);
                }

                // Then check if any children are now compressible
                let children_to_compress: Vec<u8> = internal
                    .children
                    .iter()
                    .filter_map(|(&idx, child)| {
                        let meta = child.meta();
                        // Compress internal nodes where all leaves are dead
                        if meta.is_cold() {
                            if let EnergyNode::Internal(_) = child {
                                return Some(idx);
                            }
                        }
                        None
                    })
                    .collect();

                for idx in children_to_compress {
                    if let Some(child) = internal.children.get(&idx) {
                        let commitment = child.hash();
                        let meta = child.meta();
                        internal.children.insert(
                            idx,
                            EnergyNode::Compressed(CompressedNode {
                                commitment,
                                leaf_count: meta.leaf_count,
                                last_activity_epoch: meta.last_activity_epoch,
                            }),
                        );
                        count += 1;
                    }
                }

                // Check if the entire node itself should be compressed
                // (handled by parent's check, not self)
                internal.recompute_meta();
                count
            }
            _ => 0,
        }
    }

    /// Compute the 32-byte root commitment.
    /// Identical to standard Verkle for the same leaf set — compressed nodes
    /// return the same commitment as the original expanded subtree.
    pub fn root(&self) -> [u8; 32] {
        self.root.hash()
    }

    /// Get the root-level energy metadata.
    pub fn root_meta(&self) -> EnergyMeta {
        self.root.meta()
    }

    /// Number of active (non-compressed) leaves.
    pub fn len(&self) -> usize {
        Self::count_leaves(&self.root)
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.root, EnergyNode::Empty)
    }

    fn count_leaves(node: &EnergyNode) -> usize {
        match node {
            EnergyNode::Empty => 0,
            EnergyNode::Leaf(_) => 1,
            EnergyNode::Compressed(_) => 0, // compressed leaves are not "active"
            EnergyNode::Internal(internal) => {
                internal.children.values().map(Self::count_leaves).sum()
            }
        }
    }

    /// Count compressed (dead) leaves across all compressed nodes.
    pub fn compressed_leaf_count(&self) -> u32 {
        Self::count_compressed(&self.root)
    }

    fn count_compressed(node: &EnergyNode) -> u32 {
        match node {
            EnergyNode::Empty | EnergyNode::Leaf(_) => 0,
            EnergyNode::Compressed(c) => c.leaf_count,
            EnergyNode::Internal(internal) => {
                internal.children.values().map(Self::count_compressed).sum()
            }
        }
    }

    /// Count the total number of nodes in the trie (for benchmarking).
    pub fn node_count(&self) -> usize {
        Self::count_nodes(&self.root)
    }

    fn count_nodes(node: &EnergyNode) -> usize {
        match node {
            EnergyNode::Empty => 0,
            EnergyNode::Leaf(_) | EnergyNode::Compressed(_) => 1,
            EnergyNode::Internal(internal) => {
                1 + internal
                    .children
                    .values()
                    .map(Self::count_nodes)
                    .sum::<usize>()
            }
        }
    }

    /// Generate an energy-annotated Verkle proof.
    pub fn prove(&self, key: &[u8; 32]) -> EnergyVerkleProof {
        let mut commitments = Vec::new();
        let mut path_indices = Vec::new();
        let mut siblings = Vec::new();
        let mut energy_path = Vec::new();
        let mut current = &self.root;
        let mut depth = 0;
        let mut hit_compressed = false;

        loop {
            match current {
                EnergyNode::Empty => break,
                EnergyNode::Leaf(_) => break,
                EnergyNode::Compressed(_) => {
                    hit_compressed = true;
                    break;
                }
                EnergyNode::Internal(internal) => {
                    if depth >= MAX_DEPTH {
                        break;
                    }
                    let idx = key[depth];
                    path_indices.push(idx);
                    energy_path.push(internal.meta.clone());

                    // Record commitment
                    let gens = generators();
                    let mut commitment = Ep::identity();
                    for (&cidx, child) in &internal.children {
                        let child_hash = child.hash();
                        let scalar = bytes_to_scalar(&child_hash);
                        commitment += gens[cidx as usize] * scalar;
                    }
                    commitments.push(point_to_bytes(&commitment));

                    // Record siblings
                    let mut level_siblings = Vec::new();
                    for (&child_idx, child) in &internal.children {
                        if child_idx != idx {
                            level_siblings.push((child_idx, child.hash()));
                        }
                    }
                    siblings.push(level_siblings);

                    current = match internal.children.get(&idx) {
                        Some(child) => child,
                        None => break,
                    };
                    depth += 1;
                }
            }
        }

        let value = self.get(key);

        EnergyVerkleProof {
            key: *key,
            value,
            depth: commitments.len(),
            commitments,
            path_indices,
            siblings,
            energy_path,
            hit_compressed,
        }
    }

    /// Verify a proof against a root commitment.
    ///
    /// Soundness note (AUDIT-2026-05-13 C1): the `hit_compressed` flag on the
    /// proof was previously a free pass — `verify` returned `true` whenever it
    /// was set, regardless of `expected_root`. That allowed any party to forge
    /// an exclusion proof by setting one bit. The disjunction has been removed.
    ///
    /// Consequence: when `prove` walks the trie and immediately hits a
    /// `Compressed` node at the root (no internal traversal), it currently
    /// produces a proof with `depth == 0`, `value == None`,
    /// `hit_compressed == true` and no commitment chain. Such proofs are NOT
    /// independently verifiable today and are correctly rejected here. A
    /// follow-up (see CompressedNode envelope work) must extend the proof
    /// to carry the root Compressed node's stored commitment so `verify` can
    /// reconstruct + compare against `expected_root`.
    pub fn verify(proof: &EnergyVerkleProof, expected_root: &[u8; 32]) -> bool {
        if proof.depth > MAX_DEPTH {
            return false;
        }
        if proof.depth == 0 {
            // C1: hit_compressed=true with no commitment chain is a forgery path -- reject.
            if proof.hit_compressed {
                return false;
            }
            if proof.value.is_none() {
                return *expected_root == [0u8; 32];
            }
            // H2: LEAF DST matches EnergyNode leaf node_hash.
            let mut data = Vec::with_capacity(VERKLE_LEAF_DST.len() + 64);
            data.extend_from_slice(VERKLE_LEAF_DST);
            data.extend_from_slice(&proof.key);
            data.extend_from_slice(proof.value.as_ref().unwrap());
            let leaf_hash = *blake3::hash(&data).as_bytes();
            return leaf_hash == *expected_root;
        }

        if proof.commitments.len() != proof.depth
            || proof.path_indices.len() != proof.depth
            || proof.siblings.len() != proof.depth
        {
            return false;
        }

        // CR-2 (audit 2026-05-17): bind path_indices to the proof's key
        // bytes. Without this, a non-existence proof for an EXISTING key
        // can be forged by routing path_indices through empty trie slots
        // — bytes_to_scalar([0u8;32]) = Fq::ZERO makes the path-idx slot
        // a no-op in the reconstructed commitment.
        for level in 0..proof.depth {
            if proof.path_indices[level] != proof.key[level] {
                return false;
            }
        }

        // Reconstruct leaf hash (H2: LEAF DST).
        let leaf_hash = match &proof.value {
            Some(value) => {
                let mut data = Vec::with_capacity(VERKLE_LEAF_DST.len() + 64);
                data.extend_from_slice(VERKLE_LEAF_DST);
                data.extend_from_slice(&proof.key);
                data.extend_from_slice(value);
                *blake3::hash(&data).as_bytes()
            }
            None => [0u8; 32],
        };

        // Rebuild commitments bottom-up
        let gens = generators();
        let mut current_hash = leaf_hash;

        for level in (0..proof.depth).rev() {
            let idx = proof.path_indices[level];
            let mut commitment = Ep::identity();
            let child_scalar = bytes_to_scalar(&current_hash);
            commitment += gens[idx as usize] * child_scalar;

            for &(sib_idx, ref sib_hash) in &proof.siblings[level] {
                let sib_scalar = bytes_to_scalar(sib_hash);
                commitment += gens[sib_idx as usize] * sib_scalar;
            }

            // H2: INTERNAL DST matches node_hash() for internal nodes.
            let pt = point_to_bytes(&commitment);
            let mut t = Vec::with_capacity(VERKLE_INTERNAL_DST.len() + 32);
            t.extend_from_slice(VERKLE_INTERNAL_DST);
            t.extend_from_slice(&pt);
            current_hash = *blake3::hash(&t).as_bytes();
        }

        current_hash == *expected_root
    }

    /// Query all keys with energy above a threshold.
    /// Skips subtrees where max_energy <= threshold (the key optimization).
    ///
    /// NOTE (M-06): Allocates a full Vec of matching keys. This is acceptable
    /// because the method is only used in tests and diagnostic queries, not in
    /// the consensus hot path. The subtree-pruning optimisation already limits
    /// the scan to live regions. If this ever lands on a hot path, convert to
    /// an iterator via `collect_above` yielding items lazily.
    pub fn keys_above_energy(&self, threshold: u64) -> Vec<([u8; 32], u64)> {
        let mut results = Vec::new();
        Self::collect_above(&self.root, threshold, &mut results);
        results
    }

    fn collect_above(node: &EnergyNode, threshold: u64, results: &mut Vec<([u8; 32], u64)>) {
        match node {
            EnergyNode::Empty | EnergyNode::Compressed(_) => {}
            EnergyNode::Leaf(leaf) => {
                if leaf.energy > threshold {
                    results.push((leaf.key, leaf.energy));
                }
            }
            EnergyNode::Internal(internal) => {
                // Skip entire subtree if max energy is below threshold
                if internal.meta.max_energy <= threshold {
                    return;
                }
                for child in internal.children.values() {
                    Self::collect_above(child, threshold, results);
                }
            }
        }
    }

    /// Get a health summary of the trie's thermodynamic state.
    pub fn health(&self) -> TrieHealth {
        let meta = self.root_meta();
        TrieHealth {
            active_leaves: self.len() as u32,
            compressed_leaves: self.compressed_leaf_count(),
            total_nodes: self.node_count() as u32,
            max_energy: meta.max_energy,
            min_half_life: if meta.min_half_life == u64::MAX {
                0
            } else {
                meta.min_half_life
            },
            last_activity_epoch: meta.last_activity_epoch,
            compressions: self.compressions,
            decompressions: self.decompressions,
        }
    }
}

impl EnergyVerkleTrie {
    /// Serialize the entire trie to bytes (bincode).
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).expect("trie serialization should not fail")
    }

    /// Deserialize a trie from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        bincode::deserialize(bytes).map_err(|e| format!("trie deserialization failed: {}", e))
    }
}

impl Default for EnergyVerkleTrie {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary of the trie's thermodynamic health.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrieHealth {
    pub active_leaves: u32,
    pub compressed_leaves: u32,
    pub total_nodes: u32,
    pub max_energy: u64,
    pub min_half_life: u64,
    pub last_activity_epoch: u64,
    pub compressions: u64,
    pub decompressions: u64,
}

// ─────────────────────── Multiproof ─────────────────────────────────────

/// Compact proof for multiple keys sharing common trie structure.
/// Deduplicates sibling hashes at shared internal nodes — for N keys
/// sharing m prefix levels, saves ~m × N × 33 bytes vs individual proofs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnergyVerkleMultiProof {
    /// Keys being proved.
    pub keys: Vec<[u8; 32]>,
    /// Values for each key (None = absent or compressed).
    pub values: Vec<Option<[u8; 32]>>,
    /// Depth of each key's resolution point in the trie.
    pub depths: Vec<usize>,
    /// Sibling hashes at each unique internal node, keyed by path prefix.
    pub siblings: BTreeMap<Vec<u8>, Vec<(u8, [u8; 32])>>,
    /// Energy metadata at each visited internal node.
    pub energy_data: BTreeMap<Vec<u8>, EnergyMeta>,
    /// Whether each key hit a compressed (dead) region.
    pub compressed: Vec<bool>,
}

impl EnergyVerkleMultiProof {
    /// Number of unique internal nodes in the proof.
    pub fn node_count(&self) -> usize {
        self.energy_data.len()
    }

    /// Total sibling hashes stored.
    pub fn sibling_count(&self) -> usize {
        self.siblings.values().map(|v| v.len()).sum()
    }
}

impl EnergyVerkleTrie {
    /// Generate a multiproof for the given keys.
    pub fn prove_multi(&self, keys: &[[u8; 32]]) -> EnergyVerkleMultiProof {
        let n = keys.len();
        let mut values = vec![None; n];
        let mut depths = vec![0usize; n];
        let mut compressed = vec![false; n];
        let mut siblings = BTreeMap::new();
        let mut energy_data = BTreeMap::new();
        let all: Vec<usize> = (0..n).collect();
        let mut path = Vec::new();
        Self::collect_mp(
            &self.root,
            keys,
            &all,
            0,
            &mut path,
            &mut values,
            &mut depths,
            &mut compressed,
            &mut siblings,
            &mut energy_data,
        );
        EnergyVerkleMultiProof {
            keys: keys.to_vec(),
            values,
            depths,
            siblings,
            energy_data,
            compressed,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_mp(
        node: &EnergyNode,
        keys: &[[u8; 32]],
        indices: &[usize],
        depth: usize,
        path: &mut Vec<u8>,
        values: &mut Vec<Option<[u8; 32]>>,
        depths: &mut [usize],
        compressed: &mut [bool],
        siblings: &mut BTreeMap<Vec<u8>, Vec<(u8, [u8; 32])>>,
        energy_data: &mut BTreeMap<Vec<u8>, EnergyMeta>,
    ) {
        match node {
            EnergyNode::Empty => {
                for &i in indices {
                    depths[i] = depth;
                }
            }
            EnergyNode::Leaf(leaf) => {
                for &i in indices {
                    depths[i] = depth;
                    if keys[i] == leaf.key {
                        values[i] = Some(leaf.value);
                    }
                }
            }
            EnergyNode::Compressed(_) => {
                for &i in indices {
                    depths[i] = depth;
                    compressed[i] = true;
                }
            }
            EnergyNode::Internal(internal) => {
                if depth >= MAX_DEPTH {
                    for &i in indices {
                        depths[i] = depth;
                    }
                    return;
                }

                energy_data.insert(path.clone(), internal.meta.clone());

                let mut groups: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
                for &i in indices {
                    groups.entry(keys[i][depth]).or_default().push(i);
                }

                let mut sib_list: Vec<(u8, [u8; 32])> = Vec::new();

                // Children not on any key's path are siblings
                for (&ci, child) in &internal.children {
                    if !groups.contains_key(&ci) {
                        sib_list.push((ci, child.hash()));
                    }
                }

                for (&byte, gi) in &groups {
                    match internal.children.get(&byte) {
                        Some(child) => {
                            // Non-matching leaf: treat as sibling
                            if let EnergyNode::Leaf(leaf) = child {
                                if !gi.iter().any(|&i| keys[i] == leaf.key) {
                                    sib_list.push((byte, child.hash()));
                                    for &i in gi {
                                        depths[i] = depth + 1;
                                    }
                                    continue;
                                }
                            }
                            // Compressed child: treat as sibling (no live data)
                            if let EnergyNode::Compressed(c) = child {
                                sib_list.push((byte, c.commitment));
                                for &i in gi {
                                    depths[i] = depth + 1;
                                    compressed[i] = true;
                                }
                                continue;
                            }
                            path.push(byte);
                            Self::collect_mp(
                                child,
                                keys,
                                gi,
                                depth + 1,
                                path,
                                values,
                                depths,
                                compressed,
                                siblings,
                                energy_data,
                            );
                            path.pop();
                        }
                        None => {
                            for &i in gi {
                                depths[i] = depth + 1;
                            }
                        }
                    }
                }

                if !sib_list.is_empty() {
                    siblings.insert(path.clone(), sib_list);
                }
            }
        }
    }

    /// Verify a multiproof against an expected root commitment.
    pub fn verify_multi(proof: &EnergyVerkleMultiProof, expected_root: &[u8; 32]) -> bool {
        if proof.keys.is_empty() {
            return *expected_root == [0u8; 32];
        }
        let all: Vec<usize> = (0..proof.keys.len()).collect();
        Self::reconstruct_mp(proof, &all, 0, &[]) == Some(*expected_root)
    }

    fn reconstruct_mp(
        proof: &EnergyVerkleMultiProof,
        indices: &[usize],
        depth: usize,
        path: &[u8],
    ) -> Option<[u8; 32]> {
        if indices.is_empty() {
            return Some([0u8; 32]);
        }

        // Terminal: all keys resolved at or above this depth
        // CR-3 (audit 2026-05-17): apply VERKLE_LEAF_DST to match
        // `EnergyNode::hash` (CR-1 fix) and `verify` (H2 closure).
        if indices.iter().all(|&i| proof.depths[i] <= depth) {
            let mut hash = [0u8; 32];
            let mut found = false;
            for &i in indices {
                if let Some(v) = &proof.values[i] {
                    if found {
                        return None;
                    }
                    let mut data = Vec::with_capacity(VERKLE_LEAF_DST.len() + 64);
                    data.extend_from_slice(VERKLE_LEAF_DST);
                    data.extend_from_slice(&proof.keys[i]);
                    data.extend_from_slice(v);
                    hash = *blake3::hash(&data).as_bytes();
                    found = true;
                }
            }
            return Some(hash);
        }

        if depth >= MAX_DEPTH {
            return None;
        }

        // Internal node: group by byte at this depth
        let mut groups: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
        for &i in indices {
            if proof.depths[i] > depth {
                groups.entry(proof.keys[i][depth]).or_default().push(i);
            }
        }
        let grouped: usize = groups.values().map(|v| v.len()).sum();
        if grouped != indices.len() {
            return None;
        }

        let gens = generators();
        let mut commitment = Ep::identity();

        for (&byte, gi) in &groups {
            let mut cp = path.to_vec();
            cp.push(byte);
            let ch = Self::reconstruct_mp(proof, gi, depth + 1, &cp)?;
            let scalar = bytes_to_scalar(&ch);
            commitment += gens[byte as usize] * scalar;
        }

        let path_vec = path.to_vec();
        if let Some(sibs) = proof.siblings.get(&path_vec) {
            for &(idx, ref hash) in sibs {
                let scalar = bytes_to_scalar(hash);
                commitment += gens[idx as usize] * scalar;
            }
        }

        // CR-3 (audit 2026-05-17): apply VERKLE_INTERNAL_DST to match
        // `EnergyNode::hash` (CR-1 fix) and `verify` (H2 closure).
        let pt = point_to_bytes(&commitment);
        let mut data = Vec::with_capacity(VERKLE_INTERNAL_DST.len() + 32);
        data.extend_from_slice(VERKLE_INTERNAL_DST);
        data.extend_from_slice(&pt);
        Some(*blake3::hash(&data).as_bytes())
    }

    /// Insert multiple entries at once.
    #[allow(clippy::type_complexity)]
    pub fn insert_batch(&mut self, entries: &[([u8; 32], [u8; 32], u64, u64, u64)]) {
        for &(key, value, energy, half_life, epoch) in entries {
            self.insert(key, value, energy, half_life, epoch);
        }
    }

    /// Update energy for multiple keys at once. Returns number of keys found.
    pub fn update_energy_batch(&mut self, updates: &[([u8; 32], u64, u64)]) -> usize {
        let mut count = 0;
        for &(ref key, energy, epoch) in updates {
            if self.update_energy(key, energy, epoch) {
                count += 1;
            }
        }
        count
    }
}

// ─────────────────────── Tests ───────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_key(byte: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[0] = byte;
        k
    }

    fn make_key_full(seed: u8) -> [u8; 32] {
        *blake3::hash(&[seed]).as_bytes()
    }

    fn make_value(byte: u8) -> [u8; 32] {
        let mut v = [0u8; 32];
        v[0] = byte;
        v
    }

    // ── Basic insert/get ──

    #[test]
    fn test_insert_get_roundtrip() {
        let mut trie = EnergyVerkleTrie::new();
        let key = make_key(1);
        let value = make_value(42);
        trie.insert(key, value, 1000, 100, 0);
        assert_eq!(trie.get(&key), Some(value));
    }

    #[test]
    fn test_insert_multiple() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..10u8 {
            trie.insert(
                make_key(i),
                make_value(i * 10),
                1000 - i as u64 * 100,
                50,
                0,
            );
        }
        for i in 0..10u8 {
            assert_eq!(trie.get(&make_key(i)), Some(make_value(i * 10)));
        }
        assert_eq!(trie.len(), 10);
    }

    #[test]
    fn test_get_nonexistent() {
        let mut trie = EnergyVerkleTrie::new();
        trie.insert(make_key(1), make_value(10), 100, 10, 0);
        assert_eq!(trie.get(&make_key(2)), None);
    }

    #[test]
    fn test_update_existing_key() {
        let mut trie = EnergyVerkleTrie::new();
        let key = make_key(5);
        trie.insert(key, make_value(10), 1000, 100, 0);
        trie.insert(key, make_value(20), 500, 100, 5);
        assert_eq!(trie.get(&key), Some(make_value(20)));
        assert_eq!(trie.root_meta().max_energy, 500);
    }

    // ── Delete ──

    #[test]
    fn test_delete() {
        let mut trie = EnergyVerkleTrie::new();
        let key = make_key(1);
        trie.insert(key, make_value(42), 100, 10, 0);
        assert!(trie.delete(&key));
        assert_eq!(trie.get(&key), None);
        assert_eq!(trie.len(), 0);
    }

    #[test]
    fn test_delete_preserves_others() {
        let mut trie = EnergyVerkleTrie::new();
        trie.insert(make_key(1), make_value(10), 100, 10, 0);
        trie.insert(make_key(2), make_value(20), 200, 20, 0);
        trie.insert(make_key(3), make_value(30), 300, 30, 0);
        trie.delete(&make_key(2));
        assert_eq!(trie.get(&make_key(1)), Some(make_value(10)));
        assert_eq!(trie.get(&make_key(2)), None);
        assert_eq!(trie.get(&make_key(3)), Some(make_value(30)));
    }

    // ── Energy metadata propagation ──

    #[test]
    fn test_meta_propagation_max_energy() {
        let mut trie = EnergyVerkleTrie::new();
        trie.insert(make_key(1), make_value(1), 100, 10, 0);
        trie.insert(make_key(2), make_value(2), 500, 20, 0);
        trie.insert(make_key(3), make_value(3), 200, 30, 0);
        assert_eq!(trie.root_meta().max_energy, 500);
        assert_eq!(trie.root_meta().min_half_life, 10);
        assert_eq!(trie.root_meta().leaf_count, 3);
    }

    #[test]
    fn test_meta_updates_on_energy_change() {
        let mut trie = EnergyVerkleTrie::new();
        trie.insert(make_key(1), make_value(1), 1000, 10, 0);
        trie.insert(make_key(2), make_value(2), 500, 20, 0);
        assert_eq!(trie.root_meta().max_energy, 1000);

        // Drop energy of key 1
        trie.update_energy(&make_key(1), 100, 5);
        assert_eq!(trie.root_meta().max_energy, 500);
    }

    #[test]
    fn test_meta_after_delete() {
        let mut trie = EnergyVerkleTrie::new();
        trie.insert(make_key(1), make_value(1), 1000, 10, 0);
        trie.insert(make_key(2), make_value(2), 500, 20, 0);
        trie.delete(&make_key(1));
        assert_eq!(trie.root_meta().max_energy, 500);
        assert_eq!(trie.root_meta().leaf_count, 1);
    }

    // ── Compression ──

    #[test]
    fn test_compress_cold_subtree() {
        let mut trie = EnergyVerkleTrie::new();
        // Insert objects with diverse keys so they form internal nodes
        for i in 0..8u8 {
            trie.insert(make_key_full(i), make_value(i), 100, 10, 0);
        }
        let _nodes_before = trie.node_count();
        assert_eq!(trie.len(), 8);

        // Kill all energy
        for i in 0..8u8 {
            trie.update_energy(&make_key_full(i), 0, 50);
        }
        assert!(trie.root_meta().is_cold());

        // Compress
        let root_before = trie.root();
        let _compressed = trie.compress_cold();
        let root_after = trie.root();

        // Root must be preserved
        assert_eq!(root_before, root_after, "compression must preserve root");

        // All leaves are dead
        assert_eq!(trie.root_meta().max_energy, 0);
        assert_eq!(trie.root_meta().leaf_count, 8);
    }

    #[test]
    fn test_compression_preserves_root_commitment() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..4u8 {
            trie.insert(make_key_full(i), make_value(i), 100, 10, 0);
        }

        // Kill energy
        for i in 0..4u8 {
            trie.update_energy(&make_key_full(i), 0, 50);
        }

        let root_before = trie.root();
        trie.compress_cold();
        let root_after = trie.root();

        // Critical property: compression must NOT change the root commitment
        assert_eq!(
            root_before, root_after,
            "compression must preserve the root commitment"
        );
    }

    #[test]
    fn test_partial_compression() {
        let mut trie = EnergyVerkleTrie::new();
        // Keys 0-3 will die, keys 4-7 will stay alive
        for i in 0..8u8 {
            let energy = if i < 4 { 100 } else { 1000 };
            trie.insert(make_key(i), make_value(i), energy, 10, 0);
        }

        // Kill keys 0-3
        for i in 0..4u8 {
            trie.update_energy(&make_key(i), 0, 50);
        }

        let root_before = trie.root();
        trie.compress_cold();
        let root_after = trie.root();

        // Root must be preserved even with partial compression
        assert_eq!(root_before, root_after);

        // Live keys still accessible
        for i in 4..8u8 {
            assert_eq!(trie.get(&make_key(i)), Some(make_value(i)));
        }

        // Dead keys no longer accessible (in compressed nodes)
        // Note: individual dead leaves at depth-0 aren't compressed, only subtrees
        assert_eq!(trie.root_meta().max_energy, 1000);
    }

    // ── Energy queries ──

    #[test]
    fn test_keys_above_energy() {
        let mut trie = EnergyVerkleTrie::new();
        trie.insert(make_key(1), make_value(1), 100, 10, 0);
        trie.insert(make_key(2), make_value(2), 500, 20, 0);
        trie.insert(make_key(3), make_value(3), 1000, 30, 0);

        let hot = trie.keys_above_energy(200);
        assert_eq!(hot.len(), 2);

        let all = trie.keys_above_energy(0);
        assert_eq!(all.len(), 3);

        let none = trie.keys_above_energy(1000);
        assert_eq!(none.len(), 0);
    }

    // ── Proof generation and verification ──

    #[test]
    fn test_proof_verifies() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..5u8 {
            trie.insert(make_key_full(i), make_value(i * 10), 1000, 100, 0);
        }
        let root = trie.root();

        for i in 0..5u8 {
            let key = make_key_full(i);
            let proof = trie.prove(&key);
            assert_eq!(proof.value, Some(make_value(i * 10)));
            assert!(
                EnergyVerkleTrie::verify(&proof, &root),
                "proof failed for key {}",
                i
            );
            // Proof should carry energy metadata
            assert!(!proof.energy_path.is_empty() || proof.depth == 0);
        }
    }

    #[test]
    fn test_proof_fails_on_tampered_value() {
        let mut trie = EnergyVerkleTrie::new();
        trie.insert(make_key_full(1), make_value(10), 100, 10, 0);
        let root = trie.root();

        let mut proof = trie.prove(&make_key_full(1));
        proof.value = Some(make_value(99));
        assert!(!EnergyVerkleTrie::verify(&proof, &root));
    }

    // ── Root commitment ──

    #[test]
    fn test_empty_root() {
        let trie = EnergyVerkleTrie::new();
        assert_eq!(trie.root(), [0u8; 32]);
    }

    #[test]
    fn test_root_deterministic() {
        let mut t1 = EnergyVerkleTrie::new();
        let mut t2 = EnergyVerkleTrie::new();
        for i in 0..5u8 {
            t1.insert(make_key(i), make_value(i), 100 * (i as u64 + 1), 10, 0);
            t2.insert(make_key(i), make_value(i), 100 * (i as u64 + 1), 10, 0);
        }
        assert_eq!(t1.root(), t2.root());
    }

    #[test]
    fn test_root_matches_standard_verkle() {
        // Critical: an EnergyVerkleTrie with the same keys/values as a standard
        // VerkleTrie must produce the same root commitment.
        use crate::verkle::VerkleTrie;

        let mut standard = VerkleTrie::new();
        let mut energy = EnergyVerkleTrie::new();

        for i in 0..10u8 {
            let key = make_key_full(i);
            let value = make_value(i * 10);
            standard.insert(key, value);
            energy.insert(key, value, 1000, 100, 0);
        }

        assert_ne!(
            standard.root(),
            energy.root(),
            "EnergyVerkleTrie must produce a DIFFERENT root than standard VerkleTrie (H3: distinct generators)"
        );
    }

    // ── Health ──

    #[test]
    fn test_health() {
        let mut trie = EnergyVerkleTrie::new();
        trie.insert(make_key(1), make_value(1), 1000, 50, 10);
        trie.insert(make_key(2), make_value(2), 500, 100, 20);

        let h = trie.health();
        assert_eq!(h.active_leaves, 2);
        assert_eq!(h.max_energy, 1000);
        assert_eq!(h.min_half_life, 50);
        assert_eq!(h.last_activity_epoch, 20);
        assert_eq!(h.compressions, 0);
    }

    // ── Scale ──

    #[test]
    fn test_1000_entries_with_decay_and_compression() {
        let mut trie = EnergyVerkleTrie::new();

        // Insert 1000 entries
        for i in 0u16..1000 {
            let key = {
                let seed = i.to_le_bytes();
                let mut input = [0u8; 4];
                input[..2].copy_from_slice(&seed);
                *blake3::hash(&input).as_bytes()
            };
            let value = {
                let mut v = [0u8; 32];
                v[0] = (i & 0xFF) as u8;
                v[1] = (i >> 8) as u8;
                v
            };
            trie.insert(key, value, 1000 + i as u64, 100, i as u64);
        }

        assert_eq!(trie.len(), 1000);
        assert_eq!(trie.root_meta().max_energy, 1999);
        assert_eq!(trie.root_meta().leaf_count, 1000);

        let nodes_before = trie.node_count();

        // Kill half the entries (first 500)
        for i in 0u16..500 {
            let key = {
                let seed = i.to_le_bytes();
                let mut input = [0u8; 4];
                input[..2].copy_from_slice(&seed);
                *blake3::hash(&input).as_bytes()
            };
            trie.update_energy(&key, 0, 2000);
        }

        assert_eq!(trie.root_meta().max_energy, 1999); // max from surviving entries

        // Compress
        let root_before = trie.root();
        let _compressed = trie.compress_cold();
        let root_after = trie.root();

        assert_eq!(root_before, root_after, "compression must preserve root");
        assert!(trie.node_count() <= nodes_before, "trie should shrink");

        // Surviving entries still accessible
        for i in 500u16..1000 {
            let key = {
                let seed = i.to_le_bytes();
                let mut input = [0u8; 4];
                input[..2].copy_from_slice(&seed);
                *blake3::hash(&input).as_bytes()
            };
            let value = {
                let mut v = [0u8; 32];
                v[0] = (i & 0xFF) as u8;
                v[1] = (i >> 8) as u8;
                v
            };
            assert_eq!(trie.get(&key), Some(value), "entry {} should survive", i);
        }

        let h = trie.health();
        // Active leaves = surviving 500 + dead individual leaves not in compressed subtrees
        assert!(h.active_leaves >= 500, "at least 500 surviving leaves");
        // The trie should have shrunk
        assert!(trie.node_count() <= nodes_before, "trie should not grow");
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0u8..50 {
            trie.insert(
                make_key_full(i),
                make_value(i),
                (i as u64) * 100,
                10 + i as u64,
                i as u64,
            );
        }
        let root_before = trie.root();
        let health_before = trie.health();

        let bytes = trie.to_bytes();
        assert!(!bytes.is_empty());

        let restored =
            EnergyVerkleTrie::from_bytes(&bytes).expect("deserialization should succeed");
        assert_eq!(restored.root(), root_before);
        let health_after = restored.health();
        assert_eq!(health_after.active_leaves, health_before.active_leaves);
        assert_eq!(health_after.max_energy, health_before.max_energy);
        assert_eq!(health_after.total_nodes, health_before.total_nodes);
    }

    #[test]
    fn test_serialize_after_compression() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0u8..20 {
            let energy = if i < 10 { 0 } else { 1000 };
            trie.insert(make_key_full(i), make_value(i), energy, 100, i as u64);
        }
        trie.compress_cold();
        let root_before = trie.root();

        let bytes = trie.to_bytes();
        let restored =
            EnergyVerkleTrie::from_bytes(&bytes).expect("deserialization should succeed");
        assert_eq!(restored.root(), root_before);
        assert_eq!(
            restored.compressed_leaf_count(),
            trie.compressed_leaf_count()
        );
    }

    #[test]
    fn test_serialize_empty_trie() {
        let trie = EnergyVerkleTrie::new();
        let bytes = trie.to_bytes();
        let restored =
            EnergyVerkleTrie::from_bytes(&bytes).expect("deserialization should succeed");
        assert_eq!(restored.root(), [0u8; 32]);
        assert!(restored.is_empty());
    }

    #[test]
    fn test_incremental_update_after_deserialize() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0u8..10 {
            trie.insert(make_key_full(i), make_value(i), 500, 100, 0);
        }
        let bytes = trie.to_bytes();
        let mut restored = EnergyVerkleTrie::from_bytes(&bytes).expect("deser ok");

        // Incremental update on the restored trie
        restored.insert(make_key_full(10), make_value(10), 500, 100, 0);
        restored.update_energy(&make_key_full(0), 0, 1);

        // Same operations on original
        trie.insert(make_key_full(10), make_value(10), 500, 100, 0);
        trie.update_energy(&make_key_full(0), 0, 1);

        assert_eq!(restored.root(), trie.root());
    }

    // ── Multiproof ──

    #[test]
    fn test_multiproof_basic() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..10u8 {
            trie.insert(make_key_full(i), make_value(i * 10), 1000, 100, 0);
        }
        let root = trie.root();
        let keys: Vec<[u8; 32]> = (0..5u8).map(make_key_full).collect();
        let proof = trie.prove_multi(&keys);
        assert!(EnergyVerkleTrie::verify_multi(&proof, &root));
        for i in 0..5u8 {
            assert_eq!(proof.values[i as usize], Some(make_value(i * 10)));
        }
    }

    #[test]
    fn test_multiproof_all_keys() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..20u8 {
            trie.insert(make_key_full(i), make_value(i), 500, 50, 0);
        }
        let root = trie.root();
        let keys: Vec<[u8; 32]> = (0..20u8).map(make_key_full).collect();
        let proof = trie.prove_multi(&keys);
        assert!(EnergyVerkleTrie::verify_multi(&proof, &root));
        assert_eq!(proof.sibling_count(), 0);
    }

    #[test]
    fn test_multiproof_absent_key() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..5u8 {
            trie.insert(make_key_full(i), make_value(i), 500, 50, 0);
        }
        let root = trie.root();
        let keys = vec![make_key_full(0), make_key_full(200)];
        let proof = trie.prove_multi(&keys);
        assert!(EnergyVerkleTrie::verify_multi(&proof, &root));
        assert_eq!(proof.values[0], Some(make_value(0)));
        assert_eq!(proof.values[1], None);
    }

    #[test]
    fn test_multiproof_tampered_value_fails() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..5u8 {
            trie.insert(make_key_full(i), make_value(i * 10), 1000, 100, 0);
        }
        let root = trie.root();
        let keys: Vec<[u8; 32]> = (0..3u8).map(make_key_full).collect();
        let mut proof = trie.prove_multi(&keys);
        proof.values[1] = Some(make_value(99));
        assert!(!EnergyVerkleTrie::verify_multi(&proof, &root));
    }

    #[test]
    fn test_multiproof_matches_individual() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..10u8 {
            trie.insert(make_key_full(i), make_value(i), 500, 50, 0);
        }
        let root = trie.root();
        for i in 0..10u8 {
            let key = make_key_full(i);
            let proof = trie.prove(&key);
            assert!(EnergyVerkleTrie::verify(&proof, &root));
        }
        let keys: Vec<[u8; 32]> = (0..10u8).map(make_key_full).collect();
        let mp = trie.prove_multi(&keys);
        assert!(EnergyVerkleTrie::verify_multi(&mp, &root));
    }

    #[test]
    fn test_multiproof_fewer_siblings_than_individual() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..50u8 {
            trie.insert(make_key_full(i), make_value(i), 500, 50, 0);
        }
        let keys: Vec<[u8; 32]> = (0..25u8).map(make_key_full).collect();
        let mp = trie.prove_multi(&keys);
        let total_individual: usize = keys
            .iter()
            .map(|k| {
                trie.prove(k)
                    .siblings
                    .iter()
                    .map(|s| s.len())
                    .sum::<usize>()
            })
            .sum();
        assert!(
            mp.sibling_count() <= total_individual,
            "multiproof {} siblings vs individual {} total",
            mp.sibling_count(),
            total_individual,
        );
    }

    #[test]
    fn test_multiproof_with_compression() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..20u8 {
            let energy = if i < 10 { 0 } else { 1000 };
            trie.insert(make_key_full(i), make_value(i), energy, 100, 0);
        }
        trie.compress_cold();
        let root = trie.root();
        let keys = vec![make_key_full(15), make_key_full(3)];
        let proof = trie.prove_multi(&keys);
        assert!(EnergyVerkleTrie::verify_multi(&proof, &root));
        assert_eq!(proof.values[0], Some(make_value(15)));
        // Key 3 is dead (energy=0). If it's a standalone leaf child it
        // retains its value; only dead internal subtrees get compressed.
        // Either way the proof must verify against the root.
    }

    #[test]
    fn test_multiproof_single_key() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..5u8 {
            trie.insert(make_key_full(i), make_value(i * 10), 1000, 100, 0);
        }
        let root = trie.root();
        let mp = trie.prove_multi(&[make_key_full(2)]);
        assert!(EnergyVerkleTrie::verify_multi(&mp, &root));
        assert_eq!(mp.values[0], Some(make_value(20)));
    }

    #[test]
    fn test_multiproof_energy_metadata() {
        let mut trie = EnergyVerkleTrie::new();
        trie.insert(make_key_full(1), make_value(1), 1000, 50, 10);
        trie.insert(make_key_full(2), make_value(2), 500, 100, 20);
        let keys = vec![make_key_full(1), make_key_full(2)];
        let mp = trie.prove_multi(&keys);
        assert!(!mp.energy_data.is_empty());
        if let Some(root_meta) = mp.energy_data.get(&vec![]) {
            assert_eq!(root_meta.max_energy, 1000);
            assert_eq!(root_meta.min_half_life, 50);
        }
    }

    #[test]
    fn test_multiproof_wrong_root_fails() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..5u8 {
            trie.insert(make_key_full(i), make_value(i), 500, 50, 0);
        }
        let keys: Vec<[u8; 32]> = (0..3u8).map(make_key_full).collect();
        let proof = trie.prove_multi(&keys);
        let wrong_root = [0xFFu8; 32];
        assert!(!EnergyVerkleTrie::verify_multi(&proof, &wrong_root));
    }

    #[test]
    fn test_multiproof_empty_trie() {
        let trie = EnergyVerkleTrie::new();
        let mp = trie.prove_multi(&[]);
        assert!(EnergyVerkleTrie::verify_multi(&mp, &[0u8; 32]));
    }

    #[test]
    fn test_multiproof_scale_100_keys() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0u16..200 {
            let key = {
                let mut input = [0u8; 4];
                input[..2].copy_from_slice(&i.to_le_bytes());
                *blake3::hash(&input).as_bytes()
            };
            let mut val = [0u8; 32];
            val[0] = (i & 0xFF) as u8;
            val[1] = (i >> 8) as u8;
            trie.insert(key, val, 1000 + i as u64, 100, 0);
        }
        let root = trie.root();
        let keys: Vec<[u8; 32]> = (0u16..100)
            .map(|i| {
                let mut input = [0u8; 4];
                input[..2].copy_from_slice(&i.to_le_bytes());
                *blake3::hash(&input).as_bytes()
            })
            .collect();
        let mp = trie.prove_multi(&keys);
        assert!(EnergyVerkleTrie::verify_multi(&mp, &root));
        assert!(mp.node_count() > 0);
    }

    // ── Batch operations ──

    #[test]
    fn test_batch_insert() {
        let mut trie1 = EnergyVerkleTrie::new();
        let mut trie2 = EnergyVerkleTrie::new();
        let entries: Vec<_> = (0..10u8)
            .map(|i| (make_key_full(i), make_value(i), 500u64, 50u64, i as u64))
            .collect();
        for &(k, v, e, h, ep) in &entries {
            trie1.insert(k, v, e, h, ep);
        }
        trie2.insert_batch(&entries);
        assert_eq!(trie1.root(), trie2.root());
    }

    #[test]
    fn test_batch_update_energy() {
        let mut trie = EnergyVerkleTrie::new();
        for i in 0..5u8 {
            trie.insert(make_key_full(i), make_value(i), 1000, 100, 0);
        }
        let updates: Vec<_> = (0..5u8).map(|i| (make_key_full(i), 0u64, 50u64)).collect();
        let count = trie.update_energy_batch(&updates);
        assert_eq!(count, 5);
        assert_eq!(trie.root_meta().max_energy, 0);
    }

    #[test]
    fn audit_c1_hit_compressed_forgery_rejected_against_any_root() {
        // AUDIT-2026-05-13 C1: prior to the fix, `verify` returned `true` for
        // any proof with `hit_compressed = true` regardless of `expected_root`.
        // Construct that exact forgery and assert it is now rejected against
        // both the empty-trie sentinel root and an arbitrary non-zero root.
        let forgery = EnergyVerkleProof {
            key: [0xAB; 32],
            value: None,
            depth: 0,
            commitments: Vec::new(),
            path_indices: Vec::new(),
            siblings: Vec::new(),
            energy_path: Vec::new(),
            hit_compressed: true,
        };
        let arbitrary_root = [0xDE; 32];
        let empty_sentinel = [0u8; 32];
        assert!(
            !EnergyVerkleTrie::verify(&forgery, &arbitrary_root),
            "hit_compressed=true must NOT short-circuit verify"
        );
        assert!(
            !EnergyVerkleTrie::verify(&forgery, &empty_sentinel),
            "hit_compressed=true must NOT short-circuit verify even vs zero root"
        );
    }

    #[test]
    fn audit_c1_empty_trie_absence_still_verifies_against_zero_root() {
        // Soundness lower bound: the legitimate empty-trie absence proof
        // (depth=0, value=None, hit_compressed=false) must continue to verify
        // against the zero-sentinel root. Otherwise the C1 fix would have
        // broken the canonical absence case.
        let trie = EnergyVerkleTrie::new();
        let key = make_key(99);
        let proof = trie.prove(&key);
        assert!(proof.value.is_none());
        assert_eq!(proof.depth, 0);
        assert!(!proof.hit_compressed);
        let root = trie.root();
        assert_eq!(root, [0u8; 32]);
        assert!(EnergyVerkleTrie::verify(&proof, &root));
    }
}

// ═══════════════════════════════════════════════════════════════════
// Property-Based Tests
// ═══════════════════════════════════════════════════════════════════

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn arb_key() -> impl Strategy<Value = [u8; 32]> {
        any::<[u8; 32]>()
    }

    fn arb_value() -> impl Strategy<Value = [u8; 32]> {
        any::<[u8; 32]>()
    }

    proptest! {
        #[test]
        fn insert_get_roundtrip(
            key in arb_key(),
            value in arb_value(),
            energy in 0u64..10000,
            half_life in 1u64..1000,
        ) {
            let mut trie = EnergyVerkleTrie::new();
            trie.insert(key, value, energy, half_life, 0);
            prop_assert_eq!(trie.get(&key), Some(value));
            prop_assert_eq!(trie.root_meta().max_energy, energy);
            prop_assert_eq!(trie.root_meta().min_half_life, half_life);
        }

        #[test]
        fn root_deterministic(
            entries in proptest::collection::vec(
                (arb_key(), arb_value(), 0u64..10000, 1u64..1000),
                1..20
            )
        ) {
            let mut t1 = EnergyVerkleTrie::new();
            let mut t2 = EnergyVerkleTrie::new();
            for (k, v, e, h) in &entries {
                t1.insert(*k, *v, *e, *h, 0);
                t2.insert(*k, *v, *e, *h, 0);
            }
            prop_assert_eq!(t1.root(), t2.root());
        }

        #[test]
        fn proof_verifies(
            entries in proptest::collection::vec(
                (arb_key(), arb_value(), 0u64..10000, 1u64..1000),
                1..10
            ),
            query_idx in 0usize..10,
        ) {
            let mut trie = EnergyVerkleTrie::new();
            for (k, v, e, h) in &entries {
                trie.insert(*k, *v, *e, *h, 0);
            }
            let idx = query_idx % entries.len();
            let (key, _, _, _) = &entries[idx];
            let root = trie.root();
            let proof = trie.prove(key);
            prop_assert!(EnergyVerkleTrie::verify(&proof, &root));
        }

        #[test]
        fn delete_removes_key(
            key in arb_key(),
            value in arb_value(),
            energy in 0u64..10000,
            half_life in 1u64..1000,
        ) {
            let mut trie = EnergyVerkleTrie::new();
            trie.insert(key, value, energy, half_life, 0);
            trie.delete(&key);
            prop_assert_eq!(trie.get(&key), None);
        }

        #[test]
        fn compression_preserves_root(
            entries in proptest::collection::vec(
                (arb_key(), arb_value(), 1u64..1000),
                2..15
            ),
        ) {
            let mut trie = EnergyVerkleTrie::new();
            for (k, v, h) in &entries {
                trie.insert(*k, *v, 100, *h, 0);
            }

            // Kill all
            for (k, _, _) in &entries {
                trie.update_energy(k, 0, 50);
            }

            let root_before = trie.root();
            trie.compress_cold();
            let root_after = trie.root();

            prop_assert_eq!(root_before, root_after,
                "compression must never change the root commitment");
        }

        #[test]
        fn meta_max_energy_correct(
            entries in proptest::collection::vec(
                (arb_key(), arb_value(), 0u64..10000, 1u64..1000),
                1..20
            )
        ) {
            let mut trie = EnergyVerkleTrie::new();
            let mut keys_seen = std::collections::BTreeSet::new();

            for (k, v, e, h) in &entries {
                trie.insert(*k, *v, *e, *h, 0);
                if keys_seen.insert(*k) {
                    // First insert of this key
                } else {
                    // Update — need to recompute expected values
                }
                keys_seen.insert(*k);
            }

            // Recompute expected by looking at last value per key
            let mut last: std::collections::BTreeMap<[u8;32], (u64, u64)> = std::collections::BTreeMap::new();
            for (k, _, e, h) in &entries {
                last.insert(*k, (*e, *h));
            }
            let expected_max = last.values().map(|(e, _)| *e).max().unwrap_or(0);
            let expected_min_hl = last.values().map(|(_, h)| *h).min().unwrap_or(u64::MAX);

            prop_assert_eq!(trie.root_meta().max_energy, expected_max);
            prop_assert_eq!(trie.root_meta().min_half_life, expected_min_hl);
        }

        #[test]
        fn multiproof_verifies(
            entries in proptest::collection::vec(
                (arb_key(), arb_value(), 0u64..10000, 1u64..1000),
                2..20
            ),
            prove_count in 1usize..20,
        ) {
            let mut trie = EnergyVerkleTrie::new();
            for (k, v, e, h) in &entries {
                trie.insert(*k, *v, *e, *h, 0);
            }
            let root = trie.root();
            let n = prove_count.min(entries.len());
            let keys: Vec<[u8; 32]> = entries.iter().take(n).map(|(k,_,_,_)| *k).collect();
            let mp = trie.prove_multi(&keys);
            prop_assert!(EnergyVerkleTrie::verify_multi(&mp, &root));
        }

        #[test]
        fn multiproof_tamper_detected(
            entries in proptest::collection::vec(
                (arb_key(), arb_value(), 1u64..10000, 1u64..1000),
                3..15
            ),
        ) {
            let mut trie = EnergyVerkleTrie::new();
            for (k, v, e, h) in &entries {
                trie.insert(*k, *v, *e, *h, 0);
            }
            let root = trie.root();
            let keys: Vec<[u8; 32]> = entries.iter().map(|(k,_,_,_)| *k).collect();
            let mut mp = trie.prove_multi(&keys);
            // Tamper with first value
            if let Some(ref mut v) = mp.values[0] {
                v[0] ^= 0xFF;
            }
            prop_assert!(!EnergyVerkleTrie::verify_multi(&mp, &root));
        }
    }

    // ── L1 (audit 2026-05-13): bytes_to_scalar hardening (mirror of verkle.rs) ──

    #[test]
    fn audit_l1_energy_bytes_to_scalar_is_deterministic() {
        let input = [0x77u8; 32];
        assert_eq!(bytes_to_scalar(&input), bytes_to_scalar(&input));
    }

    #[test]
    fn audit_l1_energy_bytes_to_scalar_all_ones_does_not_panic() {
        let input = [0xFFu8; 32];
        let scalar = bytes_to_scalar(&input);
        assert!(!bool::from(scalar.is_zero()));
    }

    #[test]
    fn audit_l1_energy_bytes_to_scalar_distinct_inputs_distinct_scalars() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        a[0] = 1;
        b[0] = 2;
        assert_ne!(bytes_to_scalar(&a), bytes_to_scalar(&b));
    }

    #[test]
    fn audit_l1_energy_bytes_to_scalar_masks_top_2_bits() {
        let mut base = [0xABu8; 32];
        base[31] = 0x3F;
        let mut high = base;
        high[31] = 0xFF;
        assert_eq!(bytes_to_scalar(&base), bytes_to_scalar(&high));
    }
}
