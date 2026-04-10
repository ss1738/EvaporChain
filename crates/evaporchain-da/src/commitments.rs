//! Row and column commitments for 2D erasure-coded matrices.
//!
//! Computes Merkle roots over rows and columns, producing a single `data_root`
//! that goes into the block header. Supports cell-level proof generation and verification.

use crate::erasure2d::Matrix2D;
use serde::{Deserialize, Serialize};

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
                let h: [u8; 32] = blake3::hash(cell).into();
                hashes.push(h);
            }
            row_roots.push(merkle_root(&hashes));
        }

        // Compute column roots
        let mut col_roots = Vec::with_capacity(dim);
        for c in 0..dim {
            let mut hashes = Vec::with_capacity(dim);
            for r in 0..dim {
                let cell = matrix.get_cell(r, c).unwrap_or(&[]);
                let h: [u8; 32] = blake3::hash(cell).into();
                hashes.push(h);
            }
            col_roots.push(merkle_root(&hashes));
        }

        // Combined data_root = Merkle root over row_roots ++ col_roots
        let mut all_roots: Vec<[u8; 32]> = Vec::with_capacity(2 * dim);
        all_roots.extend_from_slice(&row_roots);
        all_roots.extend_from_slice(&col_roots);
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
        let cell_hash: [u8; 32] = blake3::hash(&cell_data).into();

        // Row proof: cell -> row root
        let dim = self.extended_dim;
        let row_hashes: Vec<[u8; 32]> = (0..dim)
            .map(|c| {
                let cell = matrix.get_cell(row, c).unwrap_or(&[]);
                blake3::hash(cell).into()
            })
            .collect();
        let (_, row_siblings) = merkle_proof(&row_hashes, col);

        // Column proof: cell -> col root
        let col_hashes: Vec<[u8; 32]> = (0..dim)
            .map(|r| {
                let cell = matrix.get_cell(r, col).unwrap_or(&[]);
                blake3::hash(cell).into()
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
        // Verify cell hash
        let computed_hash: [u8; 32] = blake3::hash(&proof.cell_data).into();
        if computed_hash != proof.cell_hash {
            return false;
        }

        // Verify row Merkle path: cell_hash -> row_root
        let row_root = verify_merkle_path(&proof.cell_hash, proof.col, &proof.row_siblings);
        if row_root != proof.row_root {
            return false;
        }

        // Verify column Merkle path: cell_hash -> col_root
        let col_root = verify_merkle_path(&proof.cell_hash, proof.row, &proof.col_siblings);
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

fn hash_pair(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

fn merkle_root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0u8; 32];
    }
    if leaves.len() == 1 {
        return leaves[0];
    }
    let mut layer: Vec<[u8; 32]> = leaves.to_vec();
    while layer.len().count_ones() != 1 {
        layer.push([0u8; 32]);
    }
    while layer.len() > 1 {
        let mut next = Vec::with_capacity(layer.len() / 2);
        for pair in layer.chunks(2) {
            next.push(hash_pair(&pair[0], &pair[1]));
        }
        layer = next;
    }
    layer[0]
}

fn merkle_proof(leaves: &[[u8; 32]], index: usize) -> ([u8; 32], Vec<[u8; 32]>) {
    if leaves.is_empty() {
        return ([0u8; 32], vec![]);
    }
    let mut layer: Vec<[u8; 32]> = leaves.to_vec();
    while layer.len().count_ones() != 1 {
        layer.push([0u8; 32]);
    }
    let mut siblings = Vec::new();
    let mut current_index = index;
    while layer.len() > 1 {
        let sibling_index = if current_index % 2 == 0 {
            current_index + 1
        } else {
            current_index - 1
        };
        if sibling_index < layer.len() {
            siblings.push(layer[sibling_index]);
        } else {
            siblings.push([0u8; 32]);
        }
        let mut next = Vec::with_capacity(layer.len() / 2);
        for pair in layer.chunks(2) {
            next.push(hash_pair(&pair[0], &pair[1]));
        }
        layer = next;
        current_index /= 2;
    }
    (layer[0], siblings)
}

fn verify_merkle_path(leaf: &[u8; 32], mut index: usize, siblings: &[[u8; 32]]) -> [u8; 32] {
    let mut current = *leaf;
    for sibling in siblings {
        current = if index % 2 == 0 {
            hash_pair(&current, sibling)
        } else {
            hash_pair(sibling, &current)
        };
        index /= 2;
    }
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
                assert!(rc.verify_cell_proof(&proof), "Proof failed at ({}, {})", r, c);
            }
        }
    }
}
