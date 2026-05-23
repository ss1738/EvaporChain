//! Hour-Block Capacity Tokens (HBCT).
//!
//! Per `research/INVENTION_STACK.md` Amendment 2 §A2.4 + §A3.4 (the
//! launch wedge):
//!
//! > **HBCT** — Electricity capacity in hour H decays to 0 at H+1.
//! > Single-λ is dimensionally honest, not metaphor. Battery
//! > state-of-charge IS a decaying inventory.
//! >
//! > "The first L1 to natively price what physics already prices."
//!
//! ## Why this is the launch wedge
//!
//! Per §A3.4 ranking:
//!
//! - $5T global electricity market; battery-storage segment >30% YoY.
//! - GB Elexon BMRS + ENTSO-E APIs are **open** — solo founder can
//!   ship testnet demo with real GB grid data, no regulator approval
//!   needed.
//! - Existing energy chains (Power Ledger, Energy Web, WePower) handle
//!   RECs/PPAs but **none use decay as primitive**.
//! - Concrete B2B customer: battery aggregators (Octopus Kraken,
//!   Habitat Energy) with day-ahead/intraday balancing pain.
//! - Reg score 3 — Ofgem/FERC capacity-market frameworks are
//!   utility-token-friendly.
//!
//! ## Substrate scope
//!
//! - [`token`] — `HbctToken { delivery_location, hour_slot,
//!   mwh_amount, holder, issued_at_epoch }`. Empty/zero rejected.
//! - [`book`] — `HbctBook` per-(location, hour_slot) ledger; mint,
//!   transfer, burn.
//! - [`burn`] — `auto_burn_at_slot_close(book, current_epoch)` walks
//!   the book and burns tokens whose `hour_slot` has closed. The
//!   "decay to 0" the doctrine names — strict, not gradual.
//! - [`oracle`] — `OracleFeed` trait. Real GB Elexon BMRS / ENTSO-E
//!   adapters plug in post-substrate.

pub mod book;
pub mod burn;
pub mod oracle;
pub mod token;

pub use book::{BookError, HbctBook};
pub use burn::{auto_burn_at_slot_close, BurnOutcome};
pub use oracle::{OracleAttestation, OracleFeed};
pub use token::{DeliveryLocation, HbctToken, HourSlot, TokenError, MAX_DELIVERY_LOCATION_LEN};

#[cfg(test)]
mod press_claim_tests {
    //! The press claim lives as a test (INVENTION_STACK §A3.4):
    //! "capacity in hour H decays to 0 at H+1 — single-λ is
    //! dimensionally honest, not metaphor."

    use super::*;
    use crate::burn::auto_burn_at_slot_close;

    fn addr(b: u8) -> evaporchain_types::AccountAddress {
        [b; 32]
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Capacity minted for slot 100 is fully burned at epoch 100.
        // The book is empty afterwards — structural decay, no explicit
        // revoke transaction.
        let mut book = HbctBook::new();
        book.mint(HbctToken::new(b"BMU-1".to_vec(), 100, 500, addr(1), 0).unwrap())
            .unwrap();
        assert_eq!(book.balance(&b"BMU-1".to_vec(), 100, addr(1)), 500);
        // Still alive at epoch 99.
        let out = auto_burn_at_slot_close(&mut book, 99);
        assert_eq!(out.entries_removed, 0);
        assert_eq!(book.balance(&b"BMU-1".to_vec(), 100, addr(1)), 500);
        // Decays to 0 at exactly epoch 100.
        let out = auto_burn_at_slot_close(&mut book, 100);
        assert_eq!(out.entries_removed, 1);
        assert_eq!(out.mwh_burnt, 500);
        assert!(book.is_empty());
    }

    #[test]
    fn is_closed_boundary_is_sharp() {
        let token = HbctToken::new(b"BMU-X".to_vec(), 200, 10, addr(2), 0).unwrap();
        assert!(!token.is_closed(199), "one epoch before slot — still valid");
        assert!(token.is_closed(200), "at slot epoch — closed");
        assert!(token.is_closed(9999), "far past slot — closed");
    }

    #[test]
    fn empty_delivery_location_rejected() {
        let err = HbctToken::new(vec![], 100, 10, addr(1), 0).unwrap_err();
        assert_eq!(err, TokenError::EmptyLocation);
    }

    #[test]
    fn location_over_cap_rejected() {
        // SUB-N5: oversized BMU id can't become a permanent BTreeMap
        // key consuming chain state.
        let big = vec![0u8; MAX_DELIVERY_LOCATION_LEN + 1];
        let err = HbctToken::new(big, 100, 10, addr(1), 0).unwrap_err();
        assert!(matches!(err, TokenError::LocationTooLong { .. }));
    }

    #[test]
    fn slot_at_or_before_issued_epoch_rejected() {
        // You can't issue a token for a slot that has already closed.
        let err = HbctToken::new(b"BMU-1".to_vec(), 50, 10, addr(1), 100).unwrap_err();
        assert!(matches!(err, TokenError::SlotInPast { .. }));
        let err = HbctToken::new(b"BMU-1".to_vec(), 100, 10, addr(1), 100).unwrap_err();
        assert!(matches!(err, TokenError::SlotInPast { .. }));
    }
}
