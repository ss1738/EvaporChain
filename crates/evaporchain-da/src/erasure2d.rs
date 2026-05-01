//! 2D Reed-Solomon erasure coding for data availability.
//!
//! Arranges data into a square matrix, extends rows and columns with parity,
//! producing a 2D extended matrix suitable for Celestia-style DAS.

use crate::erasure::{ErasureConfig, ErasureEncoder, ErasureError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A 2D erasure-coded matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Matrix2D {
    /// Original dimension (k x k data cells).
    pub original_dim: usize,
    /// Extended dimension (2k x 2k with parity).
    pub extended_dim: usize,
    /// Cell size in bytes.
    pub cell_size: usize,
    /// Flattened matrix data: row-major, extended_dim x extended_dim cells.
    pub cells: Vec<Vec<u8>>,
}

impl Matrix2D {
    /// Get the extended dimension (2k).
    pub fn extended_dim(&self) -> usize {
        self.extended_dim
    }

    /// Get a cell at (row, col) in the extended matrix.
    pub fn get_cell(&self, row: usize, col: usize) -> Option<&[u8]> {
        if row >= self.extended_dim || col >= self.extended_dim {
            return None;
        }
        let idx = row * self.extended_dim + col;
        self.cells.get(idx).map(|c| c.as_slice())
    }
}

/// 2D erasure encoder that produces an extended data square.
pub struct ErasureEncoder2D {
    cell_size: usize,
}

impl ErasureEncoder2D {
    /// Create a new 2D encoder with the given cell size.
    pub fn with_cell_size(cell_size: usize) -> Self {
        Self { cell_size }
    }

    /// Encode data into a 2D extended matrix.
    ///
    /// 1. Pad data to fill a k x k grid of cells.
    /// 2. Extend each row from k to 2k using RS parity.
    /// 3. Extend each column from k to 2k using RS parity.
    pub fn encode_2d(&self, data: &[u8]) -> Result<Matrix2D, ErasureError> {
        if data.is_empty() {
            return Err(ErasureError::EmptyData);
        }

        // Determine k: smallest k such that k*k*cell_size >= data.len()
        let total_cells_needed = data.len().div_ceil(self.cell_size);
        let k = {
            let mut dim = 1usize;
            while dim * dim < total_cells_needed {
                dim += 1;
            }
            dim.max(2) // minimum 2x2
        };

        let ext = 2 * k;

        // Pad data into k*k cells
        let mut padded = data.to_vec();
        padded.resize(k * k * self.cell_size, 0u8);

        // Build the k x k original data grid
        let mut grid: Vec<Vec<Vec<u8>>> = Vec::with_capacity(ext);
        for r in 0..k {
            let mut row = Vec::with_capacity(ext);
            for c in 0..k {
                let start = (r * k + c) * self.cell_size;
                let end = start + self.cell_size;
                row.push(padded[start..end].to_vec());
            }
            grid.push(row);
        }

        // Extend each row from k to 2k using RS
        let row_rs = ErasureEncoder::new(ErasureConfig {
            data_shards: k,
            parity_shards: k,
        })?;

        for row in grid.iter_mut().take(k) {
            let row_data: Vec<u8> = row.iter().flat_map(|c| c.iter().copied()).collect();
            let encoded = row_rs.encode(&row_data)?;
            for p in 0..k {
                let shard = &encoded.shards[k + p];
                let mut cell = vec![0u8; self.cell_size];
                let copy_len = cell.len().min(shard.data.len());
                cell[..copy_len].copy_from_slice(&shard.data[..copy_len]);
                row.push(cell);
            }
        }

        // Now grid has k rows, each with 2k columns.
        // Extend columns: add k parity rows.
        for _ in 0..k {
            let mut new_row = Vec::with_capacity(ext);
            for _ in 0..ext {
                new_row.push(vec![0u8; self.cell_size]);
            }
            grid.push(new_row);
        }

        let col_rs = ErasureEncoder::new(ErasureConfig {
            data_shards: k,
            parity_shards: k,
        })?;

        #[allow(clippy::needless_range_loop)]
        for c in 0..ext {
            let col_data: Vec<u8> = (0..k).flat_map(|r| grid[r][c].iter().copied()).collect();
            let encoded = col_rs.encode(&col_data)?;
            for p in 0..k {
                let shard = &encoded.shards[k + p];
                let mut cell = vec![0u8; self.cell_size];
                let copy_len = cell.len().min(shard.data.len());
                cell[..copy_len].copy_from_slice(&shard.data[..copy_len]);
                grid[k + p][c] = cell;
            }
        }

        // Flatten into cells vector
        let cells: Vec<Vec<u8>> = grid.iter().flatten().cloned().collect();

        Ok(Matrix2D {
            original_dim: k,
            extended_dim: ext,
            cell_size: self.cell_size,
            cells,
        })
    }
}

/// Reconstruct a full row of the 2D extended matrix from partial cells.
///
/// Given a row index and partial cell data (Some = known, None = missing),
/// uses RS(k,k) to recover all 2k cells of that row. At least k cells must
/// be known.
///
/// # Arguments
/// * `matrix` - The original Matrix2D (used for dimension/cell_size metadata)
/// * `row` - Row index in [0, extended_dim)
/// * `known_cells` - Slice of length `extended_dim`, Some(data) for known cells, None for missing
///
/// # Returns
/// A Vec of 2k cell byte-vectors representing the fully reconstructed row.
pub fn reconstruct_row(
    matrix: &Matrix2D,
    row: usize,
    known_cells: &[Option<Vec<u8>>],
) -> Result<Vec<Vec<u8>>, ErasureError> {
    let ext = matrix.extended_dim;
    let k = matrix.original_dim;
    let cell_size = matrix.cell_size;

    if row >= ext {
        return Err(ErasureError::ReedSolomon(format!(
            "row index {} out of range for extended_dim {}",
            row, ext
        )));
    }
    if known_cells.len() != ext {
        return Err(ErasureError::ReedSolomon(format!(
            "known_cells length {} does not match extended_dim {}",
            known_cells.len(),
            ext
        )));
    }

    let rs = ErasureEncoder::new(ErasureConfig {
        data_shards: k,
        parity_shards: k,
    })?;

    // Build the shard vector for reconstruction.
    // Each cell is one shard of size cell_size.
    let shards: Vec<Option<Vec<u8>>> = known_cells
        .iter()
        .map(|opt| {
            opt.as_ref().map(|data| {
                let mut shard = vec![0u8; cell_size];
                let copy_len = cell_size.min(data.len());
                shard[..copy_len].copy_from_slice(&data[..copy_len]);
                shard
            })
        })
        .collect();

    // Reconstruct data shards (first k cells)
    let data_bytes = rs.reconstruct(shards)?;

    // Re-encode to recover all 2k shards (data + parity)
    let encoded = rs.encode(&data_bytes[..k * cell_size])?;

    let mut result = Vec::with_capacity(ext);
    for i in 0..ext {
        let mut cell = vec![0u8; cell_size];
        let copy_len = cell_size.min(encoded.shards[i].data.len());
        cell[..copy_len].copy_from_slice(&encoded.shards[i].data[..copy_len]);
        result.push(cell);
    }

    Ok(result)
}

/// Reconstruct a full column of the 2D extended matrix from partial cells.
///
/// Given a column index and partial cell data (Some = known, None = missing),
/// uses RS(k,k) to recover all 2k cells of that column. At least k cells must
/// be known.
///
/// # Arguments
/// * `matrix` - The original Matrix2D (used for dimension/cell_size metadata)
/// * `col` - Column index in [0, extended_dim)
/// * `known_cells` - Slice of length `extended_dim`, Some(data) for known cells, None for missing
///
/// # Returns
/// A Vec of 2k cell byte-vectors representing the fully reconstructed column.
pub fn reconstruct_column(
    matrix: &Matrix2D,
    col: usize,
    known_cells: &[Option<Vec<u8>>],
) -> Result<Vec<Vec<u8>>, ErasureError> {
    let ext = matrix.extended_dim;
    let k = matrix.original_dim;
    let cell_size = matrix.cell_size;

    if col >= ext {
        return Err(ErasureError::ReedSolomon(format!(
            "column index {} out of range for extended_dim {}",
            col, ext
        )));
    }
    if known_cells.len() != ext {
        return Err(ErasureError::ReedSolomon(format!(
            "known_cells length {} does not match extended_dim {}",
            known_cells.len(),
            ext
        )));
    }

    let rs = ErasureEncoder::new(ErasureConfig {
        data_shards: k,
        parity_shards: k,
    })?;

    // Build the shard vector for reconstruction.
    let shards: Vec<Option<Vec<u8>>> = known_cells
        .iter()
        .map(|opt| {
            opt.as_ref().map(|data| {
                let mut shard = vec![0u8; cell_size];
                let copy_len = cell_size.min(data.len());
                shard[..copy_len].copy_from_slice(&data[..copy_len]);
                shard
            })
        })
        .collect();

    // Reconstruct data shards (first k cells)
    let data_bytes = rs.reconstruct(shards)?;

    // Re-encode to recover all 2k shards (data + parity)
    let encoded = rs.encode(&data_bytes[..k * cell_size])?;

    let mut result = Vec::with_capacity(ext);
    for i in 0..ext {
        let mut cell = vec![0u8; cell_size];
        let copy_len = cell_size.min(encoded.shards[i].data.len());
        cell[..copy_len].copy_from_slice(&encoded.shards[i].data[..copy_len]);
        result.push(cell);
    }

    Ok(result)
}

/// Reconstruct the full 2D extended matrix from sparsely sampled cells.
///
/// Given a set of known cells identified by (row, col) coordinates, iteratively
/// reconstructs rows and columns until the matrix is fully recovered or no
/// further progress can be made.
///
/// Strategy:
/// 1. For each row/column with >= k known cells, reconstruct it fully.
/// 2. Newly recovered cells may enable reconstruction of other rows/columns.
/// 3. Iterate until complete or stalled.
///
/// # Arguments
/// * `dim` - Original dimension k (extended matrix is 2k x 2k)
/// * `cell_size` - Size of each cell in bytes
/// * `known_cells` - Map from (row, col) to cell data for sampled cells
///
/// # Returns
/// A fully reconstructed Matrix2D, or an error if insufficient cells.
#[allow(clippy::needless_range_loop)]
pub fn reconstruct_from_samples(
    dim: usize,
    cell_size: usize,
    known_cells: &HashMap<(usize, usize), Vec<u8>>,
) -> Result<Matrix2D, ErasureError> {
    let k = dim;
    let ext = 2 * k;

    if k < 2 {
        return Err(ErasureError::ReedSolomon(
            "dimension must be at least 2".to_string(),
        ));
    }

    // Build a mutable grid of Option<Vec<u8>>
    let mut grid: Vec<Vec<Option<Vec<u8>>>> = vec![vec![None; ext]; ext];

    for (&(r, c), data) in known_cells {
        if r >= ext || c >= ext {
            return Err(ErasureError::ReedSolomon(format!(
                "cell ({}, {}) out of range for extended_dim {}",
                r, c, ext
            )));
        }
        grid[r][c] = Some(data.clone());
    }

    let rs = ErasureEncoder::new(ErasureConfig {
        data_shards: k,
        parity_shards: k,
    })?;

    let mut progress = true;
    while progress {
        progress = false;

        // Check how many cells are still missing total
        let total_missing: usize = grid
            .iter()
            .flat_map(|row| row.iter())
            .filter(|c| c.is_none())
            .count();
        if total_missing == 0 {
            break;
        }

        // Try to reconstruct each row
        for r in 0..ext {
            let row_known_count = grid[r].iter().filter(|c| c.is_some()).count();
            let row_missing = ext - row_known_count;
            if row_missing == 0 {
                continue; // row already complete
            }
            if row_known_count < k {
                continue; // not enough cells to reconstruct
            }

            // Build shards for this row
            let shards: Vec<Option<Vec<u8>>> = grid[r]
                .iter()
                .map(|opt| {
                    opt.as_ref().map(|data| {
                        let mut shard = vec![0u8; cell_size];
                        let copy_len = cell_size.min(data.len());
                        shard[..copy_len].copy_from_slice(&data[..copy_len]);
                        shard
                    })
                })
                .collect();

            let data_bytes = match rs.reconstruct(shards) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let encoded = match rs.encode(&data_bytes[..k * cell_size]) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for c in 0..ext {
                if grid[r][c].is_none() {
                    let mut cell = vec![0u8; cell_size];
                    let copy_len = cell_size.min(encoded.shards[c].data.len());
                    cell[..copy_len].copy_from_slice(&encoded.shards[c].data[..copy_len]);
                    grid[r][c] = Some(cell);
                    progress = true;
                }
            }
        }

        // Try to reconstruct each column
        for c in 0..ext {
            let col_known_count = (0..ext).filter(|&r| grid[r][c].is_some()).count();
            let col_missing = ext - col_known_count;
            if col_missing == 0 {
                continue;
            }
            if col_known_count < k {
                continue;
            }

            let shards: Vec<Option<Vec<u8>>> = (0..ext)
                .map(|r| {
                    grid[r][c].as_ref().map(|data| {
                        let mut shard = vec![0u8; cell_size];
                        let copy_len = cell_size.min(data.len());
                        shard[..copy_len].copy_from_slice(&data[..copy_len]);
                        shard
                    })
                })
                .collect();

            let data_bytes = match rs.reconstruct(shards) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let encoded = match rs.encode(&data_bytes[..k * cell_size]) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for r in 0..ext {
                if grid[r][c].is_none() {
                    let mut cell = vec![0u8; cell_size];
                    let copy_len = cell_size.min(encoded.shards[r].data.len());
                    cell[..copy_len].copy_from_slice(&encoded.shards[r].data[..copy_len]);
                    grid[r][c] = Some(cell);
                    progress = true;
                }
            }
        }
    }

    // Check if reconstruction is complete
    let missing: usize = grid
        .iter()
        .flat_map(|row| row.iter())
        .filter(|c| c.is_none())
        .count();
    if missing > 0 {
        return Err(ErasureError::NotEnoughShards {
            have: ext * ext - missing,
            need: ext * ext,
        });
    }

    // Flatten into cells
    let mut cells = Vec::with_capacity(ext * ext);
    for r in 0..ext {
        for c in 0..ext {
            cells.push(grid[r][c].take().unwrap());
        }
    }

    Ok(Matrix2D {
        original_dim: k,
        extended_dim: ext,
        cell_size,
        cells,
    })
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_2d_basic() {
        let encoder = ErasureEncoder2D::with_cell_size(32);
        let data = vec![0xABu8; 256];
        let matrix = encoder.encode_2d(&data).unwrap();
        assert!(matrix.extended_dim() >= 4);
        assert_eq!(
            matrix.cells.len(),
            matrix.extended_dim * matrix.extended_dim
        );
    }

    #[test]
    fn test_cell_access() {
        let encoder = ErasureEncoder2D::with_cell_size(16);
        let data = vec![0xCDu8; 128];
        let matrix = encoder.encode_2d(&data).unwrap();
        // All cells within range should be accessible
        for r in 0..matrix.extended_dim {
            for c in 0..matrix.extended_dim {
                assert!(matrix.get_cell(r, c).is_some());
            }
        }
        // Out-of-range should be None
        assert!(matrix.get_cell(matrix.extended_dim, 0).is_none());
    }

    #[test]
    fn test_reconstruct_row_from_partial() {
        let cell_size = 16;
        let encoder = ErasureEncoder2D::with_cell_size(cell_size);
        // 4 * 16 = 64 bytes -> k=2 (2x2 data, 4x4 extended)
        let data = vec![0xAAu8; 64];
        let matrix = encoder.encode_2d(&data).unwrap();
        let ext = matrix.extended_dim;
        let k = matrix.original_dim;

        // For row 0, provide only the first k cells (data shards) and drop parity
        let mut known: Vec<Option<Vec<u8>>> = vec![None; ext];
        for c in 0..k {
            known[c] = Some(matrix.get_cell(0, c).unwrap().to_vec());
        }

        let reconstructed = reconstruct_row(&matrix, 0, &known).unwrap();
        assert_eq!(reconstructed.len(), ext);
        // Verify all cells match the original matrix
        for c in 0..ext {
            assert_eq!(
                reconstructed[c],
                matrix.get_cell(0, c).unwrap(),
                "row 0, col {} mismatch",
                c
            );
        }

        // Also test with parity cells known and some data cells missing
        let mut known2: Vec<Option<Vec<u8>>> = vec![None; ext];
        // Keep last k cells (parity), drop first k (data)
        for c in k..ext {
            known2[c] = Some(matrix.get_cell(0, c).unwrap().to_vec());
        }
        let reconstructed2 = reconstruct_row(&matrix, 0, &known2).unwrap();
        for c in 0..ext {
            assert_eq!(
                reconstructed2[c],
                matrix.get_cell(0, c).unwrap(),
                "row 0 (from parity), col {} mismatch",
                c
            );
        }
    }

    #[test]
    fn test_reconstruct_column_from_partial() {
        let cell_size = 16;
        let encoder = ErasureEncoder2D::with_cell_size(cell_size);
        let data = vec![0xBBu8; 64];
        let matrix = encoder.encode_2d(&data).unwrap();
        let ext = matrix.extended_dim;
        let k = matrix.original_dim;

        // For column 1, provide only the first k cells and drop parity rows
        let mut known: Vec<Option<Vec<u8>>> = vec![None; ext];
        for r in 0..k {
            known[r] = Some(matrix.get_cell(r, 1).unwrap().to_vec());
        }

        let reconstructed = reconstruct_column(&matrix, 1, &known).unwrap();
        assert_eq!(reconstructed.len(), ext);
        for r in 0..ext {
            assert_eq!(
                reconstructed[r],
                matrix.get_cell(r, 1).unwrap(),
                "col 1, row {} mismatch",
                r
            );
        }

        // Test with parity rows known, data rows missing
        let mut known2: Vec<Option<Vec<u8>>> = vec![None; ext];
        for r in k..ext {
            known2[r] = Some(matrix.get_cell(r, 1).unwrap().to_vec());
        }
        let reconstructed2 = reconstruct_column(&matrix, 1, &known2).unwrap();
        for r in 0..ext {
            assert_eq!(
                reconstructed2[r],
                matrix.get_cell(r, 1).unwrap(),
                "col 1 (from parity), row {} mismatch",
                r
            );
        }
    }

    #[test]
    fn test_reconstruct_from_samples_sparse() {
        let cell_size = 16;
        let encoder = ErasureEncoder2D::with_cell_size(cell_size);
        let data = vec![0xCCu8; 64];
        let matrix = encoder.encode_2d(&data).unwrap();
        let ext = matrix.extended_dim;
        let k = matrix.original_dim;

        // Sample enough cells: provide k cells per row (half the columns).
        // This gives every row enough cells to reconstruct, which then fills
        // the whole matrix.
        let mut samples: HashMap<(usize, usize), Vec<u8>> = HashMap::new();
        for r in 0..ext {
            for c in 0..k {
                samples.insert((r, c), matrix.get_cell(r, c).unwrap().to_vec());
            }
        }

        let recovered = reconstruct_from_samples(k, cell_size, &samples).unwrap();
        assert_eq!(recovered.extended_dim, ext);
        assert_eq!(recovered.original_dim, k);

        // Verify all cells match
        for r in 0..ext {
            for c in 0..ext {
                assert_eq!(
                    recovered.get_cell(r, c).unwrap(),
                    matrix.get_cell(r, c).unwrap(),
                    "cell ({}, {}) mismatch after reconstruction",
                    r,
                    c
                );
            }
        }

        // Also test cross-reconstruction: provide k cells per column instead,
        // spread across different rows so columns can be reconstructed first,
        // which then enables row reconstruction.
        let mut col_samples: HashMap<(usize, usize), Vec<u8>> = HashMap::new();
        for c in 0..ext {
            for r in 0..k {
                col_samples.insert((r, c), matrix.get_cell(r, c).unwrap().to_vec());
            }
        }
        let recovered2 = reconstruct_from_samples(k, cell_size, &col_samples).unwrap();
        for r in 0..ext {
            for c in 0..ext {
                assert_eq!(
                    recovered2.get_cell(r, c).unwrap(),
                    matrix.get_cell(r, c).unwrap(),
                );
            }
        }
    }

    #[test]
    fn test_reconstruct_fails_insufficient_cells() {
        let cell_size = 16;
        let encoder = ErasureEncoder2D::with_cell_size(cell_size);
        let data = vec![0xDDu8; 64];
        let matrix = encoder.encode_2d(&data).unwrap();
        let k = matrix.original_dim;

        // Provide only 1 cell — nowhere near enough
        let mut samples: HashMap<(usize, usize), Vec<u8>> = HashMap::new();
        samples.insert((0, 0), matrix.get_cell(0, 0).unwrap().to_vec());

        let result = reconstruct_from_samples(k, cell_size, &samples);
        assert!(result.is_err(), "should fail with only 1 cell");

        // Provide fewer than k cells per every row and column
        // With k=2, ext=4: give 1 cell per row, 1 per column — not enough
        let mut sparse: HashMap<(usize, usize), Vec<u8>> = HashMap::new();
        // Diagonal cells: (0,0), (1,1), (2,2), (3,3) — each row has 1 cell, need 2
        for i in 0..matrix.extended_dim {
            sparse.insert((i, i), matrix.get_cell(i, i).unwrap().to_vec());
        }
        let result2 = reconstruct_from_samples(k, cell_size, &sparse);
        assert!(
            result2.is_err(),
            "diagonal-only sampling should be insufficient for k={}",
            k
        );
    }
}
