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
    /// SUB-N3 (audit 2026-05-15): the recipient's balance would
    /// overflow u64 on credit. Layer-0 conservation requires the
    /// credit to land in full; if u64 can't hold it, the transfer
    /// is rejected before any mutation.
    #[error("recipient {recipient:?} balance overflow: {existing} + {amount} > u64::MAX")]
    RecipientOverflow {
        recipient: AccountAddress,
        existing: u64,
        amount: u64,
    },
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
    ///
    /// SUB-N3 (audit 2026-05-15): pre-validate the credit side
    /// (recipient's new balance) BEFORE any debit. The previous
    /// implementation debited `from` unconditionally then credited
    /// `to` with `saturating_add` — if `to`'s balance was near
    /// `u64::MAX`, the credit silently clipped while the full debit
    /// landed. Net effect: total MWh on the books decreased,
    /// breaking the Layer-0 conservation invariant. The fix uses
    /// `checked_add` on the credit, fails with `RecipientOverflow`
    /// before mutating either side, so the transfer is atomic at
    /// the conservation level.
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
        // SUB-N3: read recipient's existing balance + verify the
        // credit fits in u64 BEFORE touching `from`.
        let to_key = (location.clone(), slot, to);
        let to_existing = *self.entries.get(&to_key).unwrap_or(&0);
        let to_new = to_existing
            .checked_add(amount)
            .ok_or(BookError::RecipientOverflow {
                recipient: to,
                existing: to_existing,
                amount,
            })?;

        // Now safe to mutate both sides.
        if avail == amount {
            self.entries.remove(&from_key);
        } else {
            *self.entries.get_mut(&from_key).unwrap() -= amount;
        }
        self.entries.insert(to_key, to_new);
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

    /// SUB-N3 (audit 2026-05-15) regression: transfer must reject
    /// the operation when crediting the recipient would overflow u64.
    /// Pre-fix the debit landed in full while the credit clipped via
    /// `saturating_add`, silently deleting MWh from the chain-wide
    /// supply — a Layer-0 conservation break.
    #[test]
    fn sub_n3_transfer_rejects_recipient_overflow_atomic() {
        let mut b = HbctBook::new();
        let loc: DeliveryLocation = b"BMU-1".to_vec();
        // Sender holds 100 MWh.
        b.mint(token(1, 100, 100)).unwrap();
        // Pre-seed recipient at u64::MAX - 50 (close to overflow).
        let to_key = (loc.clone(), 100u64, addr(2));
        b.entries.insert(to_key, u64::MAX - 50);

        let total_before: u128 = b.entries.values().map(|v| *v as u128).sum();

        // Transfer 100 → recipient_new = u64::MAX-50 + 100 = overflow.
        let err = b
            .transfer(&loc, 100, addr(1), addr(2), 100)
            .expect_err("must reject");
        assert!(
            matches!(err, BookError::RecipientOverflow { .. }),
            "SUB-N3: expected RecipientOverflow, got {err:?}"
        );

        // Atomicity: NEITHER side has been mutated.
        let total_after: u128 = b.entries.values().map(|v| *v as u128).sum();
        assert_eq!(
            total_before, total_after,
            "SUB-N3: failed transfer must NOT mutate any balance \
             (conservation invariant)"
        );
        assert_eq!(
            b.balance(&loc, 100, addr(1)),
            100,
            "SUB-N3: sender balance must be unchanged after rejection"
        );
        assert_eq!(
            b.balance(&loc, 100, addr(2)),
            u64::MAX - 50,
            "SUB-N3: recipient balance must be unchanged after rejection"
        );
    }

    /// SUB-N3: normal transfer (no overflow) still conserves total
    /// MWh exactly — sender debit equals recipient credit.
    #[test]
    fn sub_n3_normal_transfer_conserves_total() {
        let mut b = HbctBook::new();
        let loc: DeliveryLocation = b"BMU-1".to_vec();
        b.mint(token(1, 100, 30)).unwrap();
        b.mint(token(2, 100, 70)).unwrap(); // recipient pre-existing balance
        let total_before: u128 = b.entries.values().map(|v| *v as u128).sum();

        b.transfer(&loc, 100, addr(1), addr(2), 10).unwrap();

        let total_after: u128 = b.entries.values().map(|v| *v as u128).sum();
        assert_eq!(total_before, total_after, "SUB-N3: total MWh must conserve");
        assert_eq!(b.balance(&loc, 100, addr(1)), 20);
        assert_eq!(b.balance(&loc, 100, addr(2)), 80);
    }
}
