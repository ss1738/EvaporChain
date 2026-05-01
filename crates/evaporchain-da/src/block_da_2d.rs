//! 2D Block DA — Celestia-style data availability with row/column commitments.
//!
//! Upgrades from 1D shard encoding to 2D extended data square (EDS).
//! Each block's data is arranged into a k×k matrix, extended to 2k×2k,
//! with row and column Merkle roots producing a single `data_root`.
//! Light clients can verify availability by sampling random cells and
//! checking proofs against both row and column commitments.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::commitments::{generate_2d_queries, CellProof, CellQuery, RowColumnCommitments};
use crate::erasure2d::{ErasureEncoder2D, Matrix2D};
use crate::namespace::{NamespaceId, NamespaceMerkleTree, NamespacedBlob, NmtNode};

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

/// Availability scoring metrics for DAS light-client sampling.
///
/// Given a set of `CellSampleResult`s from `light_client_sample()`, this struct
/// computes recovery feasibility and a confidence score using the standard DAS
/// confidence bound (as used by Celestia): `confidence = 1 - 2^(-valid_samples)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailabilityMetrics {
    /// Total number of samples attempted.
    pub total_samples: usize,
    /// Number of samples that verified successfully.
    pub valid_samples: usize,
    /// Number of unique rows containing at least one valid sample.
    pub unique_rows_hit: usize,
    /// Number of unique columns containing at least one valid sample.
    pub unique_cols_hit: usize,
    /// Extended matrix dimension (2k).
    pub extended_dim: usize,
    /// Whether recovery is possible: true if unique_rows_hit >= k OR unique_cols_hit >= k.
    pub recovery_possible: bool,
    /// DAS confidence that >= 50% of cells are available: `1 - 2^(-valid_samples)`.
    pub confidence: f64,
}

impl AvailabilityMetrics {
    /// Compute availability metrics from a set of cell sample results.
    ///
    /// `extended_dim` is the 2k dimension of the extended data square.
    /// The original dimension k = extended_dim / 2. Recovery requires at least k
    /// unique rows OR k unique columns with valid samples (sufficient for
    /// Reed-Solomon reconstruction along that axis).
    pub fn from_samples(samples: &[CellSampleResult], extended_dim: usize) -> Self {
        let mut valid_rows: HashSet<usize> = HashSet::new();
        let mut valid_cols: HashSet<usize> = HashSet::new();
        let mut valid_samples = 0usize;

        for s in samples {
            if s.valid {
                valid_samples += 1;
                valid_rows.insert(s.row);
                valid_cols.insert(s.col);
            }
        }

        let unique_rows_hit = valid_rows.len();
        let unique_cols_hit = valid_cols.len();
        let k = extended_dim / 2;
        let recovery_possible = unique_rows_hit >= k || unique_cols_hit >= k;

        // Standard DAS confidence bound: probability that >= 50% of cells are
        // available given S valid random samples is 1 - (1/2)^S.
        let confidence = if valid_samples == 0 {
            0.0
        } else {
            1.0 - 2.0_f64.powi(-(valid_samples as i32))
        };

        Self {
            total_samples: samples.len(),
            valid_samples,
            unique_rows_hit,
            unique_cols_hit,
            extended_dim,
            recovery_possible,
            confidence,
        }
    }
}

/// 2D block DA encoder with Celestia-style cell sampling.
pub struct BlockDA2D {
    cell_size: usize,
}

impl Default for BlockDA2D {
    fn default() -> Self {
        Self::new()
    }
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
        let queries = Self::generate_cell_queries(block_number, &package.header, num_samples, seed);

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

    /// Compute availability metrics from cell sample results.
    ///
    /// Convenience method that delegates to `AvailabilityMetrics::from_samples`.
    pub fn availability_score(
        samples: &[CellSampleResult],
        extended_dim: usize,
    ) -> AvailabilityMetrics {
        AvailabilityMetrics::from_samples(samples, extended_dim)
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
                    r,
                    c
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

    // ─────────────── Availability scoring tests ──────────────────────────

    #[test]
    fn test_availability_all_valid() {
        // 16 valid samples across a 2k=8 matrix (k=4).
        // Spread across 4 rows and 4 cols so recovery is possible.
        let extended_dim = 8;
        let samples: Vec<CellSampleResult> = (0..16)
            .map(|i| CellSampleResult {
                row: i % extended_dim,
                col: (i * 3) % extended_dim,
                cell_hash: format!("hash_{}", i),
                row_root: format!("rr_{}", i % extended_dim),
                col_root: format!("cr_{}", (i * 3) % extended_dim),
                valid: true,
            })
            .collect();

        let metrics = BlockDA2D::availability_score(&samples, extended_dim);
        assert_eq!(metrics.total_samples, 16);
        assert_eq!(metrics.valid_samples, 16);
        assert_eq!(metrics.extended_dim, 8);
        // confidence = 1 - 2^(-16) ≈ 0.999985
        assert!(
            metrics.confidence > 0.9999,
            "confidence should be very high: {}",
            metrics.confidence
        );
        // 16 samples mod 8 rows => hits all 8 rows, k=4, so recovery_possible
        assert!(
            metrics.recovery_possible,
            "recovery should be possible with all rows hit"
        );
    }

    #[test]
    fn test_availability_some_invalid() {
        let extended_dim = 8;
        let mut samples: Vec<CellSampleResult> = Vec::new();

        // 10 valid
        for i in 0..10 {
            samples.push(CellSampleResult {
                row: i % extended_dim,
                col: i % extended_dim,
                cell_hash: format!("hash_{}", i),
                row_root: format!("rr_{}", i),
                col_root: format!("cr_{}", i),
                valid: true,
            });
        }
        // 6 invalid
        for i in 10..16 {
            samples.push(CellSampleResult {
                row: i % extended_dim,
                col: i % extended_dim,
                cell_hash: String::new(),
                row_root: String::new(),
                col_root: String::new(),
                valid: false,
            });
        }

        let metrics = BlockDA2D::availability_score(&samples, extended_dim);
        assert_eq!(metrics.total_samples, 16);
        assert_eq!(metrics.valid_samples, 10);
        // confidence = 1 - 2^(-10) ≈ 0.999
        assert!(
            metrics.confidence > 0.999,
            "confidence based on valid count: {}",
            metrics.confidence
        );
        assert!(metrics.confidence < 1.0);
        // Invalid samples don't count toward unique rows/cols
        // valid rows: 0..10 mod 8 => {0,1,2,3,4,5,6,7} = 8, invalid not counted
        assert_eq!(metrics.unique_rows_hit, 8);
    }

    #[test]
    fn test_availability_recovery_possible() {
        // extended_dim = 6, k = 3. Need >= 3 unique rows OR >= 3 unique cols.
        let extended_dim = 6;

        // 4 valid samples hitting rows {0, 1, 2} — that's >= k=3
        let samples = vec![
            CellSampleResult {
                row: 0,
                col: 0,
                cell_hash: "h0".into(),
                row_root: "r0".into(),
                col_root: "c0".into(),
                valid: true,
            },
            CellSampleResult {
                row: 1,
                col: 0,
                cell_hash: "h1".into(),
                row_root: "r1".into(),
                col_root: "c0".into(),
                valid: true,
            },
            CellSampleResult {
                row: 2,
                col: 1,
                cell_hash: "h2".into(),
                row_root: "r2".into(),
                col_root: "c1".into(),
                valid: true,
            },
            CellSampleResult {
                row: 2,
                col: 2,
                cell_hash: "h3".into(),
                row_root: "r2".into(),
                col_root: "c2".into(),
                valid: true,
            },
        ];

        let metrics = AvailabilityMetrics::from_samples(&samples, extended_dim);
        assert_eq!(metrics.unique_rows_hit, 3); // rows 0, 1, 2
        assert_eq!(metrics.unique_cols_hit, 3); // cols 0, 1, 2
        assert!(
            metrics.recovery_possible,
            "3 unique rows >= k=3, recovery should be possible"
        );
        assert_eq!(metrics.valid_samples, 4);
        // confidence = 1 - 2^(-4) = 0.9375
        let expected = 1.0 - 2.0_f64.powi(-4);
        assert!((metrics.confidence - expected).abs() < 1e-12);
    }

    #[test]
    fn test_availability_insufficient() {
        // extended_dim = 8, k = 4. Only 2 valid samples hitting 2 unique rows/cols.
        let extended_dim = 8;

        let samples = vec![
            CellSampleResult {
                row: 0,
                col: 0,
                cell_hash: "h0".into(),
                row_root: "r0".into(),
                col_root: "c0".into(),
                valid: true,
            },
            CellSampleResult {
                row: 1,
                col: 1,
                cell_hash: "h1".into(),
                row_root: "r1".into(),
                col_root: "c1".into(),
                valid: true,
            },
            CellSampleResult {
                row: 5,
                col: 5,
                cell_hash: String::new(),
                row_root: String::new(),
                col_root: String::new(),
                valid: false,
            },
        ];

        let metrics = BlockDA2D::availability_score(&samples, extended_dim);
        assert_eq!(metrics.total_samples, 3);
        assert_eq!(metrics.valid_samples, 2);
        assert_eq!(metrics.unique_rows_hit, 2);
        assert_eq!(metrics.unique_cols_hit, 2);
        assert!(
            !metrics.recovery_possible,
            "2 rows/cols < k=4, recovery should NOT be possible"
        );
        // confidence = 1 - 2^(-2) = 0.75
        let expected = 1.0 - 2.0_f64.powi(-2);
        assert!((metrics.confidence - expected).abs() < 1e-12);
    }
}
