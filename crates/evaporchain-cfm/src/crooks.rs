//! Crooks 1999 fluctuation theorem identity.
//!
//! For forward and reverse work distributions `P_F`, `P_R`:
//!
//! ```text
//!   log( P_F(W) / P_R(−W) ) = β · (W − ΔF)
//! ```
//!
//! On EvaporChain this gives an *exact equality* (not a bound) between
//! transactional "work" (energy paid as fees) and the chain's "free
//! energy difference" `ΔF`. The substrate just exposes the identity:
//! consumers compute the LHS from observed forward/reverse pmfs and
//! compare to the RHS to detect equilibrium violations.
//!
//! All values are in millibits (consistent with `kl_millibits` and
//! `beta_millibits_per_fee`).

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CrooksError {
    #[error("forward probability is zero — log undefined")]
    ForwardZero,
    #[error("reverse probability is zero — log undefined")]
    ReverseZero,
}

/// `log_2(p_forward / p_reverse)` in millibits, given fixed-point pmf
/// values (FIXED_POINT_SCALE units). Sign is preserved (negative when
/// reverse > forward, positive otherwise).
pub fn crooks_log_ratio_millibits(
    p_forward: u64,
    p_reverse: u64,
) -> Result<i64, CrooksError> {
    if p_forward == 0 {
        return Err(CrooksError::ForwardZero);
    }
    if p_reverse == 0 {
        return Err(CrooksError::ReverseZero);
    }
    let f_bits = bit_length(p_forward) as i64;
    let r_bits = bit_length(p_reverse) as i64;
    Ok((f_bits - r_bits) * 1_000)
}

fn bit_length(n: u64) -> u64 {
    if n == 0 {
        0
    } else {
        64 - n.leading_zeros() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_forward_and_reverse_zero_log_ratio() {
        assert_eq!(crooks_log_ratio_millibits(500_000, 500_000).unwrap(), 0);
    }

    #[test]
    fn forward_double_reverse_one_bit() {
        // bit_length(800_000) = 20, bit_length(400_000) = 19 → diff = 1 bit
        assert_eq!(crooks_log_ratio_millibits(800_000, 400_000).unwrap(), 1_000);
    }

    #[test]
    fn reverse_double_forward_negative_one_bit() {
        assert_eq!(crooks_log_ratio_millibits(400_000, 800_000).unwrap(), -1_000);
    }

    #[test]
    fn zero_forward_rejected() {
        assert_eq!(
            crooks_log_ratio_millibits(0, 100).unwrap_err(),
            CrooksError::ForwardZero
        );
    }

    #[test]
    fn zero_reverse_rejected() {
        assert_eq!(
            crooks_log_ratio_millibits(100, 0).unwrap_err(),
            CrooksError::ReverseZero
        );
    }
}
