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
pub use token::{DeliveryLocation, HbctToken, HourSlot, TokenError};
