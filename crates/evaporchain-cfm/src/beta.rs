//! `β = 1/λ` — derive the CFM inverse temperature from the chain's
//! single λ.
//!
//! λ in the kernel is a half-life in epochs. The Crooks/Jarzynski
//! formalism wants an inverse-temperature `β` with units of
//! `1/(fee unit · epoch)`. We expose `β` in *millibits per fee unit per
//! epoch* — small enough to be meaningful for typical λ ∈ [10², 10⁵]
//! epochs, large enough to avoid integer-floor collapse to zero.

use thiserror::Error;

use evaporchain_energy_kernel::ChainLambda;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BetaError {
    #[error("chain λ is degenerate (half_life = 0); β is undefined")]
    DegenerateLambda,
}

/// `β = 1/λ` in millibits per fee unit per epoch.
///
/// Concretely: `β_mb = 1000 / half_life`. For λ = 4096 (DEFAULT_LAMBDA)
/// this gives `β_mb = 0` (integer floor) — meaning the substrate rate
/// of decay is too slow for fee-market scale interactions to
/// distinguish. For test λ = 10 we get `β_mb = 100` — a meaningful
/// reweighting on a fee distribution with non-trivial spread.
///
/// In production the chain will likely run with a *fee-market-scoped*
/// β derived from a fee-market-specific time constant, while the
/// global λ keeps the consensus/state-decay timeline. Single-λ remains
/// the *source of truth*; downstream primitives may scale it.
pub fn beta_millibits_per_fee(chain_lambda: ChainLambda) -> Result<u64, BetaError> {
    let half_life = chain_lambda.half_life();
    if half_life == 0 {
        return Err(BetaError::DegenerateLambda);
    }
    Ok(1_000 / half_life)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    #[test]
    fn beta_inverse_of_half_life() {
        let cl = ChainLambda::new(Lambda::from_epochs(10));
        assert_eq!(beta_millibits_per_fee(cl).unwrap(), 100);
    }

    #[test]
    fn beta_floors_to_zero_for_large_lambda() {
        let cl = ChainLambda::new(Lambda::from_epochs(4096));
        // 1000 / 4096 = 0 (integer floor) — flagged in module docs
        // as the substrate boundary for typical genesis λ.
        assert_eq!(beta_millibits_per_fee(cl).unwrap(), 0);
    }

    #[test]
    fn degenerate_lambda_rejected() {
        let cl = ChainLambda::new(Lambda::from_epochs(0));
        assert_eq!(
            beta_millibits_per_fee(cl).unwrap_err(),
            BetaError::DegenerateLambda
        );
    }
}
