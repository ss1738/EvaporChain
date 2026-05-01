//! GB Elexon BMRS v1 oracle adapter for HBCT settlement.
//!
//! Implements [`evaporchain_hbct::OracleFeed`] against the real Elexon BMRS
//! REST API.  The relevant dataset is **B1790** (Actual Generation Output per
//! Generation Unit), which returns confirmed MWh output per BMU per
//! settlement period.
//!
//! # Chain-epoch → settlement period mapping
//!
//! Elexon divides each calendar day into 50 half-hour settlement periods
//! (SP 1 = 00:00–00:30 UTC, SP 50 = 23:30–00:00).  The mapping from chain
//! epochs to settlement periods requires two config values:
//!
//! - `genesis_unix_ts`: Unix timestamp (seconds) of the chain's epoch 0.
//! - `epoch_duration_s`: seconds per chain epoch (default 12 s, matching
//!   the evaporchain-consensus slot time).
//!
//! Given a token `hour_slot` (the chain epoch at which the capacity slot
//! *closes*):
//!
//! ```text
//! slot_unix_ts = genesis_unix_ts + hour_slot * epoch_duration_s
//! settlement_date = UTC date of slot_unix_ts
//! settlement_period = floor((slot_unix_ts mod 86400) / 1800) + 1   (1..=48)
//! ```
//!
//! SP 49 and 50 cover long-day BST transitions; we clamp at 48 for
//! robustness.
//!
//! # BMU ID
//!
//! `DeliveryLocation` bytes are interpreted as a UTF-8 NGC BMU identifier
//! (e.g. `b"T_RATS-1"`).  Non-UTF-8 locations always return `None`.
//!
//! # Error policy
//!
//! Network failures, parse failures, or unknown BMU IDs all yield `None`
//! on the `OracleFeed::attest` call — the chain's burn path treats a missing
//! attestation as unconfirmed delivery and does not burn.

pub mod client;
pub mod mapping;

pub use client::{ElexonConfig, ElexonOracleFeed};
