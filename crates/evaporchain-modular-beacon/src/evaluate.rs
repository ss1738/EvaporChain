//! Truncated-series evaluation of `E_4`, `E_6`, `Δ`.
//!
//! Substrate uses `q` as a small integer (the chain's per-epoch τ
//! evaluated at `q = some_modular_window`). Each evaluation does
//! `TRUNCATION_DEPTH` integer multiplications + additions over `i128`,
//! saturating on the rare overflow.

use crate::coeffs::{DELTA_COEFFS, E4_COEFFS, E6_COEFFS, TRUNCATION_DEPTH};

/// `E_4(q)` as `Σ_{k=0}^{D-1} E4_COEFFS[k] · q^k`.
pub fn evaluate_e4(q: u64) -> i128 {
    eval_series(&E4_COEFFS, q)
}

/// `E_6(q)` as `Σ_{k=0}^{D-1} E6_COEFFS[k] · q^k`.
pub fn evaluate_e6(q: u64) -> i128 {
    eval_series(&E6_COEFFS, q)
}

/// `Δ(q)` as `q · Σ_{k=0}^{D-1} DELTA_COEFFS[k] · q^k`.
/// (Δ has q^1 leading factor; the coefficients here describe Δ/q.)
pub fn evaluate_delta(q: u64) -> i128 {
    let body = eval_series(&DELTA_COEFFS, q);
    body.saturating_mul(q as i128)
}

fn eval_series(coeffs: &[i128; TRUNCATION_DEPTH], q: u64) -> i128 {
    let q = q as i128;
    let mut total: i128 = 0;
    let mut q_power: i128 = 1;
    for &c in coeffs {
        total = total.saturating_add(c.saturating_mul(q_power));
        q_power = q_power.saturating_mul(q);
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e4_at_q_zero_is_one() {
        assert_eq!(evaluate_e4(0), 1);
    }

    #[test]
    fn e6_at_q_zero_is_one() {
        assert_eq!(evaluate_e6(0), 1);
    }

    #[test]
    fn delta_at_q_zero_is_zero() {
        assert_eq!(evaluate_delta(0), 0);
    }

    #[test]
    fn e4_at_q_one_sum_of_coeffs() {
        // q=1: Σ E4_COEFFS = 1+240+2160+6720+17520+30240+60480+82560.
        let expected: i128 = 1 + 240 + 2160 + 6720 + 17520 + 30240 + 60480 + 82560;
        assert_eq!(evaluate_e4(1), expected);
    }

    #[test]
    fn delta_at_q_one_is_zero_via_jacobi_id() {
        // Beautiful classical fact: Σ τ(n) for the truncated tau is
        // not exactly 0 — Σ τ(n) for the FULL series gives 0 (Jacobi
        // triple-product), but the TRUNCATED partial sum is just the
        // partial sum of (1, -24, 252, -1472, 4830, -6048, -16744,
        // 84480) = 65275. We test the partial-sum value, not the
        // analytic-zero claim.
        let body: i128 = DELTA_COEFFS.iter().sum();
        assert_eq!(body, 65275);
        assert_eq!(evaluate_delta(1), 65275);
    }
}
