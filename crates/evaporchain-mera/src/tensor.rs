//! Dense real tensor with bond dimension χ = 4.
//!
//! All tensors in this crate are stored as flat `Vec<f64>` in row-major order.
//! Indexing: for a rank-2 tensor T[i][j] the flat index is `i * CHI + j`.

/// Bond dimension for the χ=4 prototype.
pub const CHI: usize = 4;

/// A flat real tensor. Rank and shape are managed by callers.
#[derive(Clone, Debug, PartialEq)]
pub struct Tensor {
    pub data: Vec<f64>,
}

impl Tensor {
    pub fn zeros(size: usize) -> Self {
        Self {
            data: vec![0.0; size],
        }
    }

    pub fn from_vec(data: Vec<f64>) -> Self {
        Self { data }
    }

    pub fn as_bytes(&self) -> Vec<u8> {
        self.data.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    /// L2-norm.
    pub fn norm(&self) -> f64 {
        self.data.iter().map(|x| x * x).sum::<f64>().sqrt()
    }

    /// Normalise in-place; no-op if norm ≈ 0.
    pub fn normalise(&mut self) {
        let n = self.norm();
        if n > 1e-12 {
            for x in &mut self.data {
                *x /= n;
            }
        }
    }
}

/// Encode a u64 energy value as a CHI-dimensional unit vector.
///
/// Uses the polynomial basis: `[1, e/scale, (e/scale)², (e/scale)³]`
/// where `scale = 2^32` (so the coordinates stay in [0, 1] for u64 inputs).
pub fn encode_energy(energy: u64) -> Tensor {
    let e = (energy as f64) / (u32::MAX as f64 + 1.0); // normalise to [0, 1)
    let mut t = Tensor::from_vec(vec![1.0, e, e * e, e * e * e]);
    t.normalise();
    t
}

/// Matrix-vector product: M (CHI×CHI) · v (CHI) → result (CHI).
#[allow(dead_code)]
pub fn mat_vec(m: &[f64], v: &[f64]) -> Vec<f64> {
    assert_eq!(m.len(), CHI * CHI);
    assert_eq!(v.len(), CHI);
    let mut out = vec![0.0f64; CHI];
    for i in 0..CHI {
        for j in 0..CHI {
            out[i] += m[i * CHI + j] * v[j];
        }
    }
    out
}

/// (CHI²×CHI²) matrix · (CHI²)-vector → (CHI²)-vector.
pub fn mat_vec_chi2(m: &[f64], v: &[f64]) -> Vec<f64> {
    let n = CHI * CHI;
    assert_eq!(m.len(), n * n);
    assert_eq!(v.len(), n);
    let mut out = vec![0.0f64; n];
    for i in 0..n {
        for j in 0..n {
            out[i] += m[i * n + j] * v[j];
        }
    }
    out
}

/// Isometry: maps a (CHI²)-vector to a (CHI)-vector by taking the first CHI rows.
/// This is the "keep the top-CHI modes" truncation.
pub fn isometry_apply(m_chi2_x_chi: &[f64], v: &[f64]) -> Vec<f64> {
    assert_eq!(m_chi2_x_chi.len(), CHI * CHI * CHI); // CHI rows × CHI² cols
    assert_eq!(v.len(), CHI * CHI);
    let mut out = vec![0.0f64; CHI];
    for i in 0..CHI {
        for j in 0..(CHI * CHI) {
            out[i] += m_chi2_x_chi[i * (CHI * CHI) + j] * v[j];
        }
    }
    out
}

/// Concatenate two CHI-vectors into a CHI²-vector (tensor product of indices).
pub fn kron_vec(a: &[f64], b: &[f64]) -> Vec<f64> {
    assert_eq!(a.len(), CHI);
    assert_eq!(b.len(), CHI);
    let mut out = vec![0.0f64; CHI * CHI];
    for i in 0..CHI {
        for j in 0..CHI {
            out[i * CHI + j] = a[i] * b[j];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_zero_energy_normalises() {
        let t = encode_energy(0);
        assert!(
            (t.norm() - 1.0).abs() < 1e-10,
            "zero energy should normalise to unit vector"
        );
        assert!(
            (t.data[0] - 1.0).abs() < 1e-10,
            "first component should dominate"
        );
    }

    #[test]
    fn encode_max_energy_normalises() {
        let t = encode_energy(u64::MAX);
        let n = t.norm();
        assert!((n - 1.0).abs() < 1e-10, "max energy unit norm: got {n}");
    }

    #[test]
    fn kron_vec_dimensions() {
        let a = vec![1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0];
        let out = kron_vec(&a, &b);
        assert_eq!(out.len(), CHI * CHI);
        assert_eq!(out[1], 1.0); // a[0]*b[1]
    }

    #[test]
    fn tensor_bytes_roundtrip() {
        let t = Tensor::from_vec(vec![1.0, 2.0, 3.0, 4.0]);
        let b = t.as_bytes();
        assert_eq!(b.len(), 4 * 8);
        let recovered: Vec<f64> = b
            .chunks(8)
            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(recovered, t.data);
    }

    // ─── Additional coverage tests (session 62) ───────────────────────────────

    #[test]
    fn zeros_creates_all_zero_data() {
        let t = Tensor::zeros(4);
        assert_eq!(t.data.len(), 4);
        assert!(t.data.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn zeros_with_different_sizes() {
        assert_eq!(Tensor::zeros(0).data.len(), 0);
        assert_eq!(Tensor::zeros(8).data.len(), 8);
    }

    #[test]
    fn normalise_scales_non_unit_vector_to_unit() {
        // [3,4,0,0] has norm=5; after normalise → [0.6, 0.8, 0, 0]
        let mut t = Tensor::from_vec(vec![3.0, 4.0, 0.0, 0.0]);
        t.normalise();
        assert!((t.norm() - 1.0).abs() < 1e-10, "must be unit after normalise");
        assert!((t.data[0] - 0.6).abs() < 1e-10);
        assert!((t.data[1] - 0.8).abs() < 1e-10);
    }

    #[test]
    fn normalise_near_zero_is_noop() {
        // norm ≈ 0 → if-guard skips division; data unchanged
        let mut t = Tensor::from_vec(vec![0.0, 0.0, 0.0, 0.0]);
        t.normalise();
        assert!(t.data.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn mat_vec_identity_returns_input() {
        #[rustfmt::skip]
        let id = vec![
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];
        let v = vec![1.0, 2.0, 3.0, 4.0];
        let out = mat_vec(&id, &v);
        assert_eq!(out, v);
    }

    #[test]
    fn mat_vec_scaling_matrix() {
        // Diagonal matrix with [2,3,4,5] → each component scaled
        #[rustfmt::skip]
        let m = vec![
            2.0, 0.0, 0.0, 0.0,
            0.0, 3.0, 0.0, 0.0,
            0.0, 0.0, 4.0, 0.0,
            0.0, 0.0, 0.0, 5.0,
        ];
        let v = vec![1.0, 1.0, 1.0, 1.0];
        let out = mat_vec(&m, &v);
        assert_eq!(out, vec![2.0, 3.0, 4.0, 5.0]);
    }
}
