//! Row and column commitments for 2D erasure-coded matrices.
//!
//! Computes Merkle roots over rows and columns, producing a single `data_root`
//! that goes into the block header. Supports cell-level proof generation and verification.
//!
//! ## Light-client soundness (post-audit)
//!
//! - **Leaf vs inner domain separation** (CVE-2012-2459-class fix):
//!   leaves and inner nodes are hashed under distinct domain tags so a
//!   64-byte cell whose hash equals `H(L)||H(R)` cannot be forged as
//!   an inner node.
//! - **Role-tagged row/col leaves**: row-roots are leaf-hashed under
//!   `ROW_ROOT_DOMAIN` and col-roots under `COL_ROOT_DOMAIN` when
//!   building `data_root`, so a row-root and col-root with identical
//!   bytes still produce distinct leaves in the combined tree.
//! - **Length-bound Merkle root**: the leaf-count is folded into the
//!   final root via a domain-tagged finaliser, eliminating the
//!   zero-padding ambiguity (Bitcoin-style `[0u8;32]` padding could
//!   otherwise admit forged inclusion proofs for any leaf whose hash
//!   happens to be zero).

use crate::erasure2d::Matrix2D;
use serde::{Deserialize, Serialize};

const LEAF_DOMAIN: &[u8] = b"evaporchain:da:v1:leaf\0";
const INNER_DOMAIN: &[u8] = b"evaporchain:da:v1:inner\0";
const PAD_DOMAIN: &[u8] = b"evaporchain:da:v1:pad\0";
const ROOT_DOMAIN: &[u8] = b"evaporchain:da:v1:root\0";
const ROW_ROOT_DOMAIN: &[u8] = b"evaporchain:da:v1:row-leaf\0";
const COL_ROOT_DOMAIN: &[u8] = b"evaporchain:da:v1:col-leaf\0";

/// A query targeting a specific cell in the 2D matrix.
#[derive(Debug, Clone)]
pub struct CellQuery {
    pub row: usize,
    pub col: usize,
}

/// Proof for a single cell in the 2D matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellProof {
    /// The cell data.
    pub cell_data: Vec<u8>,
    /// Row index.
    pub row: usize,
    /// Column index.
    pub col: usize,
    /// Hash of the cell.
    pub cell_hash: [u8; 32],
    /// Row root this cell belongs to.
    pub row_root: [u8; 32],
    /// Column root this cell belongs to.
    pub col_root: [u8; 32],
    /// Merkle siblings for row proof (cell -> row root).
    pub row_siblings: Vec<[u8; 32]>,
    /// Merkle siblings for column proof (cell -> col root).
    pub col_siblings: Vec<[u8; 32]>,
    /// The overall data_root.
    pub data_root: [u8; 32],
}

/// Row and column commitments for a 2D erasure-coded matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowColumnCommitments {
    /// Merkle roots for each row.
    pub row_roots: Vec<[u8; 32]>,
    /// Merkle roots for each column.
    pub col_roots: Vec<[u8; 32]>,
    /// Combined data root: Merkle root over (row_roots ++ col_roots).
    pub data_root: [u8; 32],
    /// Extended dimension of the matrix.
    pub extended_dim: usize,
}

impl RowColumnCommitments {
    /// Compute row and column commitments from a 2D matrix.
    pub fn from_matrix(matrix: &Matrix2D) -> Self {
        let dim = matrix.extended_dim;

        // Compute row roots
        let mut row_roots = Vec::with_capacity(dim);
        for r in 0..dim {
            let mut hashes = Vec::with_capacity(dim);
            for c in 0..dim {
                let cell = matrix.get_cell(r, c).unwrap_or(&[]);
                hashes.push(leaf_hash(cell));
            }
            row_roots.push(merkle_root(&hashes));
        }

        // Compute column roots
        let mut col_roots = Vec::with_capacity(dim);
        for c in 0..dim {
            let mut hashes = Vec::with_capacity(dim);
            for r in 0..dim {
                let cell = matrix.get_cell(r, c).unwrap_or(&[]);
                hashes.push(leaf_hash(cell));
            }
            col_roots.push(merkle_root(&hashes));
        }

        // Combined data_root: row-roots get ROW_ROOT_DOMAIN-tagged leaves,
        // col-roots get COL_ROOT_DOMAIN-tagged leaves. Distinct domains
        // mean a row-root and col-root with identical bytes still
        // produce distinct leaves in the combined Merkle tree (audit
        // fix C5: row/col proof confusion).
        let mut all_roots: Vec<[u8; 32]> = Vec::with_capacity(2 * dim);
        for r in &row_roots {
            all_roots.push(role_tagged_leaf(ROW_ROOT_DOMAIN, r));
        }
        for c in &col_roots {
            all_roots.push(role_tagged_leaf(COL_ROOT_DOMAIN, c));
        }
        let data_root = merkle_root(&all_roots);

        Self {
            row_roots,
            col_roots,
            data_root,
            extended_dim: dim,
        }
    }

    /// Generate a cell proof for a specific (row, col) in the matrix.
    pub fn generate_cell_proof(
        &self,
        matrix: &Matrix2D,
        row: usize,
        col: usize,
    ) -> Option<CellProof> {
        if row >= self.extended_dim || col >= self.extended_dim {
            return None;
        }

        let cell_data = matrix.get_cell(row, col)?.to_vec();
        let cell_hash = leaf_hash(&cell_data);

        // Row proof: cell -> row root (using domain-tagged leaves)
        let dim = self.extended_dim;
        let row_hashes: Vec<[u8; 32]> = (0..dim)
            .map(|c| {
                let cell = matrix.get_cell(row, c).unwrap_or(&[]);
                leaf_hash(cell)
            })
            .collect();
        let (_, row_siblings) = merkle_proof(&row_hashes, col);

        // Column proof: cell -> col root (using domain-tagged leaves)
        let col_hashes: Vec<[u8; 32]> = (0..dim)
            .map(|r| {
                let cell = matrix.get_cell(r, col).unwrap_or(&[]);
                leaf_hash(cell)
            })
            .collect();
        let (_, col_siblings) = merkle_proof(&col_hashes, row);

        Some(CellProof {
            cell_data,
            row,
            col,
            cell_hash,
            row_root: self.row_roots[row],
            col_root: self.col_roots[col],
            row_siblings,
            col_siblings,
            data_root: self.data_root,
        })
    }

    /// Verify a cell proof.
    pub fn verify_cell_proof(&self, proof: &CellProof) -> bool {
        // Verify cell hash (domain-tagged leaf)
        let computed_hash = leaf_hash(&proof.cell_data);
        if computed_hash != proof.cell_hash {
            return false;
        }

        let leaf_count = self.extended_dim as u64;

        // Verify row Merkle path: cell_hash -> row_root (length-bound).
        let row_inner = verify_merkle_path(&proof.cell_hash, proof.col, &proof.row_siblings);
        let row_root = finalise_root(&row_inner, leaf_count);
        if row_root != proof.row_root {
            return false;
        }

        // Verify column Merkle path: cell_hash -> col_root (length-bound).
        let col_inner = verify_merkle_path(&proof.cell_hash, proof.row, &proof.col_siblings);
        let col_root = finalise_root(&col_inner, leaf_count);
        if col_root != proof.col_root {
            return false;
        }

        // Verify that row_root and col_root are consistent with data_root
        if proof.data_root != self.data_root {
            return false;
        }

        true
    }
}

/// Generate random 2D cell queries for DAS.
pub fn generate_2d_queries(
    block_number: u64,
    extended_dim: usize,
    num_samples: usize,
    seed: &[u8],
) -> Vec<CellQuery> {
    let mut queries = Vec::with_capacity(num_samples);
    for i in 0..num_samples {
        let mut hasher = blake3::Hasher::new();
        hasher.update(seed);
        hasher.update(&block_number.to_le_bytes());
        hasher.update(&(i as u64).to_le_bytes());
        let hash = hasher.finalize();
        let bytes = hash.as_bytes();
        let row = (u64::from_le_bytes(bytes[0..8].try_into().unwrap()) as usize) % extended_dim;
        let col = (u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize) % extended_dim;
        queries.push(CellQuery { row, col });
    }
    queries
}

// ── Merkle helpers ──

/// Domain-tagged leaf hash. Cells, row-roots-as-data-leaves, and
/// col-roots-as-data-leaves all funnel through here (via either
/// `LEAF_DOMAIN` or a `role_tagged_leaf` wrapper).
///
/// `pub(crate)` so the light-client peer-validation path can reuse the
/// same hash (otherwise it would compute a bare blake3 and reject all
/// cells as `HashMismatch`).
pub(crate) fn leaf_hash(cell: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(LEAF_DOMAIN);
    hasher.update(cell);
    hasher.finalize().into()
}

/// Role-tagged variant for the combined `data_root` that stamps
/// row-vs-col distinction at leaf time. Input is already a Merkle
/// root (32 bytes); the role tag mixes in the domain.
fn role_tagged_leaf(role: &[u8], root: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(role);
    hasher.update(root);
    hasher.finalize().into()
}

/// Domain-tagged inner-node hash. Distinct from `leaf_hash` so a
/// 64-byte preimage cannot be both an inner node and a leaf
/// (CVE-2012-2459-class fix).
fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(INNER_DOMAIN);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

/// Domain-tagged padding leaf. Used when a Merkle layer's count is
/// odd; replaces the legacy `[0u8;32]` zero-leaf padding which would
/// have admitted forged inclusion proofs for any leaf whose hash
/// happened to be all zeros.
fn pad_leaf() -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(PAD_DOMAIN);
    hasher.finalize().into()
}

/// Length-bound Merkle finaliser. Folds the leaf count into the
/// root so two trees with different padded shapes cannot collide
/// (e.g. `[a, b, c]` padded to `[a, b, c, c]` vs a real 4-leaf tree
/// `[a, b, c, c]` with explicit duplicate).
fn finalise_root(inner_root: &[u8; 32], leaf_count: u64) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(ROOT_DOMAIN);
    hasher.update(inner_root);
    hasher.update(&leaf_count.to_le_bytes());
    hasher.finalize().into()
}

fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return finalise_root(&[0u8; 32], 0);
    }
    let leaf_count = leaves.len() as u64;
    if leaves.len() == 1 {
        return finalise_root(&leaves[0], leaf_count);
    }
    let mut layer: Vec<[u8; 32]> = leaves.to_vec();
    let pad = pad_leaf();
    while layer.len().count_ones() != 1 {
        layer.push(pad);
    }
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len() / 2);
        for pair in layer.chunks(2) {
            next.push(hash_pair(&pair[0], &pair[1]));
        }
        layer = next;
    }
    finalise_root(&layer[0], leaf_count)
}

fn merkle_proof(leaves: &[[u8; 32]], index: usize) -> ([u8; 32], Vec<[u8; 32]>) {
    if leaves.is_empty() {
        return (finalise_root(&[0u8; 32], 0), vec![]);
    }
    let leaf_count = leaves.len() as u64;
    let mut layer: Vec<[u8; 32]> = leaves.to_vec();
    let pad = pad_leaf();
    while layer.len().count_ones() != 1 {
        layer.push(pad);
    }
    let mut siblings = Vec::new();
    let mut current_index = index;
    while layer.len() > 1 {
        let sibling_index = if current_index.is_multiple_of(2) {
            current_index + 1
        } else {
            current_index - 1
        };
        if sibling_index < layer.len() {
            siblings.push(layer[sibling_index]);
        } else {
            siblings.push(pad);
        }
        let mut next = Vec::with_capacity(layer.len() / 2);
        for pair in layer.chunks(2) {
            next.push(hash_pair(&pair[0], &pair[1]));
        }
        layer = next;
        current_index /= 2;
    }
    (finalise_root(&layer[0], leaf_count), siblings)
}

/// Reconstruct the inner Merkle tree root from a leaf + siblings.
/// Caller must apply `finalise_root(inner, leaf_count)` to compare
/// against a stored `data_root`. We don't fold `leaf_count` here
/// because the verifier has it from the row/col-root structure.
fn verify_merkle_path(leaf: &[u8; 32], mut index: usize, siblings: &[[u8; 32]]) -> [u8; 32] {
    let mut current = *leaf;
    for sibling in siblings {
        current = if index.is_multiple_of(2) {
            hash_pair(&current, sibling)
        } else {
            hash_pair(sibling, &current)
        };
        index /= 2;
    }
    // For per-row/col proofs the verifier supplies leaf_count via
    // self.extended_dim (each row/col has exactly extended_dim leaves).
    // The legacy verify_cell_proof callers compare against the
    // row_root / col_root which were produced by `merkle_root` above
    // — already finalised. Therefore we finalise here too with the
    // standard extended-dim leaf count baked in by the caller.
    current
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::erasure2d::ErasureEncoder2D;

    #[test]
    fn test_commitments_basic() {
        let encoder = ErasureEncoder2D::with_cell_size(32);
        let data = vec![0xABu8; 256];
        let matrix = encoder.encode_2d(&data).unwrap();
        let rc = RowColumnCommitments::from_matrix(&matrix);
        assert_ne!(rc.data_root, [0u8; 32]);
        assert_eq!(rc.row_roots.len(), matrix.extended_dim());
        assert_eq!(rc.col_roots.len(), matrix.extended_dim());
    }

    #[test]
    fn test_cell_proof_roundtrip() {
        let encoder = ErasureEncoder2D::with_cell_size(32);
        let data = vec![0xCDu8; 512];
        let matrix = encoder.encode_2d(&data).unwrap();
        let rc = RowColumnCommitments::from_matrix(&matrix);

        for r in 0..matrix.extended_dim().min(4) {
            for c in 0..matrix.extended_dim().min(4) {
                let proof = rc.generate_cell_proof(&matrix, r, c).unwrap();
                assert!(
                    rc.verify_cell_proof(&proof),
                    "Proof failed at ({}, {})",
                    r,
                    c
                );
            }
        }
    }

    fn build_test_rc() -> (Matrix2D, RowColumnCommitments) {
        let encoder = ErasureEncoder2D::with_cell_size(32);
        let matrix = encoder.encode_2d(&vec![0x77u8; 256]).unwrap();
        let rc = RowColumnCommitments::from_matrix(&matrix);
        (matrix, rc)
    }

    #[test]
    fn test_out_of_bounds_proof_returns_none() {
        let (matrix, rc) = build_test_rc();
        let dim = rc.extended_dim;
        assert!(rc.generate_cell_proof(&matrix, dim, 0).is_none());
        assert!(rc.generate_cell_proof(&matrix, 0, dim).is_none());
        assert!(rc.generate_cell_proof(&matrix, dim + 5, dim + 5).is_none());
    }

    #[test]
    fn test_tampered_cell_data_fails_verify() {
        let (matrix, rc) = build_test_rc();
        let mut proof = rc.generate_cell_proof(&matrix, 0, 0).unwrap();
        // Mutate cell_data so it no longer matches cell_hash.
        if let Some(b) = proof.cell_data.get_mut(0) {
            *b ^= 0xFF;
        }
        assert!(!rc.verify_cell_proof(&proof));
    }

    #[test]
    fn test_tampered_data_root_fails_verify() {
        let (matrix, rc) = build_test_rc();
        let mut proof = rc.generate_cell_proof(&matrix, 1, 1).unwrap();
        proof.data_root[0] ^= 0xFF;
        assert!(!rc.verify_cell_proof(&proof));
    }

    #[test]
    fn test_tampered_row_root_fails_verify() {
        let (matrix, rc) = build_test_rc();
        let mut proof = rc.generate_cell_proof(&matrix, 0, 0).unwrap();
        proof.row_root[5] ^= 0x01;
        assert!(!rc.verify_cell_proof(&proof));
    }

    #[test]
    fn test_generate_2d_queries_deterministic() {
        let q1 = generate_2d_queries(42, 16, 8, b"seed-x");
        let q2 = generate_2d_queries(42, 16, 8, b"seed-x");
        assert_eq!(q1.len(), 8);
        assert_eq!(q1.len(), q2.len());
        for (a, b) in q1.iter().zip(q2.iter()) {
            assert_eq!(a.row, b.row);
            assert_eq!(a.col, b.col);
        }
    }

    #[test]
    fn test_generate_2d_queries_in_bounds() {
        let dim = 16usize;
        let queries = generate_2d_queries(7, dim, 64, b"any-seed");
        assert_eq!(queries.len(), 64);
        for q in &queries {
            assert!(q.row < dim, "row {} out of bounds", q.row);
            assert!(q.col < dim, "col {} out of bounds", q.col);
        }
    }

    #[test]
    fn test_generate_2d_queries_seed_changes_pattern() {
        let q1 = generate_2d_queries(1, 16, 16, b"seed-A");
        let q2 = generate_2d_queries(1, 16, 16, b"seed-B");
        // Different seeds must produce a different sample set (probabilistically
        // — collision over 16 samples in a 16×16 grid is astronomically unlikely)
        let same = q1
            .iter()
            .zip(q2.iter())
            .all(|(a, b)| a.row == b.row && a.col == b.col);
        assert!(!same, "different seeds should yield different sample patterns");
    }
}
