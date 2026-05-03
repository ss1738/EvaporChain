//! `β = 1/λ` — derive the CFM inverse temperature from the chain's
//! single λ.
//!
//! λ in the kernel is a half-life in epochs. The Crooks/Jarzynski
//! formalism wants an inverse-temperature `β` with units of
//! `1/(fee unit · epoch)`. We expose `β` in **microbits per fee unit
//! per epoch** — fine enough to give a non-zero β for the chain's
//! genesis `DEFAULT_LAMBDA = 4096` (where the original millibits scale
//! integer-floored to zero), while still fitting in `u64` headroom.
//!
//! The historical Rust + JSON field name `beta_mb` is kept for API
//! stability — it predates the unit fix that renamed the *scale* from
//! milli (×10³) to micro (×10⁶). Treat `_mb` as an opaque tag, not
//! literally "millibits". Per the doctrine punch list (`Layer 0
//! item 5`): fixing the resolution is non-negotiable; renaming the
//! tag across consensus / mcc / node / mcp is deferred.

use thiserror::Error;

use evaporchain_energy_kernel::ChainLambda;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BetaError {
    #[error("chain λ is degenerate (half_life = 0); β is undefined")]
    DegenerateLambda,
}

/// `β = 1/λ` in **microbits per fee unit per epoch**.
///
/// Concretely: `β_mb = 1_000_000 / half_life`. For λ = 4096
/// (`DEFAULT_LAMBDA`) this gives `β_mb = 244` — small but non-zero,
/// so a fee-market histogram with spread above ~4_000 fee units sees
/// a meaningful reweighting (vs the prior millibits scale where
/// every fee mapped to `MAX_WEIGHT`). For test λ = 10 we get
/// `β_mb = 100_000` — aggressive, drives multi-bit shifts even at
/// fees of 10.
///
/// Function name is kept (`_millibits_per_fee`) to avoid a 30-touch
/// rename across consensus / mcc / node / mcp / annealing-integration.
/// The historical `_mb` suffix is now an opaque tag.
pub fn beta_millibits_per_fee(chain_lambda: ChainLambda) -> Result<u64, BetaError> {
    let half_life = chain_lambda.half_life();
    if half_life == 0 {
        return Err(BetaError::DegenerateLambda);
    }
    Ok(1_000_000 / half_life)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_energy_kernel::Lambda;

    #[test]
    fn beta_inverse_of_half_life() {
        let cl = ChainLambda::new(Lambda::from_epochs(10));
        // 1_000_000 / 10 = 100_000 (microbits per fee per epoch).
        assert_eq!(beta_millibits_per_fee(cl).unwrap(), 100_000);
    }

    #[test]
    fn beta_nonzero_at_default_lambda() {
        let cl = ChainLambda::new(Lambda::from_epochs(4096));
        // 1_000_000 / 4096 = 244 (was 0 under the millibits scale —
        // that was the degenerate case the doctrine punch list
        // flagged). Layer 0 item 5: β must remain non-zero at
        // genesis λ so the fee-market caliber score is meaningful.
        assert_eq!(beta_millibits_per_fee(cl).unwrap(), 244);
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
