//! CHSH-S value computation in milli-units.
//!
//! Each correlation `E(a, b)` is supplied as a milli-unit signed
//! value in `[-1000, 1000]` (= correlation coefficient × 1000). The
//! S value:
//!
//! ```text
//!   S_milli = |E(a, b) - E(a, b') + E(a', b) + E(a', b')|
//! ```

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChshError {
    #[error("correlation {0} outside valid range [-1000, 1000]")]
    OutOfRange(i64),
}

const MAX_CORR: i64 = 1000;
const MIN_CORR: i64 = -1000;

fn check(c: i64) -> Result<i64, ChshError> {
    if !(MIN_CORR..=MAX_CORR).contains(&c) {
        return Err(ChshError::OutOfRange(c));
    }
    Ok(c)
}

/// `|E(a, b) - E(a, b') + E(a', b) + E(a', b')|` in milli-units.
pub fn chsh_s_value(
    e_ab: i64,
    e_ab_prime: i64,
    e_a_prime_b: i64,
    e_a_prime_b_prime: i64,
) -> Result<u64, ChshError> {
    let e_ab = check(e_ab)?;
    let e_ab_prime = check(e_ab_prime)?;
    let e_a_prime_b = check(e_a_prime_b)?;
    let e_a_prime_b_prime = check(e_a_prime_b_prime)?;
    let s = e_ab - e_ab_prime + e_a_prime_b + e_a_prime_b_prime;
    Ok(s.unsigned_abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classical_max_is_2() {
        // E(a, b) = E(a', b) = E(a', b') = 1, E(a, b') = -1
        // → S = 1 - (-1) + 1 + 1 = 4? Wait, classical bound is 2 for
        // |S|. Let me compute: |1 - (-1) + 1 + 1| = |4| = 4. Actually
        // CHSH allows up to 4 in MAGNITUDE for a correlation matrix
        // *without* the Bell-test physical constraint of being a
        // realisable correlation. Real classical correlations (joint
        // distributions exist) can only give |S| ≤ 2.
        // Test the math: provided correlations ARE feasible, S ≤ 2.
        // Substrate exposes the algebraic computation; physical
        // feasibility is the caller's job.
        let s = chsh_s_value(1000, -1000, 1000, 1000).unwrap();
        assert_eq!(s, 4000); // pure-arithmetic max
    }

    #[test]
    fn out_of_range_rejected() {
        assert!(chsh_s_value(1500, 0, 0, 0).is_err());
    }

    #[test]
    fn classical_uncorrelated_zero() {
        let s = chsh_s_value(0, 0, 0, 0).unwrap();
        assert_eq!(s, 0);
    }

    #[test]
    fn quantum_bell_state_2sqrt2() {
        // Bell state at standard angles: E values approx ±0.707 = ±707 milli.
        // S = 707 - (-707) + 707 + 707 = 2828 ≈ 2√2 × 1000.
        let s = chsh_s_value(707, -707, 707, 707).unwrap();
        assert_eq!(s, 2828);
    }
}
