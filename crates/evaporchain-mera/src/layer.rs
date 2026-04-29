//! MERA layer: disentangler + isometry, both parameterised by λ and layer index ℓ.
//!
//! # Parameterisation (§A1.4)
//!
//! Disentangler W(λ, ℓ):
//!   A CHI²×CHI² orthogonal matrix derived deterministically from
//!   `seed = blake3(λ_bytes || ℓ_bytes)`.  Built as a product of
//!   Givens rotations with angles drawn from the seed.  The rotation angle
//!   grows with layer depth: `θ_k = seed_byte_k / 255 * π * (ℓ+1) / λ_scale`.
//!
//! Isometry U(λ, ℓ):
//!   The first CHI rows of W (the "top-CHI modes" after disentangling).
//!   Half-life for layer ℓ: τ₀·2^ℓ.  Accounts at layer ℓ whose energy
//!   has decayed below the layer threshold are "isometrically removed" —
//!   they contribute zero to higher layers, matching the energy filtration.
//!
//! # Energy filtration
//!
//! Layer ℓ corresponds to the energy scale E·(1/2)^ℓ.  Accounts with
//! energy below this threshold are masked to zero before the disentangler,
//! so the RG flow *is* the energy filtration (per §A1.4).

use crate::tensor::{isometry_apply, kron_vec, mat_vec_chi2, Tensor, CHI};

/// One layer of the MERA tensor network.
#[derive(Clone, Debug)]
pub struct MeraLayer {
    /// Layer index ℓ (0 = physical layer).
    pub index: usize,
    /// Half-life for this layer: τ₀ · 2^ℓ.
    pub half_life: u64,
    /// Disentangler: CHI²×CHI² orthogonal matrix (flat, row-major).
    pub disentangler: Vec<f64>,
    /// Isometry: CHI×CHI² matrix (flat, row-major).
    pub isometry: Vec<f64>,
}

impl MeraLayer {
    /// Build a deterministic layer from `lambda_half_life` and layer index.
    ///
    /// Tensors are seeded via blake3 so they are reproducible across nodes
    /// given the same λ — essential for consensus.
    pub fn new(lambda_half_life: u64, base_half_life: u64, index: usize) -> Self {
        let half_life = base_half_life.saturating_mul(1u64 << index.min(62));
        let seed = Self::seed(lambda_half_life, index);
        let disentangler = build_disentangler(&seed, index);
        let isometry = build_isometry(&disentangler);
        Self { index, half_life, disentangler, isometry }
    }

    /// Apply (disentangler ∘ isometry) to a pair of CHI-vectors,
    /// producing a single coarse-grained CHI-vector.
    ///
    /// Optionally zeroes the input if either account's energy is below
    /// the layer energy threshold (energy filtration).
    pub fn apply(&self, left: &[f64], right: &[f64], filter_threshold: Option<f64>) -> Vec<f64> {
        let mut kv = kron_vec(left, right); // CHI² vector

        // Energy filtration: if both halves are near-zero post-threshold, zero the input.
        if let Some(thresh) = filter_threshold {
            let left_norm: f64 = left.iter().map(|x| x * x).sum::<f64>().sqrt();
            let right_norm: f64 = right.iter().map(|x| x * x).sum::<f64>().sqrt();
            if left_norm < thresh && right_norm < thresh {
                return vec![0.0; CHI];
            }
        }

        // Disentangle: W · kv
        let disentangled = mat_vec_chi2(&self.disentangler, &kv);

        // Normalise before isometry (numerical stability).
        let norm: f64 = disentangled.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm > 1e-12 {
            let inv = 1.0 / norm;
            let renorm: Vec<f64> = disentangled.iter().map(|x| x * inv).collect();
            // Isometry: U · renorm → CHI vector
            let mut out = isometry_apply(&self.isometry, &renorm);
            // Normalise output.
            let on: f64 = out.iter().map(|x| x * x).sum::<f64>().sqrt();
            if on > 1e-12 {
                for x in &mut out {
                    *x /= on;
                }
            }
            return out;
        }
        vec![0.0; CHI]
    }

    fn seed(lambda_half_life: u64, layer: usize) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"evaporchain-mera-layer-v1");
        hasher.update(&lambda_half_life.to_le_bytes());
        hasher.update(&(layer as u64).to_le_bytes());
        *hasher.finalize().as_bytes()
    }
}

/// Build a CHI²×CHI² orthogonal matrix as a product of Givens rotations.
///
/// Each pair of axes (i, j) where i < j gets a Givens rotation with angle
/// derived from the seed bytes.  This guarantees the matrix is orthogonal
/// by construction without any QR decomposition.
fn build_disentangler(seed: &[u8; 32], layer_index: usize) -> Vec<f64> {
    let n = CHI * CHI; // 16
    // Start from identity.
    let mut m: Vec<f64> = (0..n * n).map(|k| if k % (n + 1) == 0 { 1.0 } else { 0.0 }).collect();

    let mut byte_idx = 0usize;
    for i in 0..n {
        for j in (i + 1)..n {
            // Angle in [0, π] scaled by (layer+1) / lambda_scale.
            let raw = seed[byte_idx % 32] as f64 / 255.0;
            let theta = raw * std::f64::consts::PI * (layer_index + 1) as f64 / 8.0;
            let (s, c) = theta.sin_cos();
            givens_inplace(&mut m, n, i, j, s, c);
            byte_idx += 1;
        }
    }
    m
}

/// Apply Givens rotation G(i,j,θ) in-place to matrix m (n×n).
fn givens_inplace(m: &mut Vec<f64>, n: usize, i: usize, j: usize, s: f64, c: f64) {
    for k in 0..n {
        let ri = m[k * n + i];
        let rj = m[k * n + j];
        m[k * n + i] =  c * ri + s * rj;
        m[k * n + j] = -s * ri + c * rj;
    }
}

/// Isometry = first CHI rows of the disentangler (CHI × CHI² matrix).
fn build_isometry(disentangler: &[f64]) -> Vec<f64> {
    let n = CHI * CHI; // 16
    // disentangler is (n×n); isometry = first CHI rows = CHI×n.
    disentangler[..CHI * n].to_vec()
}

/// Hash a layer's tensors to a 32-byte digest.
pub fn hash_layer(layer: &MeraLayer) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&(layer.index as u64).to_le_bytes());
    hasher.update(&layer.half_life.to_le_bytes());
    for f in &layer.disentangler {
        hasher.update(&f.to_le_bytes());
    }
    *hasher.finalize().as_bytes()
}

/// Hash a site tensor (CHI-vector) to a 32-byte digest.
pub fn hash_site(tensor: &Tensor) -> [u8; 32] {
    *blake3::hash(&tensor.as_bytes()).as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_zero_is_deterministic() {
        let l1 = MeraLayer::new(4096, 100, 0);
        let l2 = MeraLayer::new(4096, 100, 0);
        assert_eq!(l1.disentangler, l2.disentangler);
    }

    #[test]
    fn different_lambda_different_layer() {
        let l1 = MeraLayer::new(4096, 100, 0);
        let l2 = MeraLayer::new(2048, 100, 0);
        assert_ne!(l1.disentangler, l2.disentangler);
    }

    #[test]
    fn different_depth_different_layer() {
        let l1 = MeraLayer::new(4096, 100, 0);
        let l2 = MeraLayer::new(4096, 100, 1);
        assert_ne!(l1.disentangler, l2.disentangler);
    }

    #[test]
    fn apply_produces_chi_vector() {
        let layer = MeraLayer::new(4096, 100, 0);
        let a = vec![1.0, 0.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0, 0.0];
        let out = layer.apply(&a, &b, None);
        assert_eq!(out.len(), CHI);
    }

    #[test]
    fn apply_zero_inputs_zero_output() {
        let layer = MeraLayer::new(4096, 100, 0);
        let z = vec![0.0; CHI];
        let out = layer.apply(&z, &z, None);
        assert!(out.iter().all(|x| x.abs() < 1e-10));
    }

    #[test]
    fn half_life_doubles_per_layer() {
        let base = 100u64;
        for ℓ in 0..4usize {
            let l = MeraLayer::new(4096, base, ℓ);
            assert_eq!(l.half_life, base * (1u64 << ℓ));
        }
    }
}
