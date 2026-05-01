//! `auto_burn_at_slot_close` — sweep the book for closed slots.
//!
//! Per the doctrine: "capacity in hour H decays to 0 at H+1." This
//! is the per-block hook that enforces it: every entry whose
//! `hour_slot ≤ current_epoch` is removed.

use serde::{Deserialize, Serialize};

use evaporchain_types::AccountAddress;

use crate::book::HbctBook;
use crate::token::{DeliveryLocation, HourSlot};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BurnOutcome {
    /// Number of distinct ledger entries removed.
    pub entries_removed: usize,
    /// Total MWh of capacity that decayed to zero this tick.
    pub mwh_burnt: u64,
    /// Per-(location, slot, holder) detail for any chain auditor /
    /// indexer that wants the breakdown.
    pub burnt: Vec<(DeliveryLocation, HourSlot, AccountAddress, u64)>,
}

/// Walk `book.entries` and remove every entry whose `hour_slot` has
/// closed at `current_epoch`. Returns the burn outcome for chain-side
/// audit.
pub fn auto_burn_at_slot_close(book: &mut HbctBook, current_epoch: u64) -> BurnOutcome {
    let mut to_remove: Vec<(DeliveryLocation, HourSlot, AccountAddress)> = Vec::new();
    let mut burnt: Vec<(DeliveryLocation, HourSlot, AccountAddress, u64)> = Vec::new();
    let mut total_mwh: u64 = 0;
    for ((loc, slot, holder), &mwh) in &book.entries {
        if *slot <= current_epoch {
            to_remove.push((loc.clone(), *slot, *holder));
            burnt.push((loc.clone(), *slot, *holder, mwh));
            total_mwh = total_mwh.saturating_add(mwh);
        }
    }
    for k in &to_remove {
        book.entries.remove(k);
    }
    BurnOutcome {
        entries_removed: to_remove.len(),
        mwh_burnt: total_mwh,
        burnt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::HbctToken;
    use evaporchain_types::AccountAddress;

    fn addr(b: u8) -> AccountAddress {
        [b; 32]
    }

    #[test]
    fn no_closed_slots_no_burn() {
        let mut b = HbctBook::new();
        b.mint(HbctToken::new(b"BMU-1".to_vec(), 100, 50, addr(1), 0).unwrap())
            .unwrap();
        let out = auto_burn_at_slot_close(&mut b, 50);
        assert_eq!(out.entries_removed, 0);
        assert_eq!(out.mwh_burnt, 0);
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn closed_slot_burnt_at_close_epoch() {
        let mut b = HbctBook::new();
        b.mint(HbctToken::new(b"BMU-1".to_vec(), 100, 50, addr(1), 0).unwrap())
            .unwrap();
        let out = auto_burn_at_slot_close(&mut b, 100);
        assert_eq!(out.entries_removed, 1);
        assert_eq!(out.mwh_burnt, 50);
        assert_eq!(b.len(), 0);
    }

    #[test]
    fn future_slots_survive_partial_burn() {
        let mut b = HbctBook::new();
        b.mint(HbctToken::new(b"BMU-1".to_vec(), 100, 50, addr(1), 0).unwrap())
            .unwrap();
        b.mint(HbctToken::new(b"BMU-1".to_vec(), 200, 30, addr(2), 0).unwrap())
            .unwrap();
        let out = auto_burn_at_slot_close(&mut b, 150);
        assert_eq!(out.entries_removed, 1);
        assert_eq!(out.mwh_burnt, 50);
        assert_eq!(b.len(), 1);
        assert_eq!(b.balance(&b"BMU-1".to_vec(), 200, addr(2)), 30);
    }

    #[test]
    fn many_entries_at_one_slot_all_burnt_together() {
        let mut b = HbctBook::new();
        b.mint(HbctToken::new(b"BMU-1".to_vec(), 100, 50, addr(1), 0).unwrap())
            .unwrap();
        b.mint(HbctToken::new(b"BMU-2".to_vec(), 100, 30, addr(2), 0).unwrap())
            .unwrap();
        b.mint(HbctToken::new(b"BMU-3".to_vec(), 100, 20, addr(3), 0).unwrap())
            .unwrap();
        let out = auto_burn_at_slot_close(&mut b, 100);
        assert_eq!(out.entries_removed, 3);
        assert_eq!(out.mwh_burnt, 100);
        assert!(b.is_empty());
    }
}
