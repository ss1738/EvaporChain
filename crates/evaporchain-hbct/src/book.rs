//! `HbctBook` — chain-wide ledger of HBCT tokens.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

use evaporchain_types::AccountAddress;

use crate::token::{DeliveryLocation, HbctToken, HourSlot, TokenError};

/// Composite key for the ledger: (location, slot, holder). Lets a
/// single (location, slot) carry separate tokens for distinct
/// holders without collision.
type Key = (DeliveryLocation, HourSlot, AccountAddress);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HbctBook {
    pub entries: BTreeMap<Key, u64>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BookError {
    #[error("token validation: {0}")]
    Token(#[from] TokenError),
    #[error("holder {0:?} has no entry at this (location, slot)")]
    NoEntry(AccountAddress),
    #[error("holder {holder:?} has only {available} mwh; cannot transfer/burn {amount}")]
    Insufficient {
        holder: AccountAddress,
        available: u64,
        amount: u64,
    },
    /// SUB-N3 (audit 2026-05-15): transfer would overflow recipient's balance.
    /// Rejected *before* debiting `from` so Layer-0 conservation is preserved.
    #[error("transfer would overflow recipient balance")]
    RecipientOverflow,
}

impl HbctBook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mint a fresh HBCT (validates the token shape, then credits).
    pub fn mint(&mut self, token: HbctToken) -> Result<(), BookError> {
        let key = (token.delivery_location, token.hour_slot, token.holder);
        let slot = self.entries.entry(key).or_insert(0);
        *slot = slot.saturating_add(token.mwh_amount);
        Ok(())
    }

    /// Transfer `amount` MWh of capacity at `(location, slot)` from
    /// `from` to `to`.
    pub fn transfer(
        &mut self,
        location: &DeliveryLocation,
        slot: HourSlot,
        from: AccountAddress,
        to: AccountAddress,
        amount: u64,
    ) -> Result<(), BookError> {
        let from_key = (location.clone(), slot, from);
        let avail = *self
            .entries
            .get(&from_key)
            .ok_or(BookError::NoEntry(from))?;
        if avail < amount {
            return Err(BookError::Insufficient {
                holder: from,
                available: avail,
                amount,
            });
        }
        // SUB-N3 (audit 2026-05-15): verify the recipient won't overflow
        // BEFORE debiting `from`. `saturating_add` on the credit side
        // silently clips while the debit still lands → total MWh decreases,
        // breaking Layer-0 conservation.
        let to_key = (location.clone(), slot, to);
        let to_current = self.entries.get(&to_key).copied().unwrap_or(0);
        to_current
            .checked_add(amount)
            .ok_or(BookError::RecipientOverflow)?;

        // Both checks passed — mutate safely.
        if avail == amount {
            self.entries.remove(&from_key);
        } else {
            *self.entries.get_mut(&from_key).unwrap() -= amount;
        }
        let to_slot = self.entries.entry(to_key).or_insert(0);
        *to_slot = to_current + amount; // overflow confirmed impossible above
        Ok(())
    }

    /// Burn `amount` MWh from `holder`'s position at (location, slot).
    pub fn burn(
        &mut self,
        location: &DeliveryLocation,
        slot: HourSlot,
        holder: AccountAddress,
        amount: u64,
    ) -> Result<(), BookError> {
        let key = (location.clone(), slot, holder);
        let avail = *self.entries.get(&key).ok_or(BookError::NoEntry(holder))?;
        if avail < amount {
            return Err(BookError::Insufficient {
                holder,
                available: avail,
                amount,
            });
        }
        if avail == amount {
            self.entries.remove(&key);
        } else {
            *self.entries.get_mut(&key).unwrap() -= amount;
        }
        Ok(())
    }

    /// Query: how many MWh does `holder` have at (location, slot)?
    pub fn balance(
        &self,
        location: &DeliveryLocation,
        slot: HourSlot,
        holder: AccountAddress,
    ) -> u64 {
        *self
            .entries
            .get(&(location.clone(), slot, holder))
            .unwrap_or(&0)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(b: u8) -> AccountAddress {
        [b; 32]
    }

    fn token(holder: u8, slot: u64, mwh: u64) -> HbctToken {
        HbctToken::new(b"BMU-1".to_vec(), slot, mwh, addr(holder), 0).unwrap()
    }

    #[test]
    fn mint_credits_holder() {
        let mut b = HbctBook::new();
        b.mint(token(1, 100, 50)).unwrap();
        assert_eq!(b.balance(&b"BMU-1".to_vec(), 100, addr(1)), 50);
    }

    #[test]
    fn transfer_moves_capacity() {
        let mut b = HbctBook::new();
        b.mint(token(1, 100, 50)).unwrap();
        b.transfer(&b"BMU-1".to_vec(), 100, addr(1), addr(2), 30)
            .unwrap();
        assert_eq!(b.balance(&b"BMU-1".to_vec(), 100, addr(1)), 20);
        assert_eq!(b.balance(&b"BMU-1".to_vec(), 100, addr(2)), 30);
    }

    #[test]
    fn full_transfer_removes_entry() {
        let mut b = HbctBook::new();
        b.mint(token(1, 100, 50)).unwrap();
        b.transfer(&b"BMU-1".to_vec(), 100, addr(1), addr(2), 50)
            .unwrap();
        assert_eq!(b.balance(&b"BMU-1".to_vec(), 100, addr(1)), 0);
    }

    #[test]
    fn burn_reduces_balance() {
        let mut b = HbctBook::new();
        b.mint(token(1, 100, 50)).unwrap();
        b.burn(&b"BMU-1".to_vec(), 100, addr(1), 20).unwrap();
        assert_eq!(b.balance(&b"BMU-1".to_vec(), 100, addr(1)), 30);
    }

    #[test]
    fn burn_full_removes_entry() {
        let mut b = HbctBook::new();
        b.mint(token(1, 100, 50)).unwrap();
        b.burn(&b"BMU-1".to_vec(), 100, addr(1), 50).unwrap();
        assert_eq!(b.len(), 0);
    }

    #[test]
    fn insufficient_transfer_rejected() {
        let mut b = HbctBook::new();
        b.mint(token(1, 100, 50)).unwrap();
        let err = b
            .transfer(&b"BMU-1".to_vec(), 100, addr(1), addr(2), 100)
            .unwrap_err();
        assert!(matches!(err, BookError::Insufficient { .. }));
    }

    #[test]
    fn no_entry_rejected() {
        let mut b = HbctBook::new();
        let err = b.burn(&b"BMU-1".to_vec(), 100, addr(99), 1).unwrap_err();
        assert!(matches!(err, BookError::NoEntry(_)));
    }

    #[test]
    fn double_mint_aggregates() {
        // Two mints to the same (location, slot, holder) triple stack.
        let mut b = HbctBook::new();
        b.mint(token(1, 100, 30)).unwrap();
        b.mint(token(1, 100, 20)).unwrap();
        assert_eq!(b.balance(&b"BMU-1".to_vec(), 100, addr(1)), 50);
    }

    #[test]
    fn sub_n3_transfer_to_near_max_rejected_without_state_change() {
        // SUB-N3 (audit 2026-05-15): if recipient's balance is near u64::MAX,
        // `saturating_add` would clip the credit while the debit still lands,
        // silently destroying MWh (conservation break). The fix uses
        // `checked_add` and rejects BEFORE touching `from`.
        let mut b = HbctBook::new();
        let loc = b"BMU-1".to_vec();
        let from = addr(1);
        let to = addr(2);

        // Mint sender with 10 MWh.
        b.mint(token(1, 100, 10)).unwrap();

        // Force recipient to u64::MAX - 5 via direct map insertion
        // (no public API reaches this value, which is the whole point).
        let to_key = (loc.clone(), 100u64, to);
        b.entries.insert(to_key, u64::MAX - 5);

        let from_before = b.balance(&loc, 100, from);
        let to_before = b.balance(&loc, 100, to);

        // Transfer 10 would overflow recipient (u64::MAX - 5 + 10 wraps).
        let err = b.transfer(&loc, 100, from, to, 10).unwrap_err();
        assert_eq!(err, BookError::RecipientOverflow);

        // Neither balance must have changed — no conservation loss.
        assert_eq!(
            b.balance(&loc, 100, from),
            from_before,
            "sender must not be debited on overflow-rejected transfer"
        );
        assert_eq!(
            b.balance(&loc, 100, to),
            to_before,
            "recipient must not be credited on overflow-rejected transfer"
        );
    }
}
