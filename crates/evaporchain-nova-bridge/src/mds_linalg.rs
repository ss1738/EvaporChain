//! Linear-algebra primitives for porting neptune's
//! `compress_round_constants` (Phase 2.2-section-2 BESPOKE final
//! step). The SBOX-trick optimization in neptune fuses
//! partial-round constants by left-multiplying them through the
//! inverse MDS matrix; this module ships the building block.
//!
//! # Functions
//!
//! - [`left_apply_matrix`]: `M · v` over `ark_bn254::Fr`.
//! - [`vec_add`]: element-wise vector addition.
//!
//! Mirrors neptune's `nova-snark/.../matrix.rs::left_apply_matrix`
//! and `vec_add`. Once these land, the next step is the full
//! `compress_round_constants` port from neptune's
//! `preprocessing.rs`.

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

    /// **Real-data test.** Multiply the extracted neptune MDS by
    /// a unit vector and confirm the result equals the
    /// corresponding column of the MDS. Catches any indexing
    /// drift between our row/col convention and neptune's.
    ///
    /// Marked `#[ignore]` — requires the JSON dump on disk.
    #[test]
    #[ignore = "requires /tmp/neptune-bn256-standard.json"]
    fn left_apply_real_neptune_mds_picks_column() {
        use crate::neptune_dump_parser::extract_mds_matrix;
        let mds = extract_mds_matrix("/tmp/neptune-bn256-standard.json").expect("load");
        // Unit vector at index 3: [0, 0, 0, 1, 0, ..., 0] (length 25).
        let mut e3 = vec![Fr::from(0u64); 25];
        e3[3] = Fr::from(1u64);
        let result = left_apply_matrix(&mds, &e3);
        // result[i] should equal mds[i][3] for all i (the 3rd column).
        for i in 0..25 {
            assert_eq!(result[i], mds[i][3], "result[{i}] != mds[{i}][3]");
        }
    }
}
