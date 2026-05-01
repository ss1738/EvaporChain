//! `TropicalScalar` — element of the (min, +) semiring over `i64`,
//! plus an explicit `Infinity` for the tropical zero.
//!
//! Identities:
//!
//! - **Tropical zero** `0_t = +∞`. Acts as the additive identity:
//!   `min(x, +∞) = x` for all finite `x`.
//! - **Tropical one** `1_t = 0`. Acts as the multiplicative identity:
//!   `0 + x = x` for all `x`.
//!
//! Tropical operations:
//!
//! - `a ⊕ b = min(a, b)` (tropical addition).
//! - `a ⊗ b = a + b` (tropical multiplication; saturating on `i64`
//!   overflow, with `Infinity` absorbing).

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TropicalScalar {
    Finite(i64),
    Infinity,
}

impl TropicalScalar {
    pub const ZERO_T: TropicalScalar = TropicalScalar::Infinity;
    pub const ONE_T: TropicalScalar = TropicalScalar::Finite(0);

    pub const fn finite(x: i64) -> Self {
        Self::Finite(x)
    }

    pub const fn is_infinity(self) -> bool {
        matches!(self, TropicalScalar::Infinity)
    }

    /// Tropical addition: `min(a, b)`.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, other: Self) -> Self {
        match (self, other) {
            (TropicalScalar::Infinity, x) | (x, TropicalScalar::Infinity) => x,
            (TropicalScalar::Finite(a), TropicalScalar::Finite(b)) => {
                TropicalScalar::Finite(a.min(b))
            }
        }
    }

    /// Tropical multiplication: `a + b` on the finite values; `Infinity`
    /// absorbs (the tropical "zero times anything is zero").
    #[allow(clippy::should_implement_trait)]
    pub fn mul(self, other: Self) -> Self {
        match (self, other) {
            (TropicalScalar::Infinity, _) | (_, TropicalScalar::Infinity) => {
                TropicalScalar::Infinity
            }
            (TropicalScalar::Finite(a), TropicalScalar::Finite(b)) => {
                TropicalScalar::Finite(a.saturating_add(b))
            }
        }
    }
}

impl Ord for TropicalScalar {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (TropicalScalar::Infinity, TropicalScalar::Infinity) => Ordering::Equal,
            (TropicalScalar::Infinity, _) => Ordering::Greater,
            (_, TropicalScalar::Infinity) => Ordering::Less,
            (TropicalScalar::Finite(a), TropicalScalar::Finite(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for TropicalScalar {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_is_min() {
        assert_eq!(
            TropicalScalar::finite(3).add(TropicalScalar::finite(5)),
            TropicalScalar::finite(3)
        );
        assert_eq!(
            TropicalScalar::finite(-2).add(TropicalScalar::finite(7)),
            TropicalScalar::finite(-2)
        );
    }

    #[test]
    fn mul_is_plus() {
        assert_eq!(
            TropicalScalar::finite(3).mul(TropicalScalar::finite(5)),
            TropicalScalar::finite(8)
        );
    }

    #[test]
    fn zero_t_is_additive_identity() {
        let x = TropicalScalar::finite(42);
        assert_eq!(x.add(TropicalScalar::ZERO_T), x);
        assert_eq!(TropicalScalar::ZERO_T.add(x), x);
    }

    #[test]
    fn one_t_is_multiplicative_identity() {
        let x = TropicalScalar::finite(42);
        assert_eq!(x.mul(TropicalScalar::ONE_T), x);
        assert_eq!(TropicalScalar::ONE_T.mul(x), x);
    }

    #[test]
    fn infinity_absorbs_under_mul() {
        let x = TropicalScalar::finite(7);
        assert_eq!(x.mul(TropicalScalar::ZERO_T), TropicalScalar::ZERO_T);
        assert_eq!(TropicalScalar::ZERO_T.mul(x), TropicalScalar::ZERO_T);
    }

    #[test]
    fn ord_infinity_is_largest() {
        assert!(TropicalScalar::Infinity > TropicalScalar::finite(i64::MAX));
        assert_eq!(TropicalScalar::Infinity, TropicalScalar::Infinity);
    }

    #[test]
    fn mul_saturates_on_overflow() {
        let big = TropicalScalar::finite(i64::MAX);
        let result = big.mul(TropicalScalar::finite(1));
        assert_eq!(result, TropicalScalar::finite(i64::MAX));
    }
}
