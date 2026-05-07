//! `AttentionQuantum` — the unit primitive.
//!
//! Each AQ carries an `initial_value`, a `half_life` (typically the
//! cognitive-forgetting-curve λ ≈ 45 minutes' worth of epochs), and
//! the `minted_at_epoch`. `value_at(epoch_now)` is the standard
//! `energy_at_epoch` rule: full at mint, halves every `half_life`,
//! linear-interpolated between half-lives.
//!
//! AQs are tied to a human wallet; transfers are deliberately **not
//! supported** in V1 (attention is not fungible across humans). Once
//! minted, an AQ either earns from the pool or decays unspent.

use evaporchain_types::{energy_at_epoch, AccountAddress, Energy, Epoch, HalfLife};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type AqId = [u8; 32];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AqMintError {
    #[error("initial_value must be > 0")]
    ZeroInitial,
    #[error("half_life must be > 0")]
    ZeroHalfLife,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionQuantum {
    pub id: AqId,
    /// The verified-human wallet that emitted this quantum.
    pub holder: AccountAddress,
    pub initial_value: Energy,
    pub half_life: HalfLife,
    pub minted_at_epoch: Epoch,
}

impl AttentionQuantum {
    pub fn mint(
        id: AqId,
        holder: AccountAddress,
        initial_value: Energy,
        half_life: HalfLife,
        minted_at_epoch: Epoch,
    ) -> Result<Self, AqMintError> {
        if initial_value == 0 {
            return Err(AqMintError::ZeroInitial);
        }
        if half_life == 0 {
            return Err(AqMintError::ZeroHalfLife);
        }
        Ok(Self {
            id,
            holder,
            initial_value,
            half_life,
            minted_at_epoch,
        })
    }

    /// Decayed value at `epoch_now`.
    pub fn value_at(&self, epoch_now: Epoch) -> Energy {
        let elapsed = epoch_now.saturating_sub(self.minted_at_epoch);
        energy_at_epoch(self.initial_value, self.half_life, elapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> AqId {
        let mut x = [0u8; 32];
        x[0] = b;
        x
    }

    fn addr(b: u8) -> AccountAddress {
        id(b)
    }

    #[test]
    fn mint_rejects_zero_initial() {
        let err = AttentionQuantum::mint(id(1), addr(0xAA), 0, 100, 0).unwrap_err();
        assert_eq!(err, AqMintError::ZeroInitial);
    }

    #[test]
    fn mint_rejects_zero_half_life() {
        let err = AttentionQuantum::mint(id(1), addr(0xAA), 100, 0, 0).unwrap_err();
        assert_eq!(err, AqMintError::ZeroHalfLife);
    }

    #[test]
    fn fresh_aq_at_mint_epoch_is_full_value() {
        let aq = AttentionQuantum::mint(id(1), addr(0xAA), 1000, 45, 100).unwrap();
        assert_eq!(aq.value_at(100), 1000);
    }

    #[test]
    fn five_minute_aq_worth_more_than_25_minute() {
        // Doctrine claim: "A 5-minute-old AQ is worth more than a
        // 25-min one because the human is statistically still
        // cognitively-engaged."
        let aq = AttentionQuantum::mint(id(1), addr(0xAA), 1000, 45, 0).unwrap();
        let v_5min = aq.value_at(5);
        let v_25min = aq.value_at(25);
        assert!(
            v_5min > v_25min,
            "5min={v_5min} should be > 25min={v_25min}"
        );
    }

    #[test]
    fn aq_decays_to_zero_eventually() {
        let aq = AttentionQuantum::mint(id(1), addr(0xAA), 4, 1, 0).unwrap();
        // After many half-lives, value reaches zero.
        assert_eq!(aq.value_at(10_000), 0);
    }

    #[test]
    fn round_trip_serde() {
        let aq = AttentionQuantum::mint(id(7), addr(0xAB), 5000, 45, 42).unwrap();
        let s = serde_json::to_string(&aq).unwrap();
        let back: AttentionQuantum = serde_json::from_str(&s).unwrap();
        assert_eq!(aq, back);
    }
}
