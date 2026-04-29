//! `compute_delta_f_millibits` and `compute_refund`.

use thiserror::Error;

use evaporchain_cfm::{crooks_log_ratio_millibits, CrooksError};
use evaporchain_types::Energy;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RefundError {
    #[error("crooks log-ratio error: {0}")]
    Crooks(#[from] CrooksError),
    #[error("β (millibits per fee unit) is zero — Crooks formula undefined")]
    ZeroBeta,
}

/// Compute `ΔF` in millibits, given the work `w_milli`, the
/// observed forward/reverse log-ratio (already in millibits, e.g.
/// from `evaporchain_cfm::crooks_log_ratio_millibits`), and the
/// inverse temperature `β` in millibits per fee unit.
///
/// `ΔF = W − (1/β) · log_ratio` (in millibits throughout).
pub fn compute_delta_f_millibits(
    w_milli: i64,
    log_ratio_milli: i64,
    beta_mb: u64,
) -> Result<i64, RefundError> {
    if beta_mb == 0 {
        return Err(RefundError::ZeroBeta);
    }
    // `(1/β) · log_ratio` in millibits-per-fee-unit / millibits
    // collapses to a dimensionless quotient, then scaled by 1000 to
    // re-express in millibits matching `w_milli`.
    let scaled_log_term = (log_ratio_milli as i128 * 1_000) / beta_mb as i128;
    Ok(w_milli.saturating_sub(scaled_log_term as i64))
}

/// Convenience wrapper: takes raw forward/reverse pmfs (fixed-point)
/// + work + β; returns ΔF in millibits.
pub fn compute_delta_f_from_pmfs(
    p_forward_pmf: u64,
    p_reverse_pmf: u64,
    w_milli: i64,
    beta_mb: u64,
) -> Result<i64, RefundError> {
    let log_ratio = crooks_log_ratio_millibits(p_forward_pmf, p_reverse_pmf)?;
    compute_delta_f_millibits(w_milli, log_ratio, beta_mb)
}

/// Refund = `max(0, work_extracted − delta_f_in_energy_units)`.
/// `delta_f_milli` is interpreted at 1 millibit ≈ 1 energy unit
/// (caller scales if their bit-budget convention differs).
pub fn compute_refund(work_extracted: Energy, delta_f_milli: i64) -> Energy {
    if delta_f_milli <= 0 {
        // ΔF non-positive → all of work_extracted was dissipative MEV.
        return work_extracted;
    }
    let delta_f = delta_f_milli as u64;
    work_extracted.saturating_sub(delta_f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_beta_rejected() {
        assert!(matches!(
            compute_delta_f_millibits(1000, 100, 0).unwrap_err(),
            RefundError::ZeroBeta
        ));
    }

    #[test]
    fn equal_forward_and_reverse_delta_f_equals_work() {
        // log_ratio = 0 ⇒ ΔF = W.
        let df = compute_delta_f_millibits(1000, 0, 100).unwrap();
        assert_eq!(df, 1000);
    }

    #[test]
    fn positive_log_ratio_lowers_delta_f() {
        // log_ratio > 0 (P_F > P_R) ⇒ ΔF < W (dissipative).
        let df = compute_delta_f_millibits(1000, 100, 10).unwrap();
        // W=1000; (1/β)·log_ratio = (1/10)·100 = 10, scaled by 1000 → 10000.
        // ΔF = 1000 − 10000 = -9000.
        assert_eq!(df, -9000);
    }

    #[test]
    fn negative_log_ratio_raises_delta_f() {
        let df = compute_delta_f_millibits(1000, -100, 10).unwrap();
        // (1/β)·log_ratio = (1/10)·(-100) = -10, scaled → -10000.
        // ΔF = 1000 − (-10000) = 11000.
        assert_eq!(df, 11000);
    }

    #[test]
    fn from_pmfs_round_trip() {
        // P_F = 800_000, P_R = 400_000 → log_ratio = 1000 (1 bit).
        let df = compute_delta_f_from_pmfs(800_000, 400_000, 1000, 10).unwrap();
        assert_eq!(df, -99_000);
    }

    #[test]
    fn refund_zero_when_delta_f_exceeds_work() {
        let r = compute_refund(100, 1000);
        assert_eq!(r, 0);
    }

    #[test]
    fn refund_full_work_when_delta_f_negative() {
        let r = compute_refund(1000, -50);
        assert_eq!(r, 1000);
    }

    #[test]
    fn refund_partial_when_delta_f_in_range() {
        let r = compute_refund(1000, 300);
        assert_eq!(r, 700);
    }
}
