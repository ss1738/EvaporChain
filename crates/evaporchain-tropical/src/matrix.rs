//! `TropicalMatrix` — n×n matrix over `TropicalScalar`. Used as the
//! pairwise-distance matrix for n leaves; the `(i, j)` entry is the
//! tropical distance from leaf `i` to leaf `j`.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::scalar::TropicalScalar;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TropicalMatrix {
    n: usize,
    /// Row-major flattened storage; `data[i * n + j]` is entry `(i, j)`.
    data: Vec<TropicalScalar>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MatrixError {
    #[error("index ({i}, {j}) out of bounds for {n}×{n}")]
    OutOfBounds { i: usize, j: usize, n: usize },
}

impl TropicalMatrix {
    /// New `n × n` matrix with all entries set to the tropical zero
    /// (`Infinity`). The diagonal is left at `Infinity` by default;
    /// some primitives may overwrite it (e.g. star-tree distance from
    /// a leaf to itself is conventionally `+∞` or `0`, depending on
    /// the construction).
    pub fn new(n: usize) -> Self {
        Self {
            n,
            data: vec![TropicalScalar::ZERO_T; n * n],
        }
    }

    pub fn dim(&self) -> usize {
        self.n
    }

    pub fn get(&self, i: usize, j: usize) -> TropicalScalar {
        self.data[i * self.n + j]
    }

    pub fn try_get(&self, i: usize, j: usize) -> Result<TropicalScalar, MatrixError> {
        if i >= self.n || j >= self.n {
            return Err(MatrixError::OutOfBounds { i, j, n: self.n });
        }
        Ok(self.get(i, j))
    }

    pub fn set(&mut self, i: usize, j: usize, v: TropicalScalar) {
        self.data[i * self.n + j] = v;
    }

    pub fn try_set(&mut self, i: usize, j: usize, v: TropicalScalar) -> Result<(), MatrixError> {
        if i >= self.n || j >= self.n {
            return Err(MatrixError::OutOfBounds { i, j, n: self.n });
        }
        self.set(i, j, v);
        Ok(())
    }

    /// True iff `M[i][j] == M[j][i]` for all `i, j`. A tree-metric
    /// matrix MUST be symmetric.
    pub fn is_symmetric(&self) -> bool {
        for i in 0..self.n {
            for j in (i + 1)..self.n {
                if self.get(i, j) != self.get(j, i) {
                    return false;
                }
            }
        }
        true
    }

    /// Read-only access to the flat storage, in row-major order. Used
    /// by `commitment::plucker_commitment` to build a canonical hash.
    pub fn raw(&self) -> &[TropicalScalar] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_initializes_to_infinity() {
        let m = TropicalMatrix::new(3);
        for i in 0..3 {
            for j in 0..3 {
                assert_eq!(m.get(i, j), TropicalScalar::Infinity);
            }
        }
    }

    #[test]
    fn set_and_get() {
        let mut m = TropicalMatrix::new(3);
        m.set(0, 1, TropicalScalar::finite(7));
        assert_eq!(m.get(0, 1), TropicalScalar::finite(7));
    }

    #[test]
    fn out_of_bounds_rejected() {
        let m = TropicalMatrix::new(2);
        assert!(matches!(
            m.try_get(2, 0).unwrap_err(),
            MatrixError::OutOfBounds { .. }
        ));
    }

    #[test]
    fn symmetric_check() {
        let mut m = TropicalMatrix::new(2);
        m.set(0, 1, TropicalScalar::finite(1));
        m.set(1, 0, TropicalScalar::finite(1));
        assert!(m.is_symmetric());

        m.set(1, 0, TropicalScalar::finite(2));
        assert!(!m.is_symmetric());
    }

    /// T1.20 — try_get + try_set: in-bounds returns Ok, out-of-
    /// bounds returns OutOfBounds (lines 44-61).
    #[test]
    fn t1_20_matrix_try_get_set_bounds_checked() {
        let mut m = TropicalMatrix::new(2);
        // In-bounds set + get.
        m.try_set(0, 1, TropicalScalar::finite(5)).unwrap();
        assert_eq!(m.try_get(0, 1).unwrap(), TropicalScalar::finite(5));

        // Out-of-bounds get returns Err.
        assert!(m.try_get(2, 0).is_err());
        assert!(m.try_get(0, 2).is_err());

        // Out-of-bounds set returns Err.
        assert!(m.try_set(2, 0, TropicalScalar::finite(1)).is_err());
        assert!(m.try_set(0, 2, TropicalScalar::finite(1)).is_err());
    }
}
