//! `cmu_bound` and `cmu_check` — the Shalizi-Crutchfield identity
//! `Cμ ≤ E + hμ` as a chain-side gate.
//!
//! Inputs are in millibits (consistent with the rest of the
//! workspace's information-theory estimators). `cmu_check` returns
//! `Verdict::Ok` iff the observed `Cμ` is at or below the bound.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Ok { observed_cmu: u64, bound: u64 },
    Violation { observed_cmu: u64, bound: u64 },
}

/// `bound = E + hμ` in millibits. Saturating to avoid `u64` overflow
/// at extreme values.
pub fn cmu_bound(excess_entropy_mb: u64, entropy_rate_mb: u64) -> u64 {
    excess_entropy_mb.saturating_add(entropy_rate_mb)
}

/// `cmu_check(Cμ, E, hμ)` — returns `Verdict::Ok` iff `Cμ ≤ E + hμ`.
pub fn cmu_check(observed_cmu_mb: u64, excess_entropy_mb: u64, entropy_rate_mb: u64) -> Verdict {
    let bound = cmu_bound(excess_entropy_mb, entropy_rate_mb);
    if observed_cmu_mb <= bound {
        Verdict::Ok {
            observed_cmu: observed_cmu_mb,
            bound,
        }
    } else {
        Verdict::Violation {
            observed_cmu: observed_cmu_mb,
            bound,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bound_is_sum() {
        assert_eq!(cmu_bound(100, 200), 300);
    }

    #[test]
    fn cmu_at_bound_is_ok() {
        let v = cmu_check(300, 100, 200);
        assert!(matches!(v, Verdict::Ok { .. }));
    }

    #[test]
    fn cmu_below_bound_is_ok() {
        let v = cmu_check(250, 100, 200);
        assert!(matches!(v, Verdict::Ok { .. }));
    }

    #[test]
    fn cmu_above_bound_is_violation() {
        let v = cmu_check(301, 100, 200);
        assert!(matches!(v, Verdict::Violation { .. }));
    }

    #[test]
    fn zero_inputs_zero_bound_zero_cmu_ok() {
        assert!(matches!(cmu_check(0, 0, 0), Verdict::Ok { .. }));
    }

    #[test]
    fn zero_inputs_positive_cmu_violation() {
        assert!(matches!(cmu_check(1, 0, 0), Verdict::Violation { .. }));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// `cmu_check` is monotone: if Cμ ≤ Cμ' and the bound is
        /// fixed, an Ok at Cμ' implies Ok at Cμ.
        #[test]
        fn ok_is_downward_closed(
            cmu in 0u64..1_000_000,
            extra in 0u64..1_000_000,
            e in 0u64..1_000_000,
            h in 0u64..1_000_000,
        ) {
            let v_low = cmu_check(cmu, e, h);
            let v_high = cmu_check(cmu.saturating_add(extra), e, h);
            // If the higher Cμ is Ok, the lower one MUST also be Ok.
            let high_ok = matches!(v_high, Verdict::Ok { .. });
            let low_ok = matches!(v_low, Verdict::Ok { .. });
            if high_ok {
                prop_assert!(low_ok);
            }
        }
    }
}
