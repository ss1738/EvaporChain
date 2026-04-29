//! `bridge_valid` — the cross-chain bridge gate.

use crate::cone::EnergyCone;

/// True iff both `cone_a` and `cone_b` contain `query_epoch`. The
/// bridge tx is valid only inside this *intersection* — replay-immune
/// because each chain's cone collapses on its own λ schedule.
pub fn bridge_valid(cone_a: &EnergyCone, cone_b: &EnergyCone, query_epoch: u64) -> bool {
    cone_a.is_inside(query_epoch) && cone_b.is_inside(query_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    fn long_lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(10_000))
    }

    fn short_lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn both_inside_valid() {
        let a = EnergyCone::new(long_lambda(), 100, 1000, 0);
        let b = EnergyCone::new(long_lambda(), 100, 1000, 0);
        assert!(bridge_valid(&a, &b, 50));
    }

    #[test]
    fn one_outside_invalid() {
        let a = EnergyCone::new(long_lambda(), 100, 1000, 0);
        // chain B has shorter λ — its cone collapses faster.
        let b = EnergyCone::new(short_lambda(), 600, 1000, 0);
        // At epoch 200, b's remaining ≈ 250 < 600 → outside.
        assert!(!bridge_valid(&a, &b, 200));
    }

    #[test]
    fn intersection_window_is_finite() {
        // Both chains have short λ but different observed_epochs.
        let a = EnergyCone::new(short_lambda(), 200, 1000, 0);
        let b = EnergyCone::new(short_lambda(), 200, 1000, 50);
        // a alive at 100? remaining = 500 ≥ 200 ✓
        // b alive at 100? elapsed = 50, remaining ≈ 750 ≥ 200 ✓
        assert!(bridge_valid(&a, &b, 100));
        // At 500: a remaining ≈ 1000 >> 5 = 31 < 200 ✗
        assert!(!bridge_valid(&a, &b, 500));
    }
}
