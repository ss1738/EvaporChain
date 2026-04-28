//! Energy-to-tropical-weight conversion.
//!
//! `tropical_weight(energy) = −log_2(energy)` as a `TropicalScalar`.
//! Higher energy → more-negative weight → "shorter" tropical edge.
//!
//! Integer approximation via bit-length: `log_2(energy) ≈
//! bit_length(energy) − 1` (exact at exact powers of two; off by less
//! than 1 elsewhere). The tropical zero `+∞` represents the absent
//! edge / fully-decayed leaf (`energy = 0`).

use evaporchain_types::Energy;

use crate::scalar::TropicalScalar;

/// Tropical edge weight for an edge labelled by `energy`.
pub fn tropical_weight(energy: Energy) -> TropicalScalar {
    if energy == 0 {
        return TropicalScalar::Infinity;
    }
    // bit_length(n) = 64 - n.leading_zeros(). For n >= 1, this is
    // floor(log_2(n)) + 1. We want -log_2(n) ≈ -(bit_length - 1).
    let bit_length = 64 - energy.leading_zeros() as i64;
    TropicalScalar::finite(-(bit_length - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_of_powers_of_two_is_negative_log() {
        assert_eq!(tropical_weight(1), TropicalScalar::finite(0));
        assert_eq!(tropical_weight(2), TropicalScalar::finite(-1));
        assert_eq!(tropical_weight(4), TropicalScalar::finite(-2));
        assert_eq!(tropical_weight(1024), TropicalScalar::finite(-10));
    }

    #[test]
    fn weight_of_zero_is_infinity() {
        assert_eq!(tropical_weight(0), TropicalScalar::Infinity);
    }

    #[test]
    fn weight_of_max_u64() {
        // bit_length(u64::MAX) = 64 → weight = -63
        assert_eq!(tropical_weight(u64::MAX), TropicalScalar::finite(-63));
    }

    #[test]
    fn higher_energy_yields_smaller_weight() {
        // "Smaller" = more negative = closer to 0_t in the (min, +) sense
        // (for any finite values, smaller is closer to bottom).
        assert!(tropical_weight(1024) < tropical_weight(2));
        assert!(tropical_weight(1024) < tropical_weight(1));
    }
}
