//! Integer Boltzmann weight `~ exp(−β·f)`.
//!
//! Implementation: `2^(−β·f)` via right-shift. Approximation factor is
//! `exp(x) ≈ 2^(x · log_2(e)) = 2^(1.4427 x)`, but at the chain
//! arithmetic granularity we use the simpler `exp(x) ≈ 2^x` — biases
//! the weights conservatively (*understates* the temperature) which
//! keeps the equilibrium distribution closer to the mempool than the
//! exact Boltzmann would.
//!
//! For chain-grade accuracy we'll later swap to a polynomial
//! approximation (degree 4 over a normalised range), but the substrate
//! shape — monotone-decreasing in `β·f`, saturating at 0 for large
//! exponent, equal to `MAX_WEIGHT` at `β·f = 0` — is what consumers
//! depend on and that doesn't change.

/// Maximum returned weight: `2^32` so a typical mempool histogram has
/// enough headroom to distinguish ~10⁹ relative magnitudes before
/// integer floor collapses outcomes.
pub const MAX_WEIGHT: u64 = 1u64 << 32;

/// `boltzmann_weight(fee, beta_mb)` = `2^(−β·f)` in `MAX_WEIGHT` units.
///
/// `beta_mb` is millibits-per-fee-unit (see [`crate::beta`]). The
/// effective shift is `(beta_mb · fee) / 1000` (millibits collapsed to
/// bits). Saturates to `0` once the shift would exhaust the 32 bits of
/// headroom in `MAX_WEIGHT`.
pub fn boltzmann_weight(fee: u64, beta_mb: u64) -> u64 {
    let shift_bits = beta_mb.saturating_mul(fee) / 1_000;
    if shift_bits >= 32 {
        return 0;
    }
    MAX_WEIGHT >> shift_bits as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beta_zero_returns_max_weight() {
        assert_eq!(boltzmann_weight(0, 0), MAX_WEIGHT);
        assert_eq!(boltzmann_weight(1_000, 0), MAX_WEIGHT);
        assert_eq!(boltzmann_weight(u64::MAX, 0), MAX_WEIGHT);
    }

    #[test]
    fn fee_zero_returns_max_weight() {
        assert_eq!(boltzmann_weight(0, 100), MAX_WEIGHT);
    }

    #[test]
    fn weight_decreases_with_fee_at_fixed_beta() {
        let beta_mb = 1_000; // 1 bit per fee unit
        let w1 = boltzmann_weight(1, beta_mb);
        let w2 = boltzmann_weight(2, beta_mb);
        let w4 = boltzmann_weight(4, beta_mb);
        assert!(w1 > w2);
        assert!(w2 > w4);
    }

    #[test]
    fn weight_saturates_to_zero_for_large_exponent() {
        let beta_mb = 1_000_000; // 1000 bits per fee unit
        assert_eq!(boltzmann_weight(1_000, beta_mb), 0);
    }

    #[test]
    fn known_shifts() {
        // shift = 1 → MAX_WEIGHT / 2
        assert_eq!(boltzmann_weight(1, 1_000), MAX_WEIGHT / 2);
        // shift = 4 → MAX_WEIGHT / 16
        assert_eq!(boltzmann_weight(4, 1_000), MAX_WEIGHT / 16);
    }
}
