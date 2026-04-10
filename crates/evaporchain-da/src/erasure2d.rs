//! 2D Reed-Solomon erasure coding for data availability.
//!
//! Arranges data into a square matrix, extends rows and columns with parity,
//! producing a 2D extended matrix suitable for Celestia-style DAS.

use crate::erasure::{ErasureConfig, ErasureEncoder, ErasureError};
use serde::{Deserialize, Serialize};

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
        let total_cells_needed = (data.len() + self.cell_size - 1) / self.cell_size;
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

        for r in 0..k {
            let row_data: Vec<u8> = grid[r].iter().flat_map(|c| c.iter().copied()).collect();
            let encoded = row_rs.encode(&row_data)?;
            // The parity shards are indices k..2k
            for p in 0..k {
                let shard = &encoded.shards[k + p];
                // Shard data may be larger than cell_size due to padding; take cell_size bytes
                let mut cell = vec![0u8; self.cell_size];
                let copy_len = cell.len().min(shard.data.len());
                cell[..copy_len].copy_from_slice(&shard.data[..copy_len]);
                grid[r].push(cell);
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

        for c in 0..ext {
            // Gather column data (rows 0..k)
            let col_data: Vec<u8> = (0..k)
                .flat_map(|r| grid[r][c].iter().copied())
                .collect();
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
        let mut cells = Vec::with_capacity(ext * ext);
        for r in 0..ext {
            for c in 0..ext {
                cells.push(grid[r][c].clone());
            }
        }

        Ok(Matrix2D {
            original_dim: k,
            extended_dim: ext,
            cell_size: self.cell_size,
            cells,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_2d_basic() {
        let encoder = ErasureEncoder2D::with_cell_size(32);
        let data = vec![0xABu8; 256];
        let matrix = encoder.encode_2d(&data).unwrap();
        assert!(matrix.extended_dim() >= 4);
        assert_eq!(matrix.cells.len(), matrix.extended_dim * matrix.extended_dim);
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
}
