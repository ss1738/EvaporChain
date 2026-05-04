//! Encode: bulk byte → N boundary shares via degree-(k-1)
//! polynomial. Coefficients derived deterministically from a seed.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::field::{add_p, mul_p, FieldElem, MOD_P};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EncodeError {
    #[error("k_threshold must be ≥ 1")]
    ZeroThreshold,
    #[error("k_threshold {k} > n_total {n}")]
    ThresholdAboveTotal { k: usize, n: usize },
    #[error("n_total must be ≥ 1")]
    ZeroTotal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Share {
    /// Boundary index in the range `1..=N` (never 0 — that's the bulk).
    pub index: u64,
    /// Polynomial evaluation at `index`.
    pub value: FieldElem,
    /// Energy at issue. Decays under chain's λ; reconstruction
    /// gate excludes shares below floor.
    pub energy: u64,
}

/// Encode a bulk value as `n_total` shares with reconstruction
/// threshold `k_threshold`.
///
/// Polynomial: `p(x) = bulk + c_1·x + c_2·x² + … + c_{k-1}·x^{k-1}`.
/// Coefficients `c_i` derived from BLAKE3(seed || i) so the same
/// seed produces the same polynomial — validators agree on the
/// share set if they agree on (bulk, seed).
pub fn encode_bulk(
    bulk: FieldElem,
    n_total: usize,
    k_threshold: usize,
    seed: [u8; 32],
    initial_energy: u64,
) -> Result<Vec<Share>, EncodeError> {
    if n_total == 0 {
        return Err(EncodeError::ZeroTotal);
    }
    if k_threshold == 0 {
        return Err(EncodeError::ZeroThreshold);
    }
    if k_threshold > n_total {
        return Err(EncodeError::ThresholdAboveTotal {
            k: k_threshold,
            n: n_total,
        });
    }
    let bulk_p = bulk % MOD_P;
    // Build polynomial coefficients [c_0=bulk, c_1, c_2, …, c_{k-1}].
    let mut coeffs: Vec<FieldElem> = Vec::with_capacity(k_threshold);
    coeffs.push(bulk_p);
    for i in 1..k_threshold {
        let mut h = blake3::Hasher::new();
        h.update(b"evaporchain:happy-code:coeff:v1\0");
        h.update(&seed);
        h.update(&(i as u64).to_le_bytes());
        let bytes: [u8; 32] = *h.finalize().as_bytes();
        let c = u64::from_le_bytes(bytes[..8].try_into().unwrap()) % MOD_P;
        coeffs.push(c);
    }
    // Evaluate at indices 1..=N.
    let mut shares = Vec::with_capacity(n_total);
    for i in 1..=n_total {
        let x = i as u64;
        let v = horner(&coeffs, x);
        shares.push(Share {
            index: x,
            value: v,
            energy: initial_energy,
        });
    }
    Ok(shares)
}

/// Horner's method: evaluate `coeffs` polynomial at `x` over F_p.
pub fn horner(coeffs: &[FieldElem], x: FieldElem) -> FieldElem {
    let mut acc: FieldElem = 0;
    for &c in coeffs.iter().rev() {
        acc = add_p(mul_p(acc, x), c);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_total() {
        let err = encode_bulk(42, 0, 1, [0; 32], 1000).unwrap_err();
        assert_eq!(err, EncodeError::ZeroTotal);
    }

    #[test]
    fn rejects_zero_threshold() {
        let err = encode_bulk(42, 5, 0, [0; 32], 1000).unwrap_err();
        assert_eq!(err, EncodeError::ZeroThreshold);
    }

    #[test]
    fn rejects_threshold_above_total() {
        let err = encode_bulk(42, 3, 5, [0; 32], 1000).unwrap_err();
        assert!(matches!(err, EncodeError::ThresholdAboveTotal { .. }));
    }

    #[test]
    fn encode_produces_n_shares() {
        let shares = encode_bulk(42, 5, 3, [0xAA; 32], 1000).unwrap();
        assert_eq!(shares.len(), 5);
    }

    #[test]
    fn share_indices_are_one_through_n() {
        let shares = encode_bulk(42, 5, 3, [0xAA; 32], 1000).unwrap();
        let indices: Vec<u64> = shares.iter().map(|s| s.index).collect();
        assert_eq!(indices, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn shares_carry_initial_energy() {
        let shares = encode_bulk(42, 5, 3, [0xAA; 32], 777).unwrap();
        for s in &shares {
            assert_eq!(s.energy, 777);
        }
    }

    #[test]
    fn same_seed_same_shares() {
        let a = encode_bulk(42, 5, 3, [0xAA; 32], 1000).unwrap();
        let b = encode_bulk(42, 5, 3, [0xAA; 32], 1000).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn different_seed_different_shares() {
        let a = encode_bulk(42, 5, 3, [0xAA; 32], 1000).unwrap();
        let b = encode_bulk(42, 5, 3, [0xBB; 32], 1000).unwrap();
        // The first coefficient is bulk = 42 in both, so all shares
        // could match if degree 0; but with k=3 the higher-order
        // coefficients differ → values differ.
        assert_ne!(a, b);
    }

    #[test]
    fn k_one_threshold_makes_all_shares_equal_to_bulk() {
        // k=1 → degree-0 polynomial → constant = bulk.
        let shares = encode_bulk(42, 5, 1, [0xAA; 32], 1000).unwrap();
        for s in &shares {
            assert_eq!(s.value, 42);
        }
    }

    #[test]
    fn horner_evaluates_correctly() {
        // p(x) = 1 + 2x + 3x² at x=4: 1 + 8 + 48 = 57.
        let coeffs = vec![1u64, 2, 3];
        assert_eq!(horner(&coeffs, 4), 57);
    }
}
