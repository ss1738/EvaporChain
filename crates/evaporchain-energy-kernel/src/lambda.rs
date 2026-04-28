//! The single chain constant λ.
//!
//! Per INVENTION_STACK.md §1.1: every layer of the chain — consensus,
//! mempool, time, gas, stake, governance, capabilities, identity,
//! demurrage — is parameterized by *one* number. Naming variations like
//! "decay rate", "energy half-life", "epoch evaporation rate", "refresh
//! interval" all collapse to this same `Lambda`.
//!
//! The unit is "epochs per halving" (a `HalfLife` in `evaporchain-types`).
//! A primitive that wants to express decay over a different physical unit
//! (e.g. blocks-per-halving, seconds-per-halving) must convert at its
//! boundary, not introduce a second constant.

use evaporchain_types::HalfLife;
use serde::{Deserialize, Serialize};

/// Newtype around `HalfLife` so the *single chain constant* has a
/// type-distinct identity from arbitrary per-call half-lives an old
/// API might still take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Lambda(pub HalfLife);

impl Lambda {
    /// Construct the chain λ from a raw half-life in epochs.
    pub const fn from_epochs(epochs: HalfLife) -> Self {
        Self(epochs)
    }

    /// Raw half-life in epochs — the unit `energy_at_epoch` consumes.
    pub const fn epochs(self) -> HalfLife {
        self.0
    }

    /// True iff λ is set to a value that disables decay (half_life = 0
    /// causes `energy_at_epoch` to return 0 immediately, which is the
    /// degenerate case used in tests, not in production).
    pub const fn is_degenerate(self) -> bool {
        self.0 == 0
    }
}

/// Default λ — placeholder until tokenomics ceremony per INVENTION_STACK.md
/// §8 open question "Total chain energy budget at genesis sets all other
/// constants". 4096 epochs (~roughly a long but finite tenancy window in
/// any per-second epoch schedule) is conservative and matches the
/// half-life shape used in the existing `evaporchain-state::decay_curves`
/// integer arithmetic.
pub const DEFAULT_LAMBDA: Lambda = Lambda::from_epochs(4096);

/// The chain-global λ source. Layers read λ via this struct rather than
/// hard-coding `DEFAULT_LAMBDA`, so a future governance amendment can
/// rotate it without each layer carrying its own constant.
///
/// Construction is intentionally explicit: a node binary builds one
/// `ChainLambda` at startup, then hands it down to every consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainLambda {
    lambda: Lambda,
}

impl ChainLambda {
    /// Construct from a chosen λ.
    pub const fn new(lambda: Lambda) -> Self {
        Self { lambda }
    }

    /// Convenience: the default-tokenomics build.
    pub const fn default_genesis() -> Self {
        Self::new(DEFAULT_LAMBDA)
    }

    /// Read the chain λ.
    pub const fn lambda(self) -> Lambda {
        self.lambda
    }

    /// Read the half-life in epochs (the unit `energy_at_epoch` accepts).
    pub const fn half_life(self) -> HalfLife {
        self.lambda.epochs()
    }
}

impl Default for ChainLambda {
    fn default() -> Self {
        Self::default_genesis()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_lambda_non_degenerate() {
        assert!(!DEFAULT_LAMBDA.is_degenerate());
        assert_eq!(DEFAULT_LAMBDA.epochs(), 4096);
    }

    #[test]
    fn chain_lambda_round_trip() {
        let cl = ChainLambda::new(Lambda::from_epochs(8192));
        assert_eq!(cl.half_life(), 8192);
        assert_eq!(cl.lambda().epochs(), 8192);
    }

    #[test]
    fn default_chain_lambda_matches_default_constant() {
        assert_eq!(ChainLambda::default(), ChainLambda::new(DEFAULT_LAMBDA));
    }
}
