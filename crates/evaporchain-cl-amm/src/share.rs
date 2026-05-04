//! `LpShare` — one holder's LP record + energy tag.

use serde::{Deserialize, Serialize};

/// 32-byte chain-wide handle for a liquidity-provider account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct HolderId(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LpShare {
    pub holder: HolderId,
    /// LP-share count.
    pub shares: u128,
    /// Energy tag. Must be > pool's `energy_floor` to withdraw.
    /// Decays externally via `Pool::decay_holder` calls from
    /// the chain's tick.
    pub energy: u64,
    /// Epoch the share was last anchored / re-anchored.
    pub anchored_at_epoch: u64,
}

impl LpShare {
    pub fn new(holder: HolderId, shares: u128, energy: u64, anchored_at_epoch: u64) -> Self {
        Self {
            holder,
            shares,
            energy,
            anchored_at_epoch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_serde() {
        let s = LpShare::new(HolderId([0xAA; 32]), 1_000_000, 1000, 100);
        let j = serde_json::to_string(&s).unwrap();
        let back: LpShare = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
