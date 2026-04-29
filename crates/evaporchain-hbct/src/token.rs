//! `HbctToken` — single capacity claim for one (location, hour) slot.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use evaporchain_types::AccountAddress;

/// Opaque grid-zone identifier (e.g. GB BMRS BMU id, ENTSO-E bidding
/// zone). Substrate keeps it as `Vec<u8>` so the kernel doesn't bind
/// to any single grid's identifier scheme.
pub type DeliveryLocation = Vec<u8>;

/// Delivery hour, expressed as the chain's epoch index of the slot's
/// *close*. A token with `hour_slot = 100` is valid until just before
/// epoch 100; at epoch 100 it auto-burns per the doctrine
/// "capacity in hour H decays to 0 at H+1".
pub type HourSlot = u64;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HbctToken {
    pub delivery_location: DeliveryLocation,
    pub hour_slot: HourSlot,
    /// Megawatt-hours committed for this slot (integer; chain stores
    /// in `u64` because no real grid trades sub-Wh fractions).
    pub mwh_amount: u64,
    pub holder: AccountAddress,
    pub issued_at_epoch: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("delivery_location is empty")]
    EmptyLocation,
    #[error("mwh_amount is zero")]
    ZeroMwh,
    #[error("hour_slot {hour_slot} is at-or-before issued_at_epoch {issued_at_epoch}")]
    SlotInPast {
        hour_slot: u64,
        issued_at_epoch: u64,
    },
}

impl HbctToken {
    pub fn new(
        delivery_location: DeliveryLocation,
        hour_slot: HourSlot,
        mwh_amount: u64,
        holder: AccountAddress,
        issued_at_epoch: u64,
    ) -> Result<Self, TokenError> {
        if delivery_location.is_empty() {
            return Err(TokenError::EmptyLocation);
        }
        if mwh_amount == 0 {
            return Err(TokenError::ZeroMwh);
        }
        if hour_slot <= issued_at_epoch {
            return Err(TokenError::SlotInPast { hour_slot, issued_at_epoch });
        }
        Ok(Self {
            delivery_location,
            hour_slot,
            mwh_amount,
            holder,
            issued_at_epoch,
        })
    }

    /// True iff this token's hour_slot has closed at `current_epoch`.
    /// Strict: the slot is *valid* up to `hour_slot - 1`, *closed*
    /// at `hour_slot`.
    pub fn is_closed(&self, current_epoch: u64) -> bool {
        current_epoch >= self.hour_slot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(b: u8) -> AccountAddress {
        [b; 32]
    }

    #[test]
    fn ok_construction() {
        let t = HbctToken::new(b"BMU-1".to_vec(), 100, 50, addr(1), 0).unwrap();
        assert_eq!(t.mwh_amount, 50);
    }

    #[test]
    fn empty_location_rejected() {
        let err = HbctToken::new(vec![], 100, 50, addr(1), 0).unwrap_err();
        assert_eq!(err, TokenError::EmptyLocation);
    }

    #[test]
    fn zero_mwh_rejected() {
        let err = HbctToken::new(b"BMU-1".to_vec(), 100, 0, addr(1), 0).unwrap_err();
        assert_eq!(err, TokenError::ZeroMwh);
    }

    #[test]
    fn slot_in_past_rejected() {
        let err = HbctToken::new(b"BMU-1".to_vec(), 50, 10, addr(1), 100).unwrap_err();
        assert!(matches!(err, TokenError::SlotInPast { .. }));
    }

    #[test]
    fn slot_at_issued_epoch_rejected() {
        // hour_slot == issued_at_epoch is also rejected — token must
        // be for a strictly future slot.
        let err = HbctToken::new(b"BMU-1".to_vec(), 100, 10, addr(1), 100).unwrap_err();
        assert!(matches!(err, TokenError::SlotInPast { .. }));
    }

    #[test]
    fn is_closed_at_or_after_slot() {
        let t = HbctToken::new(b"BMU-1".to_vec(), 100, 10, addr(1), 0).unwrap();
        assert!(!t.is_closed(99));
        assert!(t.is_closed(100));
        assert!(t.is_closed(101));
    }
}
