//! `CauseOfDeath` — why the account evaporated.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CauseOfDeath {
    /// Balance decayed to zero under the chain-global λ; no resurrection.
    Evaporated,
    /// Decay-Forget proof made the recoverability commitment fall
    /// below the chain-set threshold (per `evaporchain-decay-forget`).
    ForgottenViaDecayProof,
    /// Stake slashed to zero (Sanov or Entropic).
    SlashedToZero,
    /// Storage rent exhausted account balance.
    RentExhausted,
    /// Reserved for future causes; the `u32` lets governance add new
    /// reasons without breaking on-chain serialization.
    Other(u32),
}

impl CauseOfDeath {
    /// Stable wire byte for the cause. Domain-separated so adding a
    /// new variant later doesn't collide with existing tombstones'
    /// commitments.
    pub fn discriminant(self) -> u32 {
        match self {
            Self::Evaporated => 1,
            Self::ForgottenViaDecayProof => 2,
            Self::SlashedToZero => 3,
            Self::RentExhausted => 4,
            Self::Other(n) => 1_000 + n,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminants_are_distinct() {
        assert_eq!(CauseOfDeath::Evaporated.discriminant(), 1);
        assert_eq!(CauseOfDeath::ForgottenViaDecayProof.discriminant(), 2);
        assert_eq!(CauseOfDeath::SlashedToZero.discriminant(), 3);
        assert_eq!(CauseOfDeath::RentExhausted.discriminant(), 4);
        assert_eq!(CauseOfDeath::Other(0).discriminant(), 1000);
        assert_eq!(CauseOfDeath::Other(99).discriminant(), 1099);
    }
}
