//! 2D Block DA — Celestia-style data availability with row/column commitments.
//!
//! Upgrades from 1D shard encoding to 2D extended data square (EDS).
//! Each block's data is arranged into a k×k matrix, extended to 2k×2k,
//! with row and column Merkle roots producing a single `data_root`.
//! Light clients can verify availability by sampling random cells and
//! checking proofs against both row and column commitments.

use serde::{Deserialize, Serialize};

use crate::commitments::{CellProof, RowColumnCommitments, generate_2d_queries, CellQuery};
use crate::erasure2d::{ErasureEncoder2D, Matrix2D};
use crate::namespace::{NamespaceMerkleTree, NamespacedBlob, NamespaceId, NmtNode};

/// Errors from 2D block DA operations.
#[derive(Debug, thiserror::Error)]
pub enum BlockDA2DError {
    #[error("erasure error: {0}")]
    Erasure(#[from] crate::erasure::ErasureError),
    #[error("empty block data")]
    EmptyData,
    #[error("cell proof generation failed for ({0}, {1})")]
    CellProofFailed(usize, usize),
}

/// 2D DA header — goes into the block header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDA2DHeader {
    /// Combined data_root: Merkle root over (row_roots ++ col_roots).
    pub data_root: [u8; 32],
    /// Row Merkle roots.
    pub row_roots: Vec<[u8; 32]>,
    /// Column Merkle roots.
    pub col_roots: Vec<[u8; 32]>,
    /// Extended dimension (2k).
    pub extended_dim: usize,
    /// Original dimension (k).
    pub original_dim: usize,
    /// Cell size in bytes.
    pub cell_size: usize,
    /// Original data length.
    pub original_len: usize,
    /// Blake3 hash of the original data.
    pub data_hash: [u8; 32],
    /// NMT root (if namespace blobs are included).
    pub nmt_root: Option<NmtNode>,
    /// Per-namespace blob commitments.
    pub blob_commitments: Vec<[u8; 32]>,
}

/// Full 2D DA package: header + matrix + optional NMT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDA2DPackage {
    pub header: BlockDA2DHeader,
    pub matrix: Matrix2D,
}

/// Result of a 2D cell sample verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellSampleResult {
    pub row: usize,
    pub col: usize,
    pub cell_hash: String,
    pub row_root: String,
    pub col_root: String,
    pub valid: bool,
}

/// 2D block DA encoder with Celestia-style cell sampling.
pub struct BlockDA2D {
    cell_size: usize,
}

impl BlockDA2D {
    pub fn new() -> Self {
        Self { cell_size: 64 }
    }

    pub fn with_cell_size(cell_size: usize) -> Self {
        Self { cell_size }
    }

    /// Encode block data into a 2D extended data square with row/column commitments.
    pub fn encode_block(&self, block_data: &[u8]) -> Result<BlockDA2DPackage, BlockDA2DError> {
        if block_data.is_empty() {
            return Err(BlockDA2DError::EmptyData);
        }

        let data_hash: [u8; 32] = blake3::hash(block_data).into();

        let encoder = ErasureEncoder2D::with_cell_size(self.cell_size);
        let matrix = encoder.encode_2d(block_data)?;
        let commitments = RowColumnCommitments::from_matrix(&matrix);

        Ok(BlockDA2DPackage {
            header: BlockDA2DHeader {
                data_root: commitments.data_root,
                row_roots: commitments.row_roots.clone(),
                col_roots: commitments.col_roots.clone(),
                extended_dim: matrix.extended_dim,
                original_dim: matrix.original_dim,
                cell_size: self.cell_size,
                original_len: block_data.len(),
                data_hash,
                nmt_root: None,
                blob_commitments: Vec::new(),
            },
            matrix,
        })
    }

    /// Encode block data with namespace-tagged blobs.
    pub fn encode_block_with_blobs(
        &self,
        block_data: &[u8],
        blobs: &[NamespacedBlob],
    ) -> Result<BlockDA2DPackage, BlockDA2DError> {
        let mut package = self.encode_block(block_data)?;

        if !blobs.is_empty() {
            let nmt = NamespaceMerkleTree::from_blobs(blobs);
            package.header.nmt_root = Some(nmt.root().clone());
            package.header.blob_commitments = nmt.blob_commitment_hashes();
        }

        Ok(package)
    }

    /// Generate a cell proof for a specific (row, col) in the 2D matrix.
    pub fn prove_cell(
        &self,
        package: &BlockDA2DPackage,
        row: usize,
        col: usize,
    ) -> Result<CellProof, BlockDA2DError> {
        let commitments = RowColumnCommitments {
            row_roots: package.header.row_roots.clone(),
            col_roots: package.header.col_roots.clone(),
            data_root: package.header.data_root,
            extended_dim: package.header.extended_dim,
        };

        commitments
            .generate_cell_proof(&package.matrix, row, col)
            .ok_or(BlockDA2DError::CellProofFailed(row, col))
    }

    /// Verify a cell proof against the block's 2D header.
    pub fn verify_cell_proof(header: &BlockDA2DHeader, proof: &CellProof) -> bool {
        let commitments = RowColumnCommitments {
            row_roots: header.row_roots.clone(),
            col_roots: header.col_roots.clone(),
            data_root: header.data_root,
            extended_dim: header.extended_dim,
        };
        commitments.verify_cell_proof(proof)
    }

    /// Generate random cell queries for light client DAS.
    pub fn generate_cell_queries(
        block_number: u64,
        header: &BlockDA2DHeader,
        num_samples: usize,
        seed: &[u8],
    ) -> Vec<CellQuery> {
        generate_2d_queries(block_number, header.extended_dim, num_samples, seed)
    }

    /// Run a full light-client sample check: generate queries, prove cells, verify.
    /// Returns (results, all_valid).
    pub fn light_client_sample(
        &self,
        package: &BlockDA2DPackage,
        block_number: u64,
        num_samples: usize,
        seed: &[u8],
    ) -> (Vec<CellSampleResult>, bool) {
        let queries = Self::generate_cell_queries(
            block_number,
            &package.header,
            num_samples,
            seed,
        );

        let mut results = Vec::with_capacity(queries.len());
        let mut all_valid = true;

        for query in &queries {
            match self.prove_cell(package, query.row, query.col) {
                Ok(proof) => {
                    let valid = Self::verify_cell_proof(&package.header, &proof);
                    if !valid {
                        all_valid = false;
                    }
                    results.push(CellSampleResult {
                        row: query.row,
                        col: query.col,
                        cell_hash: hex::encode(proof.cell_hash),
                        row_root: hex::encode(proof.row_root),
                        col_root: hex::encode(proof.col_root),
                        valid,
                    });
                }
                Err(_) => {
                    all_valid = false;
                    results.push(CellSampleResult {
                        row: query.row,
                        col: query.col,
                        cell_hash: String::new(),
                        row_root: String::new(),
                        col_root: String::new(),
                        valid: false,
                    });
                }
            }
        }

        (results, all_valid)
    }
}

// ─────────────── Namespace helpers ──────────────────────────────────────

/// Well-known namespace IDs for EvaporChain transaction types.
pub const NS_TRANSFER: NamespaceId = [0, 0, 0, 0, 0, 0, 0, 1];
pub const NS_CREATE_OBJECT: NamespaceId = [0, 0, 0, 0, 0, 0, 0, 2];
pub const NS_REFRESH: NamespaceId = [0, 0, 0, 0, 0, 0, 0, 3];
pub const NS_DEFERRED: NamespaceId = [0, 0, 0, 0, 0, 0, 0, 4];
pub const NS_BLOB: NamespaceId = [0, 0, 0, 0, 0, 0, 0, 5];
pub const NS_SHIELDED: NamespaceId = [0, 0, 0, 0, 0, 0, 0, 6];
pub const NS_SYSTEM: NamespaceId = [0, 0, 0, 0, 0, 0, 0, 0xFF];

/// Tag a transaction's serialized bytes with its namespace.
pub fn namespace_for_tx_type(tx_type: &str) -> NamespaceId {
    match tx_type {
        "transfer" => NS_TRANSFER,
        "create_object" => NS_CREATE_OBJECT,
        "refresh" => NS_REFRESH,
        "deferred" => NS_DEFERRED,
        "blob" => NS_BLOB,
        "shielded_transfer" => NS_SHIELDED,
        _ => NS_SYSTEM,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_2d_block() {
        let da = BlockDA2D::new();
        let block_data = br#"{"number":42,"txs":["transfer","refresh","create_object"]}"#;

        let package = da.encode_block(block_data).unwrap();
        assert_ne!(package.header.data_root, [0u8; 32]);
        assert!(package.header.extended_dim >= 4);
        assert_eq!(package.header.original_len, block_data.len());
        assert!(!package.header.row_roots.is_empty());
        assert!(!package.header.col_roots.is_empty());
        assert_eq!(package.header.row_roots.len(), package.header.extended_dim);
        assert_eq!(package.header.col_roots.len(), package.header.extended_dim);
    }

    #[test]
    fn test_cell_proof_roundtrip() {
        let da = BlockDA2D::new();
        let block_data = vec![0xABu8; 512];
        let package = da.encode_block(&block_data).unwrap();

        for r in 0..package.header.extended_dim.min(4) {
            for c in 0..package.header.extended_dim.min(4) {
                let proof = da.prove_cell(&package, r, c).unwrap();
                assert!(
                    BlockDA2D::verify_cell_proof(&package.header, &proof),
                    "Cell proof failed at ({}, {})",
                    r, c
                );
            }
        }
    }

    #[test]
    fn test_tampered_cell_fails() {
        let da = BlockDA2D::new();
        let block_data = vec![0xCDu8; 256];
        let package = da.encode_block(&block_data).unwrap();

        let mut proof = da.prove_cell(&package, 0, 0).unwrap();
        proof.cell_data[0] ^= 0xFF;
        assert!(!BlockDA2D::verify_cell_proof(&package.header, &proof));
    }

    #[test]
    fn test_light_client_sample() {
        let da = BlockDA2D::new();
        let block_data = vec![0xEFu8; 1024];
        let package = da.encode_block(&block_data).unwrap();

        let (results, all_valid) = da.light_client_sample(&package, 1, 8, b"test-seed");
        assert_eq!(results.len(), 8);
        assert!(all_valid, "All light client samples should verify");
        for r in &results {
            assert!(r.valid);
            assert!(!r.cell_hash.is_empty());
        }
    }

    #[test]
    fn test_encode_with_nmt_blobs() {
        let da = BlockDA2D::new();
        let block_data = br#"{"number":1}"#;
        let blobs = vec![
            NamespacedBlob {
                namespace: NS_TRANSFER,
                data: b"transfer-data-1".to_vec(),
            },
            NamespacedBlob {
                namespace: NS_CREATE_OBJECT,
                data: b"create-obj-data".to_vec(),
            },
            NamespacedBlob {
                namespace: NS_TRANSFER,
                data: b"transfer-data-2".to_vec(),
            },
        ];

        let package = da.encode_block_with_blobs(block_data, &blobs).unwrap();
        assert!(package.header.nmt_root.is_some());
        assert_eq!(package.header.blob_commitments.len(), 3);

        let nmt_root = package.header.nmt_root.as_ref().unwrap();
        assert_eq!(nmt_root.min_namespace, NS_TRANSFER);
        assert_eq!(nmt_root.max_namespace, NS_CREATE_OBJECT);
    }

    #[test]
    fn test_deterministic_queries() {
        let da = BlockDA2D::new();
        let block_data = vec![0xAAu8; 512];
        let package = da.encode_block(&block_data).unwrap();

        let q1 = BlockDA2D::generate_cell_queries(1, &package.header, 4, b"seed");
        let q2 = BlockDA2D::generate_cell_queries(1, &package.header, 4, b"seed");
        assert_eq!(q1.len(), q2.len());
        for (a, b) in q1.iter().zip(q2.iter()) {
            assert_eq!(a.row, b.row);
            assert_eq!(a.col, b.col);
        }
    }

    #[test]
    fn test_empty_data_error() {
        let da = BlockDA2D::new();
        assert!(da.encode_block(b"").is_err());
    }

    #[test]
    fn test_namespace_for_tx_type() {
        assert_eq!(namespace_for_tx_type("transfer"), NS_TRANSFER);
        assert_eq!(namespace_for_tx_type("create_object"), NS_CREATE_OBJECT);
        assert_eq!(namespace_for_tx_type("refresh"), NS_REFRESH);
        assert_eq!(namespace_for_tx_type("unknown"), NS_SYSTEM);
    }
}
