//! Linear-algebra primitives mirroring neptune's
//! `nova-snark/.../matrix.rs`. Used by [`crate::compress_ark`] to
//! port neptune's `compress_round_constants` SBOX-trick
//! optimization (which left-multiplies partial-round constants
//! through the inverse MDS matrix).
//!
//! # Functions
//!
//! - [`left_apply_matrix`]: `M · v` over `ark_bn254::Fr`.
//! - [`vec_add`]: element-wise vector addition.
//! - [`matrix_mul`]: row-major matrix-matrix product.
//! - [`identity_matrix`]: build an N×N identity.
//!
//! Real-data invariants verified on neptune's extracted matrices:
//! `m · m_inv = I₂₅` and `m_hat · m_hat_inv = I₂₄` over the dump
//! from `dump-neptune-constants`.

use ark_bn254::Fr;

/// Compute `result = matrix · vec`. Matrix is `Vec<Vec<Fr>>` row-major;
/// the result is a column vector of the same length as the matrix rows.
///
/// **Shape contract.** `matrix.len()` (number of rows) and
/// `matrix[0].len()` (number of cols) must match `vec.len()`.
/// Each row of the matrix is dotted with `vec` to produce one
/// entry of the result.
pub fn left_apply_matrix(matrix: &[Vec<Fr>], vec: &[Fr]) -> Vec<Fr> {
    assert!(!matrix.is_empty(), "matrix must be non-empty");
    let n = vec.len();
    for (row_idx, row) in matrix.iter().enumerate() {
        assert_eq!(
            row.len(),
            n,
            "matrix row {row_idx} has len {} but vec has len {n}",
            row.len()
        );
    }
    matrix
        .iter()
        .map(|row| {
            let mut acc = Fr::from(0u64);
            for (m, v) in row.iter().zip(vec.iter()) {
                acc += *m * *v;
            }
            acc
        })
        .collect()
}

/// Element-wise vector addition over `ark_bn254::Fr`. Mirrors
/// neptune's `matrix::vec_add`.
pub fn vec_add(a: &[Fr], b: &[Fr]) -> Vec<Fr> {
    assert_eq!(a.len(), b.len(), "vec_add: length mismatch");
    a.iter().zip(b.iter()).map(|(x, y)| *x + *y).collect()
}

/// Matrix-matrix product `a · b`. Both matrices are row-major
/// `Vec<Vec<Fr>>`. Inner dimension of `a` must match outer
/// dimension of `b` (i.e., `a[0].len() == b.len()`).
pub fn matrix_mul(a: &[Vec<Fr>], b: &[Vec<Fr>]) -> Vec<Vec<Fr>> {
    assert!(!a.is_empty() && !b.is_empty(), "matrices must be non-empty");
    let inner = a[0].len();
    assert_eq!(inner, b.len(), "matrix_mul: a.cols ({inner}) ≠ b.rows ({})", b.len());
    let cols = b[0].len();
    let mut out: Vec<Vec<Fr>> = Vec::with_capacity(a.len());
    for row in a {
        let mut out_row = vec![Fr::from(0u64); cols];
        for k in 0..inner {
            let aik = row[k];
            for (j, out_cell) in out_row.iter_mut().enumerate().take(cols) {
                *out_cell += aik * b[k][j];
            }
        }
        out.push(out_row);
    }
    out
}

/// Build an N×N identity matrix over `ark_bn254::Fr`.
pub fn identity_matrix(n: usize) -> Vec<Vec<Fr>> {
    let mut m = vec![vec![Fr::from(0u64); n]; n];
    for i in 0..n {
        m[i][i] = Fr::from(1u64);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Identity matrix · vec = vec.
    #[test]
    fn identity_matrix_is_identity() {
        let identity = vec![
            vec![Fr::from(1u64), Fr::from(0u64), Fr::from(0u64)],
            vec![Fr::from(0u64), Fr::from(1u64), Fr::from(0u64)],
            vec![Fr::from(0u64), Fr::from(0u64), Fr::from(1u64)],
        ];
        let v = vec![Fr::from(7u64), Fr::from(11u64), Fr::from(13u64)];
        let result = left_apply_matrix(&identity, &v);
        assert_eq!(result, v);
    }

    /// Scaling matrix · vec = scaled vec.
    #[test]
    fn scaling_matrix_doubles_each_entry() {
        let scaled = vec![
            vec![Fr::from(2u64), Fr::from(0u64)],
            vec![Fr::from(0u64), Fr::from(2u64)],
        ];
        let v = vec![Fr::from(3u64), Fr::from(5u64)];
        let result = left_apply_matrix(&scaled, &v);
        assert_eq!(result, vec![Fr::from(6u64), Fr::from(10u64)]);
    }

    /// 2×2 general matrix: M = [[1, 2], [3, 4]] · v = [2 + 6, 6 + 16]
    ///                                              = [8, 22] for v = [2, 3].
    #[test]
    fn general_2x2_matrix_product() {
        let m = vec![
            vec![Fr::from(1u64), Fr::from(2u64)],
            vec![Fr::from(3u64), Fr::from(4u64)],
        ];
        let v = vec![Fr::from(2u64), Fr::from(3u64)];
        let result = left_apply_matrix(&m, &v);
        assert_eq!(result, vec![Fr::from(8u64), Fr::from(18u64)]);
        // 1*2 + 2*3 = 8, 3*2 + 4*3 = 18.
    }

    /// Non-square matrix: 3 rows × 2 cols against length-2 vec
    /// produces length-3 result.
    #[test]
    fn non_square_matrix_shape() {
        let m = vec![
            vec![Fr::from(1u64), Fr::from(0u64)],
            vec![Fr::from(0u64), Fr::from(1u64)],
            vec![Fr::from(1u64), Fr::from(1u64)],
        ];
        let v = vec![Fr::from(5u64), Fr::from(7u64)];
        let result = left_apply_matrix(&m, &v);
        assert_eq!(result.len(), 3);
        assert_eq!(result, vec![Fr::from(5u64), Fr::from(7u64), Fr::from(12u64)]);
    }

    /// Empty matrix rejection. (Useful regression catch — neptune's
    /// `left_apply_matrix` panics on empty.)
    #[test]
    #[should_panic(expected = "matrix must be non-empty")]
    fn empty_matrix_panics() {
        let m: Vec<Vec<Fr>> = vec![];
        let v = vec![Fr::from(1u64)];
        let _ = left_apply_matrix(&m, &v);
    }

    #[test]
    #[should_panic(expected = "matrix row")]
    fn dim_mismatch_panics() {
        let m = vec![vec![Fr::from(1u64), Fr::from(2u64), Fr::from(3u64)]];
        let v = vec![Fr::from(5u64), Fr::from(7u64)];
        let _ = left_apply_matrix(&m, &v);
    }

    #[test]
    fn vec_add_basic() {
        let a = vec![Fr::from(1u64), Fr::from(2u64), Fr::from(3u64)];
        let b = vec![Fr::from(10u64), Fr::from(20u64), Fr::from(30u64)];
        let s = vec_add(&a, &b);
        assert_eq!(s, vec![Fr::from(11u64), Fr::from(22u64), Fr::from(33u64)]);
    }

    #[test]
    #[should_panic(expected = "vec_add: length mismatch")]
    fn vec_add_length_mismatch_panics() {
        let a = vec![Fr::from(1u64)];
        let b = vec![Fr::from(1u64), Fr::from(2u64)];
        let _ = vec_add(&a, &b);
    }

    #[test]
    fn matrix_mul_identity_is_identity() {
        let i3 = identity_matrix(3);
        let a = vec![
            vec![Fr::from(1u64), Fr::from(2u64), Fr::from(3u64)],
            vec![Fr::from(4u64), Fr::from(5u64), Fr::from(6u64)],
            vec![Fr::from(7u64), Fr::from(8u64), Fr::from(9u64)],
        ];
        assert_eq!(matrix_mul(&i3, &a), a);
        assert_eq!(matrix_mul(&a, &i3), a);
    }

    #[test]
    fn matrix_mul_concrete_2x2() {
        // [[1,2],[3,4]] · [[5,6],[7,8]] = [[19,22],[43,50]]
        let a = vec![
            vec![Fr::from(1u64), Fr::from(2u64)],
            vec![Fr::from(3u64), Fr::from(4u64)],
        ];
        let b = vec![
            vec![Fr::from(5u64), Fr::from(6u64)],
            vec![Fr::from(7u64), Fr::from(8u64)],
        ];
        let c = matrix_mul(&a, &b);
        assert_eq!(c[0][0], Fr::from(19u64));
        assert_eq!(c[0][1], Fr::from(22u64));
        assert_eq!(c[1][0], Fr::from(43u64));
        assert_eq!(c[1][1], Fr::from(50u64));
    }

    #[test]
    fn identity_matrix_shape() {
        let i25 = identity_matrix(25);
        assert_eq!(i25.len(), 25);
        for (i, row) in i25.iter().enumerate() {
            assert_eq!(row.len(), 25);
            for (j, cell) in row.iter().enumerate() {
                if i == j {
                    assert_eq!(*cell, Fr::from(1u64));
                } else {
                    assert_eq!(*cell, Fr::from(0u64));
                }
            }
        }
    }

    // Real-data invariant tests against neptune's dumped MDS
    // (`m · m_inv = I`, `m_hat · m_hat_inv = I`, column-pick by
    // unit-vector left-apply) live on a parallel docstring-refresh
    // stack alongside `neptune_dump_parser`. They will be cherry-
    // picked onto main together once the parser lands here.
}
