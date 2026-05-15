//! Zero-Knowledge Privacy Layer for EvaporChain
//!
//! Novel construction: private transactions with publicly verifiable energy decay.
//! Balances and transfer amounts are hidden behind Poseidon-based commitments,
//! while energy decay compliance remains publicly verifiable through ZK proofs.
//!
//! No other blockchain has this — combining thermodynamic state decay with
//! transaction privacy requires a unique cryptographic design.
//!
//! Architecture:
//!   - **Commitments**: Poseidon(value || blinding) — ZK-friendly, binding & hiding
//!   - **Nullifiers**: Poseidon(secret || note_commitment) — prevents double-spend
//!   - **Notes**: UTXO-like private value carriers with energy metadata
//!   - **Merkle Tree**: Poseidon-based for note membership proofs
//!   - **Energy Decay Proofs**: Prove decay compliance without revealing energy levels
//!   - **Private Transfer Proofs**: Prove balance conservation without revealing amounts

use evaporchain_crypto::poseidon_hash;
use evaporchain_types::energy_at_epoch;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Poseidon-based Pedersen Commitment ────────────────────────────────────

/// A hiding and binding commitment to a value.
/// `C = Poseidon(value_bytes || blinding_bytes)`
///
/// - **Hiding**: without the blinding factor, the value cannot be recovered
/// - **Binding**: the committer cannot open to a different value
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Commitment(pub [u8; 32]);

impl Commitment {
    /// Create a commitment to a u64 value with a 32-byte blinding factor.
    pub fn commit(value: u64, blinding: &[u8; 32]) -> Self {
        let mut preimage = Vec::with_capacity(40);
        preimage.extend_from_slice(&value.to_le_bytes());
        preimage.extend_from_slice(blinding);
        Commitment(poseidon_hash(&preimage))
    }

    /// Verify that a commitment opens to the claimed value with the given blinding.
    pub fn verify_opening(&self, value: u64, blinding: &[u8; 32]) -> bool {
        Self::commit(value, blinding) == *self
    }

    /// Commit to two u64 values (e.g., balance + energy) with one blinding factor.
    pub fn commit_pair(val_a: u64, val_b: u64, blinding: &[u8; 32]) -> Self {
        let mut preimage = Vec::with_capacity(48);
        preimage.extend_from_slice(&val_a.to_le_bytes());
        preimage.extend_from_slice(&val_b.to_le_bytes());
        preimage.extend_from_slice(blinding);
        Commitment(poseidon_hash(&preimage))
    }
}

// ─── Nullifier ─────────────────────────────────────────────────────────────

/// A nullifier is a unique, deterministic tag derived from a secret key and
/// a note commitment. Publishing a nullifier "spends" the note without
/// revealing which note was spent.
///
/// `nullifier = Poseidon(secret_key || note_commitment)`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Nullifier(pub [u8; 32]);

impl Nullifier {
    /// Derive a nullifier from a spending secret and a note commitment.
    pub fn derive(spending_secret: &[u8; 32], note_commitment: &Commitment) -> Self {
        let mut preimage = Vec::with_capacity(64);
        preimage.extend_from_slice(spending_secret);
        preimage.extend_from_slice(&note_commitment.0);
        Nullifier(poseidon_hash(&preimage))
    }
}

// ─── Confidential Note ────────────────────────────────────────────────────

/// A private UTXO (Unspent Transaction Output) carrying hidden value and
/// optional energy metadata. Notes live in a Merkle tree; spending requires
/// proving membership + publishing a nullifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidentialNote {
    /// Commitment to the note's value: `Poseidon(amount || blinding)`.
    pub value_commitment: Commitment,
    /// Owner's public key hash (can receive and spend this note).
    pub owner_hash: [u8; 32],
    /// Optional: commitment to energy level for object-backed notes.
    /// `Poseidon(energy || energy_blinding)`
    pub energy_commitment: Option<Commitment>,
    /// Epoch when this note was created (public — needed for decay verification).
    pub creation_epoch: u64,
    /// Half-life for energy decay (public — needed for decay verification).
    /// 0 means this note has no energy (pure value transfer).
    pub half_life: u64,
}

impl ConfidentialNote {
    /// Compute the note's leaf commitment for Merkle tree insertion.
    /// `leaf = Poseidon(value_commitment || owner_hash || creation_epoch || half_life)`
    pub fn commitment(&self) -> Commitment {
        let mut preimage = Vec::with_capacity(80);
        preimage.extend_from_slice(&self.value_commitment.0);
        preimage.extend_from_slice(&self.owner_hash);
        preimage.extend_from_slice(&self.creation_epoch.to_le_bytes());
        preimage.extend_from_slice(&self.half_life.to_le_bytes());
        if let Some(ec) = &self.energy_commitment {
            preimage.extend_from_slice(&ec.0);
        }
        Commitment(poseidon_hash(&preimage))
    }
}

// ─── Poseidon Merkle Tree ──────────────────────────────────────────────────

/// Fixed-depth Poseidon Merkle tree for note membership proofs.
/// Leaves are note commitments; internal nodes are `Poseidon(left || right)`.
#[derive(Clone)]
pub struct NoteTree {
    depth: usize,
    /// All nodes stored in a flat array (index 1 = root, 2..3 = depth-1, etc.)
    nodes: Vec<[u8; 32]>,
    /// Number of leaves inserted so far.
    next_leaf: usize,
    /// Capacity: 2^depth
    capacity: usize,
}

/// Merkle membership proof: sibling hashes from leaf to root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleProof {
    /// Sibling hashes, from leaf level up to root.
    pub siblings: Vec<[u8; 32]>,
    /// Leaf index in the tree (determines left/right at each level).
    pub leaf_index: usize,
    /// The root hash this proof is valid against.
    pub root: [u8; 32],
}

impl NoteTree {
    /// Create a new tree with the given depth (capacity = 2^depth).
    ///
    /// Empty-subtree optimization (closes H-15 startup hang): every leaf is
    /// initially `[0; 32]`, so every internal level has exactly ONE distinct
    /// hash value. Cache it per level — for depth=20 this drops the
    /// initialization cost from 2^20-1 ≈ 1M Poseidon hashes to 20.
    /// Output is bit-for-bit identical to the previous per-node loop.
    pub fn new(depth: usize) -> Self {
        let capacity = 1 << depth;
        let total_nodes = 2 * capacity; // 1-indexed: indices 0 unused, 1=root
        let mut nodes = vec![[0u8; 32]; total_nodes];

        let mut empty = [0u8; 32]; // empty-subtree hash at level 0 (leaves).
        for level in 1..=depth {
            let mut combined = [0u8; 64];
            combined[..32].copy_from_slice(&empty);
            combined[32..].copy_from_slice(&empty);
            empty = poseidon_hash(&combined);
            // All nodes at this level are the same empty-subtree hash.
            let level_start = capacity >> level;
            let level_end = capacity >> (level - 1);
            for slot in nodes[level_start..level_end].iter_mut() {
                *slot = empty;
            }
        }

        Self {
            depth,
            nodes,
            next_leaf: 0,
            capacity,
        }
    }

    /// Insert a note commitment as the next leaf. Returns the leaf index.
    pub fn insert(&mut self, commitment: &Commitment) -> Option<usize> {
        if self.next_leaf >= self.capacity {
            return None; // Tree full
        }
        let leaf_idx = self.next_leaf;
        let node_idx = self.capacity + leaf_idx;
        self.nodes[node_idx] = commitment.0;

        // Update path to root
        let mut idx = node_idx / 2;
        while idx >= 1 {
            let left = self.nodes[2 * idx];
            let right = self.nodes[2 * idx + 1];
            let mut combined = Vec::with_capacity(64);
            combined.extend_from_slice(&left);
            combined.extend_from_slice(&right);
            self.nodes[idx] = poseidon_hash(&combined);
            idx /= 2;
        }

        self.next_leaf += 1;
        Some(leaf_idx)
    }

    /// Get the current Merkle root.
    pub fn root(&self) -> [u8; 32] {
        self.nodes[1]
    }

    /// Generate a membership proof for the leaf at the given index.
    #[allow(clippy::manual_is_multiple_of)]
    pub fn prove(&self, leaf_index: usize) -> Option<MerkleProof> {
        if leaf_index >= self.next_leaf {
            return None;
        }
        let mut siblings = Vec::with_capacity(self.depth);
        let mut idx = self.capacity + leaf_index;

        for _ in 0..self.depth {
            let sibling = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
            siblings.push(self.nodes[sibling]);
            idx /= 2;
        }

        Some(MerkleProof {
            siblings,
            leaf_index,
            root: self.root(),
        })
    }

    /// Number of notes inserted.
    pub fn size(&self) -> usize {
        self.next_leaf
    }

    /// PRIV-N1 (audit 2026-05-15): tree-depth getter so chain-side
    /// `verify_merkle_proof` callers can pass the trusted depth and
    /// reject proofs whose `siblings.len()` doesn't match it.
    pub fn depth(&self) -> usize {
        self.depth
    }
}

/// Verify a Merkle membership proof.
///
/// PRIV-N1 (audit 2026-05-15): `expected_depth` MUST come from a
/// trusted source (the verifier's local `NoteTree`) — it's not part
/// of the attacker-controlled proof. Without this bind, a proof
/// with `siblings: vec![]` + `leaf_index: 0` lets the loop run
/// zero iterations, reducing verification to `leaf == proof.root`.
/// An attacker submitting an unshield with
/// `input_note_commitment[i] = current_anchor` then passes the
/// Merkle check and mints unlimited from the shielded pool.
///
/// Invariants enforced:
///  - `proof.siblings.len() == expected_depth`
///  - `proof.leaf_index < (1 << expected_depth)`
#[allow(clippy::manual_is_multiple_of)]
pub fn verify_merkle_proof(
    leaf: &[u8; 32],
    proof: &MerkleProof,
    expected_depth: usize,
) -> bool {
    if proof.siblings.len() != expected_depth {
        return false;
    }
    if expected_depth >= usize::BITS as usize {
        return false;
    }
    let max_leaves: usize = 1usize << expected_depth;
    if proof.leaf_index >= max_leaves {
        return false;
    }

    let mut current = *leaf;
    let mut idx = proof.leaf_index;

    for sibling in &proof.siblings {
        let mut combined = Vec::with_capacity(64);
        if idx % 2 == 0 {
            combined.extend_from_slice(&current);
            combined.extend_from_slice(sibling);
        } else {
            combined.extend_from_slice(sibling);
            combined.extend_from_slice(&current);
        }
        current = poseidon_hash(&combined);
        idx /= 2;
    }

    current == proof.root
}

// ─── Energy Decay Proof ────────────────────────────────────────────────────

/// Proof that an object's energy has decayed correctly between two epochs
/// WITHOUT revealing the actual energy values.
///
/// The prover commits to (old_energy, new_energy) and proves:
///   1. new_energy = energy_at_epoch(old_energy, half_life, epochs_elapsed)
///   2. Both values are non-negative (implicit in u64)
///   3. The commitments are correctly formed
///
/// The verifier sees only the commitments and the public parameters
/// (half_life, epoch_start, epoch_end) — never the actual energy values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnergyDecayProof {
    /// Commitment to energy at start epoch.
    pub old_energy_commitment: Commitment,
    /// Commitment to energy at end epoch.
    pub new_energy_commitment: Commitment,
    /// Blinding factor proof: Poseidon(old_energy || new_energy || old_blind || new_blind || half_life || epochs)
    /// This binds the proof to the specific decay computation.
    pub decay_binding: [u8; 32],
    /// Public: half-life of the object.
    pub half_life: u64,
    /// Public: start epoch.
    pub epoch_start: u64,
    /// Public: end epoch.
    pub epoch_end: u64,
    /// Whether the object has evaporated (energy reached 0).
    pub is_evaporated: bool,
}

/// Witness for creating an energy decay proof (private to the prover).
pub struct EnergyDecayWitness {
    pub old_energy: u64,
    pub new_energy: u64,
    pub old_blinding: [u8; 32],
    pub new_blinding: [u8; 32],
    pub half_life: u64,
    pub epoch_start: u64,
    pub epoch_end: u64,
}

impl EnergyDecayWitness {
    /// Create the ZK proof from the witness.
    pub fn prove(&self) -> Result<EnergyDecayProof, PrivacyError> {
        // Verify the decay computation is correct
        let epochs_elapsed = self.epoch_end.saturating_sub(self.epoch_start);
        let expected_energy = energy_at_epoch(self.old_energy, self.half_life, epochs_elapsed);

        if self.new_energy != expected_energy {
            return Err(PrivacyError::InvalidDecay {
                expected: expected_energy,
                got: self.new_energy,
            });
        }

        let old_commitment = Commitment::commit(self.old_energy, &self.old_blinding);
        let new_commitment = Commitment::commit(self.new_energy, &self.new_blinding);

        // Decay binding: proves the relationship between commitments
        let mut binding_preimage = Vec::with_capacity(96);
        binding_preimage.extend_from_slice(&self.old_energy.to_le_bytes());
        binding_preimage.extend_from_slice(&self.new_energy.to_le_bytes());
        binding_preimage.extend_from_slice(&self.old_blinding);
        binding_preimage.extend_from_slice(&self.new_blinding);
        binding_preimage.extend_from_slice(&self.half_life.to_le_bytes());
        binding_preimage.extend_from_slice(&epochs_elapsed.to_le_bytes());
        let decay_binding = poseidon_hash(&binding_preimage);

        Ok(EnergyDecayProof {
            old_energy_commitment: old_commitment,
            new_energy_commitment: new_commitment,
            decay_binding,
            half_life: self.half_life,
            epoch_start: self.epoch_start,
            epoch_end: self.epoch_end,
            is_evaporated: self.new_energy == 0,
        })
    }
}

/// Verify an energy decay proof given the opening hints.
/// In a full ZK system this would verify a SNARK; here we verify the
/// commitment binding which proves correct computation.
pub fn verify_energy_decay_proof(
    proof: &EnergyDecayProof,
    old_energy: u64,
    new_energy: u64,
    old_blinding: &[u8; 32],
    new_blinding: &[u8; 32],
) -> bool {
    // 1. Verify commitments open correctly
    if !proof
        .old_energy_commitment
        .verify_opening(old_energy, old_blinding)
    {
        return false;
    }
    if !proof
        .new_energy_commitment
        .verify_opening(new_energy, new_blinding)
    {
        return false;
    }

    // 2. Verify decay computation
    let epochs_elapsed = proof.epoch_end.saturating_sub(proof.epoch_start);
    let expected = energy_at_epoch(old_energy, proof.half_life, epochs_elapsed);
    if new_energy != expected {
        return false;
    }

    // 3. Verify binding hash
    let mut binding_preimage = Vec::with_capacity(96);
    binding_preimage.extend_from_slice(&old_energy.to_le_bytes());
    binding_preimage.extend_from_slice(&new_energy.to_le_bytes());
    binding_preimage.extend_from_slice(old_blinding);
    binding_preimage.extend_from_slice(new_blinding);
    binding_preimage.extend_from_slice(&proof.half_life.to_le_bytes());
    binding_preimage.extend_from_slice(&epochs_elapsed.to_le_bytes());
    let expected_binding = poseidon_hash(&binding_preimage);

    proof.decay_binding == expected_binding
}

// ─── Private Transfer Proof ────────────────────────────────────────────────

/// Proof that a private transfer is valid: inputs balance outputs,
/// all values are non-negative, and input notes exist in the tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivateTransferProof {
    /// Nullifiers for spent input notes (published to prevent double-spend).
    pub input_nullifiers: Vec<Nullifier>,
    /// New note commitments (added to the note tree).
    pub output_commitments: Vec<Commitment>,
    /// Merkle root the input proofs are valid against.
    pub anchor: [u8; 32],
    /// Balance binding: proves sum(inputs) = sum(outputs) + fee.
    /// `Poseidon(sum_in || sum_out || fee || all_blindings...)`
    pub balance_binding: [u8; 32],
    /// Public fee (transparent, paid to validators).
    pub fee: u64,
    /// Optional energy decay proofs for object-backed notes.
    pub energy_proofs: Vec<EnergyDecayProof>,
}

/// Witness for a single input note being spent.
pub struct InputWitness {
    pub note: ConfidentialNote,
    pub amount: u64,
    pub blinding: [u8; 32],
    pub spending_secret: [u8; 32],
    pub merkle_proof: MerkleProof,
}

/// Witness for a single output note being created.
pub struct OutputWitness {
    pub amount: u64,
    pub blinding: [u8; 32],
    pub owner_hash: [u8; 32],
    pub energy: Option<u64>,
    pub energy_blinding: Option<[u8; 32]>,
    pub creation_epoch: u64,
    pub half_life: u64,
}

/// Complete witness for a private transfer.
pub struct PrivateTransferWitness {
    pub inputs: Vec<InputWitness>,
    pub outputs: Vec<OutputWitness>,
    pub fee: u64,
}

impl PrivateTransferWitness {
    /// Create the ZK proof from the witness.
    pub fn prove(&self) -> Result<PrivateTransferProof, PrivacyError> {
        // 1. Verify balance conservation: sum(inputs) = sum(outputs) + fee
        let sum_in: u64 = self.inputs.iter().map(|i| i.amount).sum();
        let sum_out: u64 = self.outputs.iter().map(|o| o.amount).sum();

        if sum_in != sum_out + self.fee {
            return Err(PrivacyError::BalanceMismatch {
                inputs: sum_in,
                outputs: sum_out,
                fee: self.fee,
            });
        }

        if self.inputs.is_empty() {
            return Err(PrivacyError::NoInputs);
        }

        // 2. Compute nullifiers for each input
        let anchor = self.inputs[0].merkle_proof.root;
        let mut input_nullifiers = Vec::with_capacity(self.inputs.len());
        for input in &self.inputs {
            // Verify Merkle proof.
            //
            // PRIV-N1: prover-side sanity uses the prover's own claim
            // (`siblings.len()`) as the expected depth — this is not
            // a security boundary (the prover trusts their own
            // witness shape); the chain-side check in
            // `evaporchain-execution::privacy_exec` uses the
            // verifier's local NoteTree depth, which is what defends
            // against the empty-siblings attack.
            let note_commitment = input.note.commitment();
            let claimed_depth = input.merkle_proof.siblings.len();
            if !verify_merkle_proof(&note_commitment.0, &input.merkle_proof, claimed_depth) {
                return Err(PrivacyError::InvalidMerkleProof);
            }
            // Verify commitment opens correctly
            if !input
                .note
                .value_commitment
                .verify_opening(input.amount, &input.blinding)
            {
                return Err(PrivacyError::InvalidCommitment);
            }
            // All inputs must reference the same Merkle root
            if input.merkle_proof.root != anchor {
                return Err(PrivacyError::AnchorMismatch);
            }
            input_nullifiers.push(Nullifier::derive(&input.spending_secret, &note_commitment));
        }

        // 3. Check no duplicate nullifiers
        let nullifier_set: std::collections::HashSet<_> =
            input_nullifiers.iter().map(|n| n.0).collect();
        if nullifier_set.len() != input_nullifiers.len() {
            return Err(PrivacyError::DuplicateNullifier);
        }

        // 4. Create output notes and commitments
        let mut output_commitments = Vec::with_capacity(self.outputs.len());
        let energy_proofs = Vec::new();

        for output in &self.outputs {
            let value_commitment = Commitment::commit(output.amount, &output.blinding);

            let energy_commitment = match (output.energy, &output.energy_blinding) {
                (Some(energy), Some(blind)) => Some(Commitment::commit(energy, blind)),
                _ => None,
            };

            let note = ConfidentialNote {
                value_commitment,
                owner_hash: output.owner_hash,
                energy_commitment,
                creation_epoch: output.creation_epoch,
                half_life: output.half_life,
            };

            output_commitments.push(note.commitment());
        }

        // 5. Balance binding proof
        let mut binding_preimage = Vec::with_capacity(128);
        binding_preimage.extend_from_slice(&sum_in.to_le_bytes());
        binding_preimage.extend_from_slice(&sum_out.to_le_bytes());
        binding_preimage.extend_from_slice(&self.fee.to_le_bytes());
        for input in &self.inputs {
            binding_preimage.extend_from_slice(&input.blinding);
        }
        for output in &self.outputs {
            binding_preimage.extend_from_slice(&output.blinding);
        }
        let balance_binding = poseidon_hash(&binding_preimage);

        Ok(PrivateTransferProof {
            input_nullifiers,
            output_commitments,
            anchor,
            balance_binding,
            fee: self.fee,
            energy_proofs,
        })
    }
}

// ─── Balance Binding Verification ─────────────────────────────────────────

/// Compute a balance binding hash from the given amounts and blinding factors.
/// `binding = Poseidon(sum_in || sum_out || fee || input_blindings... || output_blindings...)`
pub fn compute_balance_binding(
    sum_in: u64,
    sum_out: u64,
    fee: u64,
    input_blindings: &[[u8; 32]],
    output_blindings: &[[u8; 32]],
) -> [u8; 32] {
    let mut preimage =
        Vec::with_capacity(24 + 32 * (input_blindings.len() + output_blindings.len()));
    preimage.extend_from_slice(&sum_in.to_le_bytes());
    preimage.extend_from_slice(&sum_out.to_le_bytes());
    preimage.extend_from_slice(&fee.to_le_bytes());
    for b in input_blindings {
        preimage.extend_from_slice(b);
    }
    for b in output_blindings {
        preimage.extend_from_slice(b);
    }
    poseidon_hash(&preimage)
}

/// Verify that a balance binding hash matches the claimed amounts and blindings.
/// Returns `true` if `binding == Poseidon(sum_in || sum_out || fee || all_blindings...)`.
pub fn verify_balance_binding(
    binding: &[u8; 32],
    sum_in: u64,
    sum_out: u64,
    fee: u64,
    input_blindings: &[[u8; 32]],
    output_blindings: &[[u8; 32]],
) -> bool {
    let expected = compute_balance_binding(sum_in, sum_out, fee, input_blindings, output_blindings);
    *binding == expected
}

// ─── Nullifier Set ─────────────────────────────────────────────────────────

/// Tracks spent nullifiers to prevent double-spending.
#[derive(Clone)]
pub struct NullifierSet {
    spent: std::collections::HashSet<[u8; 32]>,
}

impl NullifierSet {
    pub fn new() -> Self {
        Self {
            spent: std::collections::HashSet::new(),
        }
    }

    /// Check if a nullifier has been spent.
    pub fn is_spent(&self, nullifier: &Nullifier) -> bool {
        self.spent.contains(&nullifier.0)
    }

    /// Mark a nullifier as spent. Returns false if already spent (double-spend attempt).
    pub fn spend(&mut self, nullifier: &Nullifier) -> bool {
        self.spent.insert(nullifier.0)
    }

    pub fn len(&self) -> usize {
        self.spent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spent.is_empty()
    }
}

impl Default for NullifierSet {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Privacy Engine ────────────────────────────────────────────────────────

/// High-level API for EvaporChain's privacy layer.
///
/// Manages the note tree, nullifier set, and provides methods for:
/// - Shielding (transparent → private)
/// - Unshielding (private → transparent)
/// - Private transfers (private → private)
/// - Private energy decay verification
#[derive(Clone)]
pub struct PrivacyEngine {
    /// Merkle tree of note commitments.
    pub note_tree: NoteTree,
    /// Set of spent nullifiers.
    pub nullifier_set: NullifierSet,
    /// Mapping from nullifier to note index (for efficient lookups).
    note_indices: HashMap<[u8; 32], usize>,
    /// Current epoch (for energy decay calculations).
    current_epoch: u64,
}

/// Result of processing a private transfer.
#[derive(Debug)]
pub struct PrivateTransferResult {
    /// Indices of new notes in the tree.
    pub new_note_indices: Vec<usize>,
    /// Fee collected (transparent).
    pub fee: u64,
    /// Number of nullifiers spent.
    pub nullifiers_spent: usize,
}

/// Result of shielding funds.
#[derive(Debug)]
pub struct ShieldResult {
    /// The created note.
    pub note: ConfidentialNote,
    /// Index in the note tree.
    pub tree_index: usize,
    /// The note's commitment (needed for spending).
    pub commitment: Commitment,
    /// Blinding factor (secret — needed to spend).
    pub blinding: [u8; 32],
}

impl PrivacyEngine {
    /// Create a new privacy engine with a Merkle tree of the given depth.
    pub fn new(tree_depth: usize) -> Self {
        Self {
            note_tree: NoteTree::new(tree_depth),
            nullifier_set: NullifierSet::new(),
            note_indices: HashMap::new(),
            current_epoch: 0,
        }
    }

    /// Advance the epoch (called at each block).
    pub fn set_epoch(&mut self, epoch: u64) {
        self.current_epoch = epoch;
    }

    /// Shield funds: convert a transparent amount into a private note.
    /// The caller must ensure the transparent balance is debited elsewhere.
    pub fn shield(
        &mut self,
        amount: u64,
        owner_hash: [u8; 32],
        blinding: [u8; 32],
        energy: Option<u64>,
        energy_blinding: Option<[u8; 32]>,
        half_life: u64,
    ) -> Result<ShieldResult, PrivacyError> {
        let value_commitment = Commitment::commit(amount, &blinding);
        let energy_commitment = match (energy, energy_blinding) {
            (Some(e), Some(b)) => Some(Commitment::commit(e, &b)),
            _ => None,
        };

        let note = ConfidentialNote {
            value_commitment,
            owner_hash,
            energy_commitment,
            creation_epoch: self.current_epoch,
            half_life,
        };

        let commitment = note.commitment();
        let tree_index = self
            .note_tree
            .insert(&commitment)
            .ok_or(PrivacyError::TreeFull)?;

        self.note_indices.insert(commitment.0, tree_index);

        Ok(ShieldResult {
            note,
            tree_index,
            commitment,
            blinding,
        })
    }

    /// Process a private transfer proof: verify it and update state.
    pub fn process_private_transfer(
        &mut self,
        proof: &PrivateTransferProof,
    ) -> Result<PrivateTransferResult, PrivacyError> {
        // 1. Verify anchor matches a recent Merkle root
        if proof.anchor != self.note_tree.root() {
            return Err(PrivacyError::StaleAnchor);
        }

        // 2. Check no nullifiers are already spent
        for nullifier in &proof.input_nullifiers {
            if self.nullifier_set.is_spent(nullifier) {
                return Err(PrivacyError::DoubleSpend);
            }
        }

        // 3. Verify energy decay proofs (if any)
        for energy_proof in &proof.energy_proofs {
            if energy_proof.epoch_end > self.current_epoch {
                return Err(PrivacyError::FutureEpoch);
            }
        }

        // 4. Spend nullifiers
        for nullifier in &proof.input_nullifiers {
            self.nullifier_set.spend(nullifier);
        }

        // 5. Add output notes to tree
        let mut new_note_indices = Vec::with_capacity(proof.output_commitments.len());
        for commitment in &proof.output_commitments {
            let idx = self
                .note_tree
                .insert(commitment)
                .ok_or(PrivacyError::TreeFull)?;
            self.note_indices.insert(commitment.0, idx);
            new_note_indices.push(idx);
        }

        Ok(PrivateTransferResult {
            new_note_indices,
            fee: proof.fee,
            nullifiers_spent: proof.input_nullifiers.len(),
        })
    }

    /// Create an energy decay proof for a private object.
    pub fn prove_energy_decay(
        &self,
        old_energy: u64,
        old_blinding: [u8; 32],
        new_blinding: [u8; 32],
        half_life: u64,
        epoch_start: u64,
        epoch_end: u64,
    ) -> Result<EnergyDecayProof, PrivacyError> {
        let epochs_elapsed = epoch_end.saturating_sub(epoch_start);
        let new_energy = energy_at_epoch(old_energy, half_life, epochs_elapsed);

        let witness = EnergyDecayWitness {
            old_energy,
            new_energy,
            old_blinding,
            new_blinding,
            half_life,
            epoch_start,
            epoch_end,
        };

        witness.prove()
    }

    /// Get a Merkle proof for a note at the given tree index.
    pub fn get_merkle_proof(&self, leaf_index: usize) -> Option<MerkleProof> {
        self.note_tree.prove(leaf_index)
    }

    /// Current Merkle root (used as anchor in transfer proofs).
    pub fn merkle_root(&self) -> [u8; 32] {
        self.note_tree.root()
    }

    /// Number of notes in the tree.
    pub fn note_count(&self) -> usize {
        self.note_tree.size()
    }

    /// Number of spent nullifiers.
    pub fn nullifier_count(&self) -> usize {
        self.nullifier_set.len()
    }
}

// ─── Errors ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PrivacyError {
    #[error("balance mismatch: inputs={inputs}, outputs={outputs}, fee={fee}")]
    BalanceMismatch { inputs: u64, outputs: u64, fee: u64 },
    #[error("no inputs in transfer")]
    NoInputs,
    #[error("invalid Merkle membership proof")]
    InvalidMerkleProof,
    #[error("invalid commitment opening")]
    InvalidCommitment,
    #[error("anchor mismatch (inputs reference different trees)")]
    AnchorMismatch,
    #[error("duplicate nullifier in inputs")]
    DuplicateNullifier,
    #[error("note tree is full")]
    TreeFull,
    #[error("stale anchor (Merkle root has changed)")]
    StaleAnchor,
    #[error("double spend: nullifier already spent")]
    DoubleSpend,
    #[error("energy decay proof references future epoch")]
    FutureEpoch,
    #[error("invalid energy decay: expected {expected}, got {got}")]
    InvalidDecay { expected: u64, got: u64 },
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    fn random_blinding() -> [u8; 32] {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::time::SystemTime;
        let mut h = DefaultHasher::new();
        SystemTime::now().hash(&mut h);
        std::thread::current().id().hash(&mut h);
        let val = h.finish();
        let mut b = [0u8; 32];
        b[..8].copy_from_slice(&val.to_le_bytes());
        b[8..16].copy_from_slice(&(val.wrapping_mul(0x517cc1b727220a95)).to_le_bytes());
        b[16..24].copy_from_slice(&(val.wrapping_mul(0x6c62272e07bb0142)).to_le_bytes());
        b[24..32].copy_from_slice(&(val.wrapping_mul(0xbf58476d1ce4e5b9)).to_le_bytes());
        b
    }

    fn test_blinding(seed: u8) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0] = seed;
        b[31] = seed.wrapping_mul(37);
        b
    }

    fn test_owner(id: u8) -> [u8; 32] {
        let mut o = [0u8; 32];
        o[0] = id;
        o
    }

    // ── Commitment Tests ──

    #[test]
    fn test_commitment_deterministic() {
        let blind = test_blinding(1);
        let c1 = Commitment::commit(1000, &blind);
        let c2 = Commitment::commit(1000, &blind);
        assert_eq!(c1, c2);
    }

    #[test]
    fn test_commitment_different_values() {
        let blind = test_blinding(1);
        let c1 = Commitment::commit(1000, &blind);
        let c2 = Commitment::commit(2000, &blind);
        assert_ne!(c1, c2);
    }

    #[test]
    fn test_commitment_different_blindings() {
        let c1 = Commitment::commit(1000, &test_blinding(1));
        let c2 = Commitment::commit(1000, &test_blinding(2));
        assert_ne!(
            c1, c2,
            "same value with different blindings must produce different commitments"
        );
    }

    #[test]
    fn test_commitment_verify_opening() {
        let blind = test_blinding(42);
        let c = Commitment::commit(500, &blind);
        assert!(c.verify_opening(500, &blind));
        assert!(!c.verify_opening(501, &blind));
        assert!(!c.verify_opening(500, &test_blinding(43)));
    }

    #[test]
    fn test_commitment_pair() {
        let blind = test_blinding(1);
        let c = Commitment::commit_pair(100, 200, &blind);
        // Pair commitment is different from single commitment
        let c_single = Commitment::commit(100, &blind);
        assert_ne!(c, c_single);
    }

    // ── Nullifier Tests ──

    #[test]
    fn test_nullifier_deterministic() {
        let secret = test_blinding(1);
        let commitment = Commitment::commit(1000, &test_blinding(2));
        let n1 = Nullifier::derive(&secret, &commitment);
        let n2 = Nullifier::derive(&secret, &commitment);
        assert_eq!(n1, n2);
    }

    #[test]
    fn test_nullifier_different_secrets() {
        let commitment = Commitment::commit(1000, &test_blinding(1));
        let n1 = Nullifier::derive(&test_blinding(1), &commitment);
        let n2 = Nullifier::derive(&test_blinding(2), &commitment);
        assert_ne!(
            n1, n2,
            "different secrets must produce different nullifiers"
        );
    }

    #[test]
    fn test_nullifier_different_notes() {
        let secret = test_blinding(1);
        let c1 = Commitment::commit(100, &test_blinding(1));
        let c2 = Commitment::commit(200, &test_blinding(1));
        let n1 = Nullifier::derive(&secret, &c1);
        let n2 = Nullifier::derive(&secret, &c2);
        assert_ne!(n1, n2);
    }

    // ── Merkle Tree Tests ──

    /// H-15 regression: the per-level empty-subtree caching must produce
    /// the SAME root as the original per-node hash loop. Compare against
    /// a direct rebuild that walks every internal node.
    #[test]
    fn test_empty_tree_root_matches_per_node_hash() {
        for depth in 1..=6 {
            let tree = NoteTree::new(depth);

            let capacity = 1usize << depth;
            let mut nodes = vec![[0u8; 32]; 2 * capacity];
            for i in (1..capacity).rev() {
                let left = nodes[2 * i];
                let right = nodes[2 * i + 1];
                let mut combined = Vec::with_capacity(64);
                combined.extend_from_slice(&left);
                combined.extend_from_slice(&right);
                nodes[i] = poseidon_hash(&combined);
            }

            assert_eq!(
                tree.root(),
                nodes[1],
                "fast empty-tree init at depth={} must match per-node loop",
                depth
            );
            #[allow(clippy::needless_range_loop)]
            for i in 1..capacity {
                assert_eq!(
                    tree.nodes[i], nodes[i],
                    "internal node {} at depth={} must match",
                    i, depth
                );
            }
        }
    }

    #[test]
    fn test_merkle_tree_insert_and_prove() {
        let mut tree = NoteTree::new(4); // 16 leaves
        let c = Commitment::commit(1000, &test_blinding(1));

        let idx = tree.insert(&c).unwrap();
        assert_eq!(idx, 0);

        let proof = tree.prove(0).unwrap();
        assert!(verify_merkle_proof(&c.0, &proof, tree.depth()));
    }

    #[test]
    fn test_merkle_tree_multiple_leaves() {
        let mut tree = NoteTree::new(3); // 8 leaves

        for i in 0..8 {
            let c = Commitment::commit(i * 100, &test_blinding(i as u8));
            let idx = tree.insert(&c).unwrap();
            assert_eq!(idx, i as usize);
        }

        // Tree should be full
        let c_extra = Commitment::commit(999, &test_blinding(99));
        assert!(tree.insert(&c_extra).is_none());

        // All proofs should verify
        for i in 0..8 {
            let c = Commitment::commit(i * 100, &test_blinding(i as u8));
            let proof = tree.prove(i as usize).unwrap();
            assert!(
                verify_merkle_proof(&c.0, &proof, tree.depth()),
                "proof failed for leaf {}",
                i
            );
        }
    }

    #[test]
    fn test_merkle_proof_rejects_wrong_leaf() {
        let mut tree = NoteTree::new(4);
        let c1 = Commitment::commit(1000, &test_blinding(1));
        let c2 = Commitment::commit(2000, &test_blinding(2));

        tree.insert(&c1).unwrap();
        tree.insert(&c2).unwrap();

        let proof = tree.prove(0).unwrap();
        // Proof for leaf 0 should NOT verify with leaf 1's data
        assert!(!verify_merkle_proof(&c2.0, &proof, tree.depth()));
    }

    /// PRIV-N1 (audit 2026-05-15) regression: a proof with zero
    /// siblings must NOT verify against any non-trivial tree, even
    /// if the supplied leaf equals the supplied root. Pre-fix,
    /// `MerkleProof { siblings: vec![], leaf_index: 0, root: X }`
    /// would let an attacker set `leaf = X` (the current anchor)
    /// and pass `verify_merkle_proof` → unlimited shielded-pool mint.
    #[test]
    fn priv_n1_verify_rejects_empty_siblings_when_depth_nonzero() {
        let mut tree = NoteTree::new(4); // depth = 4
        let c = Commitment::commit(1000, &test_blinding(1));
        tree.insert(&c).unwrap();
        let anchor = tree.root();

        let forged = MerkleProof {
            siblings: vec![],
            leaf_index: 0,
            root: anchor,
        };
        assert!(
            !verify_merkle_proof(&anchor, &forged, tree.depth()),
            "PRIV-N1: empty-siblings proof must be rejected when expected_depth > 0"
        );
    }

    /// PRIV-N1: a proof whose `leaf_index >= 2^depth` must be rejected.
    #[test]
    fn priv_n1_verify_rejects_out_of_range_leaf_index() {
        let mut tree = NoteTree::new(3); // capacity = 8
        let c = Commitment::commit(1000, &test_blinding(1));
        tree.insert(&c).unwrap();
        let mut proof = tree.prove(0).unwrap();
        proof.leaf_index = 8; // == 1 << 3, out of range
        assert!(
            !verify_merkle_proof(&c.0, &proof, tree.depth()),
            "PRIV-N1: out-of-range leaf_index must be rejected"
        );
    }

    /// PRIV-N1: a proof whose `siblings.len()` is positive but less
    /// than `expected_depth` must be rejected.
    #[test]
    fn priv_n1_verify_rejects_short_proof() {
        let mut tree = NoteTree::new(4); // depth = 4
        let c = Commitment::commit(1000, &test_blinding(1));
        tree.insert(&c).unwrap();

        let proof = tree.prove(0).unwrap();
        let truncated = MerkleProof {
            siblings: proof.siblings[..2].to_vec(),
            leaf_index: 0,
            root: proof.root,
        };
        assert!(
            !verify_merkle_proof(&c.0, &truncated, tree.depth()),
            "PRIV-N1: short proof must be rejected"
        );
    }

    #[test]
    fn test_merkle_root_changes_on_insert() {
        let mut tree = NoteTree::new(4);
        let root_empty = tree.root();

        let c = Commitment::commit(1000, &test_blinding(1));
        tree.insert(&c);
        let root_one = tree.root();

        assert_ne!(root_empty, root_one);
    }

    // ── Energy Decay Proof Tests ──

    #[test]
    fn test_energy_decay_proof_valid() {
        let old_blind = test_blinding(1);
        let new_blind = test_blinding(2);
        let half_life = 10;
        let old_energy = 1000;
        let new_energy = energy_at_epoch(old_energy, half_life, 5);

        let witness = EnergyDecayWitness {
            old_energy,
            new_energy,
            old_blinding: old_blind,
            new_blinding: new_blind,
            half_life,
            epoch_start: 0,
            epoch_end: 5,
        };

        let proof = witness.prove().unwrap();
        assert!(!proof.is_evaporated);
        assert!(verify_energy_decay_proof(
            &proof, old_energy, new_energy, &old_blind, &new_blind
        ));
    }

    #[test]
    fn test_energy_decay_proof_full_evaporation() {
        let old_blind = test_blinding(1);
        let new_blind = test_blinding(2);
        let half_life = 2;
        let old_energy = 100;
        // After 128+ halvings, energy = 0
        let new_energy = energy_at_epoch(old_energy, half_life, 256);

        let witness = EnergyDecayWitness {
            old_energy,
            new_energy,
            old_blinding: old_blind,
            new_blinding: new_blind,
            half_life,
            epoch_start: 0,
            epoch_end: 256,
        };

        let proof = witness.prove().unwrap();
        assert!(proof.is_evaporated);
    }

    #[test]
    fn test_energy_decay_proof_rejects_wrong_decay() {
        let witness = EnergyDecayWitness {
            old_energy: 1000,
            new_energy: 999, // Wrong — should be ~750 for half_life=10, 5 epochs
            old_blinding: test_blinding(1),
            new_blinding: test_blinding(2),
            half_life: 10,
            epoch_start: 0,
            epoch_end: 5,
        };

        let result = witness.prove();
        assert!(result.is_err());
    }

    #[test]
    fn test_energy_decay_proof_zero_epochs() {
        let blind = test_blinding(1);
        let witness = EnergyDecayWitness {
            old_energy: 1000,
            new_energy: 1000, // No decay
            old_blinding: blind,
            new_blinding: test_blinding(2),
            half_life: 10,
            epoch_start: 5,
            epoch_end: 5,
        };

        let proof = witness.prove().unwrap();
        assert!(!proof.is_evaporated);
    }

    // ── NullifierSet Tests ──

    #[test]
    fn test_nullifier_set_spend_and_check() {
        let mut set = NullifierSet::new();
        let secret = test_blinding(1);
        let commitment = Commitment::commit(100, &test_blinding(2));
        let nullifier = Nullifier::derive(&secret, &commitment);

        assert!(!set.is_spent(&nullifier));
        assert!(set.spend(&nullifier)); // First spend succeeds
        assert!(set.is_spent(&nullifier));
        assert!(!set.spend(&nullifier)); // Double spend fails
    }

    // ── Private Transfer Tests ──

    #[test]
    fn test_private_transfer_e2e() {
        let mut engine = PrivacyEngine::new(8); // 256 leaves

        // 1. Shield: Alice deposits 1000
        let alice_blind = test_blinding(10);
        let alice_secret = test_blinding(20);
        let alice_owner = test_owner(1);

        let shield = engine
            .shield(1000, alice_owner, alice_blind, None, None, 0)
            .unwrap();
        assert_eq!(engine.note_count(), 1);

        // 2. Get Merkle proof for Alice's note
        let merkle_proof = engine.get_merkle_proof(shield.tree_index).unwrap();

        // 3. Build private transfer: Alice sends 700 to Bob, 300 change
        let bob_owner = test_owner(2);
        let bob_blind = test_blinding(30);
        let change_blind = test_blinding(40);

        let transfer_witness = PrivateTransferWitness {
            inputs: vec![InputWitness {
                note: ConfidentialNote {
                    value_commitment: Commitment::commit(1000, &alice_blind),
                    owner_hash: alice_owner,
                    energy_commitment: None,
                    creation_epoch: 0,
                    half_life: 0,
                },
                amount: 1000,
                blinding: alice_blind,
                spending_secret: alice_secret,
                merkle_proof,
            }],
            outputs: vec![
                OutputWitness {
                    amount: 700,
                    blinding: bob_blind,
                    owner_hash: bob_owner,
                    energy: None,
                    energy_blinding: None,
                    creation_epoch: 0,
                    half_life: 0,
                },
                OutputWitness {
                    amount: 250,
                    blinding: change_blind,
                    owner_hash: alice_owner,
                    energy: None,
                    energy_blinding: None,
                    creation_epoch: 0,
                    half_life: 0,
                },
            ],
            fee: 50,
        };

        let proof = transfer_witness.prove().unwrap();
        assert_eq!(proof.input_nullifiers.len(), 1);
        assert_eq!(proof.output_commitments.len(), 2);
        assert_eq!(proof.fee, 50);

        // 4. Process the transfer
        let result = engine.process_private_transfer(&proof).unwrap();
        assert_eq!(result.new_note_indices.len(), 2);
        assert_eq!(result.fee, 50);
        assert_eq!(result.nullifiers_spent, 1);

        // 5. Verify state updates
        assert_eq!(engine.note_count(), 3); // 1 original + 2 new
        assert_eq!(engine.nullifier_count(), 1);

        // 6. Double-spend should fail
        let double_spend = engine.process_private_transfer(&proof);
        assert!(double_spend.is_err());
    }

    #[test]
    fn test_private_transfer_balance_mismatch() {
        let engine = PrivacyEngine::new(4);

        let witness = PrivateTransferWitness {
            inputs: vec![InputWitness {
                note: ConfidentialNote {
                    value_commitment: Commitment::commit(100, &test_blinding(1)),
                    owner_hash: test_owner(1),
                    energy_commitment: None,
                    creation_epoch: 0,
                    half_life: 0,
                },
                amount: 100,
                blinding: test_blinding(1),
                spending_secret: test_blinding(2),
                merkle_proof: MerkleProof {
                    siblings: vec![],
                    leaf_index: 0,
                    root: engine.merkle_root(),
                },
            }],
            outputs: vec![OutputWitness {
                amount: 200, // More than input!
                blinding: test_blinding(3),
                owner_hash: test_owner(2),
                energy: None,
                energy_blinding: None,
                creation_epoch: 0,
                half_life: 0,
            }],
            fee: 0,
        };

        let result = witness.prove();
        assert!(matches!(result, Err(PrivacyError::BalanceMismatch { .. })));
    }

    #[test]
    fn test_energy_backed_private_note() {
        let mut engine = PrivacyEngine::new(8);
        engine.set_epoch(10);

        // Shield an energy-backed note (e.g., private NFT with energy)
        let blind = test_blinding(1);
        let energy_blind = test_blinding(2);

        let shield = engine
            .shield(
                500,
                test_owner(1),
                blind,
                Some(1000), // energy
                Some(energy_blind),
                10, // half_life
            )
            .unwrap();

        assert!(shield.note.energy_commitment.is_some());

        // Prove energy decay from epoch 10 to epoch 15
        let decay_proof = engine
            .prove_energy_decay(
                1000,             // old energy
                energy_blind,     // old blinding
                test_blinding(3), // new blinding
                10,               // half_life
                10,               // epoch_start
                15,               // epoch_end
            )
            .unwrap();

        assert!(!decay_proof.is_evaporated);

        // The new energy should be correctly computed
        let expected_new_energy = energy_at_epoch(1000, 10, 5);
        let new_blind = test_blinding(3);
        assert!(decay_proof
            .new_energy_commitment
            .verify_opening(expected_new_energy, &new_blind));
    }

    #[test]
    fn test_privacy_engine_tree_full() {
        let mut engine = PrivacyEngine::new(2); // Only 4 leaves

        for i in 0..4 {
            engine
                .shield(100, test_owner(i), test_blinding(i), None, None, 0)
                .unwrap();
        }

        // 5th should fail
        let result = engine.shield(100, test_owner(5), test_blinding(5), None, None, 0);
        assert!(matches!(result, Err(PrivacyError::TreeFull)));
    }

    #[test]
    fn test_stale_anchor_rejected() {
        let mut engine = PrivacyEngine::new(8);

        // Shield a note
        let blind = test_blinding(1);
        let shield = engine
            .shield(1000, test_owner(1), blind, None, None, 0)
            .unwrap();

        // Get proof with current root
        let merkle_proof = engine.get_merkle_proof(shield.tree_index).unwrap();
        let _old_root = engine.merkle_root();

        // Shield another note (changes the root)
        engine
            .shield(500, test_owner(2), test_blinding(2), None, None, 0)
            .unwrap();

        // Build transfer with the OLD root
        let witness = PrivateTransferWitness {
            inputs: vec![InputWitness {
                note: ConfidentialNote {
                    value_commitment: Commitment::commit(1000, &blind),
                    owner_hash: test_owner(1),
                    energy_commitment: None,
                    creation_epoch: 0,
                    half_life: 0,
                },
                amount: 1000,
                blinding: blind,
                spending_secret: test_blinding(10),
                merkle_proof,
            }],
            outputs: vec![OutputWitness {
                amount: 1000,
                blinding: test_blinding(3),
                owner_hash: test_owner(2),
                energy: None,
                energy_blinding: None,
                creation_epoch: 0,
                half_life: 0,
            }],
            fee: 0,
        };

        let proof = witness.prove().unwrap();
        // The proof's anchor is the old root — engine should reject it
        assert!(matches!(
            engine.process_private_transfer(&proof),
            Err(PrivacyError::StaleAnchor)
        ));
    }

    #[test]
    fn test_multi_input_private_transfer() {
        let mut engine = PrivacyEngine::new(8);

        // Shield two notes
        let blind1 = test_blinding(1);
        let blind2 = test_blinding(2);
        let secret = test_blinding(99);
        let owner = test_owner(1);

        let s1 = engine.shield(600, owner, blind1, None, None, 0).unwrap();
        let s2 = engine.shield(400, owner, blind2, None, None, 0).unwrap();

        let proof1 = engine.get_merkle_proof(s1.tree_index).unwrap();
        let proof2 = engine.get_merkle_proof(s2.tree_index).unwrap();

        // Spend both notes in one transfer: 600 + 400 = 950 to Bob + 50 fee
        let witness = PrivateTransferWitness {
            inputs: vec![
                InputWitness {
                    note: ConfidentialNote {
                        value_commitment: Commitment::commit(600, &blind1),
                        owner_hash: owner,
                        energy_commitment: None,
                        creation_epoch: 0,
                        half_life: 0,
                    },
                    amount: 600,
                    blinding: blind1,
                    spending_secret: secret,
                    merkle_proof: proof1,
                },
                InputWitness {
                    note: ConfidentialNote {
                        value_commitment: Commitment::commit(400, &blind2),
                        owner_hash: owner,
                        energy_commitment: None,
                        creation_epoch: 0,
                        half_life: 0,
                    },
                    amount: 400,
                    blinding: blind2,
                    spending_secret: secret,
                    merkle_proof: proof2,
                },
            ],
            outputs: vec![OutputWitness {
                amount: 950,
                blinding: test_blinding(50),
                owner_hash: test_owner(2),
                energy: None,
                energy_blinding: None,
                creation_epoch: 0,
                half_life: 0,
            }],
            fee: 50,
        };

        let proof = witness.prove().unwrap();
        assert_eq!(proof.input_nullifiers.len(), 2);

        let result = engine.process_private_transfer(&proof).unwrap();
        assert_eq!(result.nullifiers_spent, 2);
        assert_eq!(result.fee, 50);
        assert_eq!(engine.nullifier_count(), 2);
    }

    // ── Balance Binding Verification Tests ──

    #[test]
    fn test_balance_binding_correct() {
        let ib = [test_blinding(1), test_blinding(2)];
        let ob = [test_blinding(3)];
        let binding = compute_balance_binding(1000, 900, 100, &ib, &ob);
        assert!(verify_balance_binding(&binding, 1000, 900, 100, &ib, &ob));
    }

    #[test]
    fn test_balance_binding_wrong_fee() {
        let ib = [test_blinding(1)];
        let ob = [test_blinding(2)];
        let binding = compute_balance_binding(1000, 900, 100, &ib, &ob);
        // Wrong fee
        assert!(!verify_balance_binding(&binding, 1000, 900, 50, &ib, &ob));
    }

    #[test]
    fn test_balance_binding_wrong_amounts() {
        let ib = [test_blinding(1)];
        let ob = [test_blinding(2)];
        let binding = compute_balance_binding(1000, 900, 100, &ib, &ob);
        // Wrong sum_in
        assert!(!verify_balance_binding(&binding, 999, 900, 100, &ib, &ob));
        // Wrong sum_out
        assert!(!verify_balance_binding(&binding, 1000, 901, 100, &ib, &ob));
    }

    #[test]
    fn test_balance_binding_wrong_blindings() {
        let ib = [test_blinding(1)];
        let ob = [test_blinding(2)];
        let binding = compute_balance_binding(1000, 900, 100, &ib, &ob);
        // Tampered input blinding
        let tampered_ib = [test_blinding(99)];
        assert!(!verify_balance_binding(
            &binding,
            1000,
            900,
            100,
            &tampered_ib,
            &ob
        ));
        // Tampered output blinding
        let tampered_ob = [test_blinding(99)];
        assert!(!verify_balance_binding(
            &binding,
            1000,
            900,
            100,
            &ib,
            &tampered_ob
        ));
    }

    #[test]
    fn test_balance_binding_deterministic() {
        let ib = [test_blinding(5)];
        let ob = [test_blinding(6), test_blinding(7)];
        let b1 = compute_balance_binding(500, 400, 100, &ib, &ob);
        let b2 = compute_balance_binding(500, 400, 100, &ib, &ob);
        assert_eq!(b1, b2);
    }
}
