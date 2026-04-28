//! `quorum_alive` — true iff at least `k` of `n` shares are alive at
//! `query_epoch`. The chain-side gate that decides whether secret
//! reconstruction may even be attempted.

use evaporchain_energy_kernel::ChainLambda;
use evaporchain_types::Energy;

use crate::share::Share;
use crate::survival::count_alive;

pub fn quorum_alive(
    shares: &[Share],
    k: usize,
    chain_lambda: ChainLambda,
    query_epoch: u64,
    threshold: Energy,
) -> bool {
    count_alive(shares, chain_lambda, query_epoch, threshold) >= k
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    fn lambda() -> ChainLambda {
        ChainLambda::new(Lambda::from_epochs(100))
    }

    #[test]
    fn quorum_met_at_zero_decay() {
        let shares = [
            Share::new(1, 1000, 0),
            Share::new(2, 1000, 0),
            Share::new(3, 1000, 0),
        ];
        assert!(quorum_alive(&shares, 2, lambda(), 0, 1));
    }

    #[test]
    fn quorum_lost_after_enough_decay() {
        let shares = [
            Share::new(1, 1000, 0),
            Share::new(2, 1000, 0),
            Share::new(3, 1000, 0),
        ];
        // Need k=2 alive at threshold=900. After enough decay, all 3
        // drop below 900 → quorum lost.
        assert!(!quorum_alive(&shares, 2, lambda(), 100, 900));
        // But 2-of-3 with low threshold still passes:
        assert!(quorum_alive(&shares, 2, lambda(), 100, 100));
    }

    #[test]
    fn k_zero_always_true() {
        let shares: [Share; 0] = [];
        assert!(quorum_alive(&shares, 0, lambda(), 100, 1));
    }

    #[test]
    fn k_greater_than_n_never_met() {
        let shares = [Share::new(1, 1000, 0)];
        assert!(!quorum_alive(&shares, 5, lambda(), 0, 1));
    }
}
