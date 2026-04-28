//! Precomputed first-`TRUNCATION_DEPTH` q-expansion coefficients of
//! `E_4`, `E_6`, and Δ.
//!
//! Reference: Zagier, *Elliptic Modular Forms and Their Applications*,
//! 1-2-3 of Modular Forms (Springer 2008), §1.1.
//!
//! - `E_4(q)`   = `1 + 240·q + 2160·q² + 6720·q³ + 17520·q⁴ + 30240·q⁵ + 60480·q⁶ + 82560·q⁷`
//! - `E_6(q)`   = `1 - 504·q - 16632·q² - 122976·q³ - 532728·q⁴ - 1575504·q⁵ - 4058208·q⁶ - 8471232·q⁷`
//! - `Δ(q)`/q  = `1 - 24·q + 252·q² - 1472·q³ + 4830·q⁴ - 6048·q⁵ - 16744·q⁶ + 84480·q⁷`
//!
//! (Δ is conventionally written `Δ = q · ∏(1-q^n)^24`. We store the
//! Ramanujan-tau coefficient sequence τ(n) starting at index 0 = τ(1).)

pub const TRUNCATION_DEPTH: usize = 8;

/// Coefficients of the q-expansion of `E_4`. `E4_COEFFS[k]` is the
/// coefficient of `q^k`.
pub const E4_COEFFS: [i128; TRUNCATION_DEPTH] = [
    1, 240, 2160, 6720, 17520, 30240, 60480, 82560,
];

/// Coefficients of the q-expansion of `E_6`. `E6_COEFFS[k]` is the
/// coefficient of `q^k`. Note negative leading correction.
pub const E6_COEFFS: [i128; TRUNCATION_DEPTH] = [
    1, -504, -16632, -122976, -532728, -1575504, -4058208, -8471232,
];

/// Coefficients of the q-expansion of `Δ / q`. `DELTA_COEFFS[k]` is
/// the coefficient of `q^k` in `Δ(q) / q`. The Ramanujan tau-function
/// `τ(n) = DELTA_COEFFS[n - 1]`.
pub const DELTA_COEFFS: [i128; TRUNCATION_DEPTH] = [
    1, -24, 252, -1472, 4830, -6048, -16744, 84480,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e4_coeff_zero_is_one() {
        assert_eq!(E4_COEFFS[0], 1);
    }

    #[test]
    fn e6_coeff_zero_is_one() {
        assert_eq!(E6_COEFFS[0], 1);
    }

    #[test]
    fn delta_q1_coefficient_is_one() {
        // Δ = q + ... , so Δ/q starts at 1.
        assert_eq!(DELTA_COEFFS[0], 1);
    }

    #[test]
    fn ramanujan_tau_5_known_value() {
        // τ(5) = 4830 (Ramanujan 1916).
        assert_eq!(DELTA_COEFFS[4], 4830);
    }

    #[test]
    fn ramanujan_tau_2_is_minus_24() {
        assert_eq!(DELTA_COEFFS[1], -24);
    }
}
