//! Field arithmetic over the Mersenne-61 prime `p = 2^61 - 1`.
//!
//! ## Why this prime
//!
//! V1 of HLTS production needs a finite-field substrate for Shamir
//! polynomial deal + Lagrange reconstruct. The Mersenne-61 prime is
//! the smallest field that's:
//!
//! - **Big enough** for real-secret demos (61-bit values; for larger
//!   secrets, chunk + share each chunk independently).
//! - **Cheap to operate on** — Mersenne primes have a fast modular
//!   reduction by bitwise mask + conditional subtract; no division.
//! - **Self-contained** — no external crypto dep needed (avoids
//!   pulling `bls12_381` into HLTS until V2).
//!
//! ## V2 path
//!
//! Swap `Scalar(u64)` for `bls12_381::Scalar` (256-bit field). The
//! polynomial / Lagrange algorithms in [`crate::secret`] operate
//! through this module's [`Scalar`] surface; V2 just re-implements
//! [`Scalar`] over the BLS field and the higher layers reuse.
//!
//! ## Security caveat
//!
//! 61-bit fields are NOT secure for production secret-sharing — an
//! attacker holding `k - 1` shares can brute-force the missing share
//! by trying ~2^61 candidates. V1 is for correctness demonstrations
//! and integration with the energy-survival gate; production-grade
//! security needs the V2 BLS upgrade.

use serde::{Deserialize, Serialize};

/// `p = 2^61 - 1`. The Mersenne-61 prime.
pub const PRIME: u64 = (1u64 << 61) - 1;

/// Element of `GF(2^61 - 1)`. Always normalised: `0 <= value < PRIME`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scalar(u64);

impl Scalar {
    /// Wrap a `u64` into the field, reducing if needed.
    #[inline]
    pub const fn from_u64(v: u64) -> Self {
        Self(reduce(v))
    }

    /// The constant `0` in the field.
    pub const ZERO: Self = Self(0);
    /// The constant `1` in the field.
    pub const ONE: Self = Self(1);

    /// Underlying canonical `u64` representation.
    #[inline]
    pub const fn to_u64(self) -> u64 {
        self.0
    }

    /// Field addition. Constant-time-ish (single conditional subtract).
    #[inline]
    pub const fn add(self, other: Self) -> Self {
        let s = self.0.wrapping_add(other.0);
        // Both operands < PRIME, so s < 2*PRIME < 2^62 — no u64 overflow.
        Self(if s >= PRIME { s - PRIME } else { s })
    }

    /// Field subtraction.
    #[inline]
    pub const fn sub(self, other: Self) -> Self {
        let s = self.0.wrapping_add(PRIME).wrapping_sub(other.0);
        Self(if s >= PRIME { s - PRIME } else { s })
    }

    /// Field multiplication via u128 intermediate + Mersenne reduction.
    #[inline]
    pub fn mul(self, other: Self) -> Self {
        let prod: u128 = (self.0 as u128) * (other.0 as u128);
        // Mersenne reduction: x mod (2^61-1) = (x & p) + (x >> 61), at
        // most one subtract to land in [0, p).
        let lo = (prod as u64) & PRIME;
        let hi = (prod >> 61) as u64;
        let s = lo.wrapping_add(hi);
        let s = if s >= PRIME { s - PRIME } else { s };
        // Hi can have bits beyond 61; one more reduction step.
        let lo2 = s & PRIME;
        let hi2 = s >> 61;
        let s2 = lo2.wrapping_add(hi2);
        Self(if s2 >= PRIME { s2 - PRIME } else { s2 })
    }

    /// Negation: `-x mod p`.
    #[inline]
    pub const fn neg(self) -> Self {
        if self.0 == 0 {
            Self(0)
        } else {
            Self(PRIME - self.0)
        }
    }

    /// Modular exponentiation by square-and-multiply.
    pub fn pow(self, mut exp: u64) -> Self {
        let mut result = Self::ONE;
        let mut base = self;
        while exp > 0 {
            if exp & 1 == 1 {
                result = result.mul(base);
            }
            base = base.mul(base);
            exp >>= 1;
        }
        result
    }

    /// Modular inverse via Fermat's little theorem: `x^(p-2) mod p`.
    /// Returns `None` for the zero element (no inverse).
    pub fn inv(self) -> Option<Self> {
        if self.0 == 0 {
            None
        } else {
            Some(self.pow(PRIME - 2))
        }
    }
}

/// Reduce a `u64` value modulo `PRIME` (single subtract is enough
/// since `u64::MAX = 2^64 - 1 < 8 * PRIME`).
#[inline]
const fn reduce(v: u64) -> u64 {
    let lo = v & PRIME;
    let hi = v >> 61;
    let s = lo.wrapping_add(hi);
    if s >= PRIME {
        s - PRIME
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_one_constants() {
        assert_eq!(Scalar::ZERO.to_u64(), 0);
        assert_eq!(Scalar::ONE.to_u64(), 1);
    }

    #[test]
    fn from_u64_reduces_to_canonical_form() {
        assert_eq!(Scalar::from_u64(0).to_u64(), 0);
        assert_eq!(Scalar::from_u64(PRIME - 1).to_u64(), PRIME - 1);
        // PRIME and above wrap.
        assert_eq!(Scalar::from_u64(PRIME).to_u64(), 0);
        assert_eq!(Scalar::from_u64(PRIME + 5).to_u64(), 5);
    }

    #[test]
    fn add_basics() {
        let a = Scalar::from_u64(3);
        let b = Scalar::from_u64(5);
        assert_eq!(a.add(b).to_u64(), 8);
        // Boundary: (p-1) + 1 = 0
        let max = Scalar::from_u64(PRIME - 1);
        assert_eq!(max.add(Scalar::ONE).to_u64(), 0);
    }

    #[test]
    fn sub_basics() {
        let a = Scalar::from_u64(7);
        let b = Scalar::from_u64(3);
        assert_eq!(a.sub(b).to_u64(), 4);
        // 0 - 1 = p - 1
        assert_eq!(Scalar::ZERO.sub(Scalar::ONE).to_u64(), PRIME - 1);
    }

    #[test]
    fn mul_basics() {
        let a = Scalar::from_u64(7);
        let b = Scalar::from_u64(11);
        assert_eq!(a.mul(b).to_u64(), 77);
        // (p-1) * 2 = 2p-2 ≡ -2 ≡ p-2
        let max = Scalar::from_u64(PRIME - 1);
        assert_eq!(max.mul(Scalar::from_u64(2)).to_u64(), PRIME - 2);
    }

    #[test]
    fn mul_wraps_correctly_for_large_operands() {
        // (p-1) * (p-1) = p^2 - 2p + 1 ≡ 1 (mod p)
        let max = Scalar::from_u64(PRIME - 1);
        assert_eq!(max.mul(max).to_u64(), 1);
    }

    #[test]
    fn inv_round_trip() {
        for n in [1u64, 2, 3, 7, 11, 13, 100, 1234567, PRIME / 2, PRIME - 1] {
            let s = Scalar::from_u64(n);
            let inv = s.inv().expect("non-zero inverts");
            assert_eq!(s.mul(inv).to_u64(), 1, "x * x^-1 = 1 for n={n}");
        }
    }

    #[test]
    fn inv_of_zero_is_none() {
        assert_eq!(Scalar::ZERO.inv(), None);
    }

    #[test]
    fn neg_round_trip() {
        let a = Scalar::from_u64(123);
        assert_eq!(a.add(a.neg()).to_u64(), 0);
    }

    #[test]
    fn pow_basics() {
        let a = Scalar::from_u64(3);
        assert_eq!(a.pow(0).to_u64(), 1);
        assert_eq!(a.pow(1).to_u64(), 3);
        assert_eq!(a.pow(2).to_u64(), 9);
        assert_eq!(a.pow(5).to_u64(), 243);
    }
}
