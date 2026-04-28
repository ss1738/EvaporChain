//! Per-share survival check.

use evaporchain_energy_kernel::{energy_at_epoch, ChainLambda};
use evaporchain_types::Energy;

use crate::share::Share;

/// True iff `share`'s λ-decayed remaining energy at `query_epoch`
/// is strictly above `threshold`.
pub fn is_alive(share: &Share, chain_lambda: ChainLambda, query_epoch: u64, threshold: Energy) -> bool {
    if query_epoch < share.observed_epoch {
        return true; // pre-observation = trivially alive (full seed).
    }
    let elapsed = query_epoch - share.observed_epoch;
    let remaining = energy_at_epoch(share.energy, chain_lambda.half_life(), elapsed);
    remaining > threshold
}

/// Count of shares alive at `query_epoch`.
pub fn count_alive(
    shares: &[Share],
    chain_lambda: ChainLambda,
    query_epoch: u64,
    threshold: Energy,
) -> usize {
    shares
        .iter()
        .filter(|s| is_alive(s, chain_lambda, query_epoch, threshold))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn fresh_share_alive() {
        let s = Share::new(1, 1000, 0);
        assert!(is_alive(&s, lambda(), 0, 1));
    }

    #[test]
    fn share_decays_below_threshold() {
        let s = Share::new(1, 1000, 0);
        assert!(is_alive(&s, lambda(), 100, 100));   // remaining=500 > 100 ✓
        assert!(!is_alive(&s, lambda(), 100, 500));  // remaining=500 = threshold → not strictly above
    }

    #[test]
    fn count_alive_counts_only_living() {
        let shares = [
            Share::new(1, 1000, 0),
            Share::new(2, 100, 0), // smaller seed — dies sooner
            Share::new(3, 1000, 0),
        ];
        // At t=100, threshold=200:
        //   share 1: remaining=500 > 200 ✓ alive
        //   share 2: remaining=50 < 200 ✗ dead
        //   share 3: remaining=500 > 200 ✓ alive
        assert_eq!(count_alive(&shares, lambda(), 100, 200), 2);
    }
}
