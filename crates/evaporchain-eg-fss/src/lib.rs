//! Evaporative Pixel (EG-FSS) — Tier 2.
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Evaporative Pixel (EG-FSS)** — Energy-indexed forward-secure
//! > signatures; underwrites Evaporated-Fork Certs at the signature
//! > layer.
//!
//! ## Forward-secure signatures
//!
//! Bellare-Miner 1999: a signing key evolves over discrete time
//! periods. After a key has evolved past period `t`, an attacker who
//! later steals the current key cannot forge signatures from periods
//! `< t`. The "evolved" key irreversibly destroys the information
//! needed to backdate.
//!
//! ## EG-FSS twist
//!
//! Periods aren't wall-clock — they're **energy windows**. The key
//! evolves once enough chain energy has been spent to cross the
//! next window boundary. Stealing a key past the current energy
//! window can't backdate signatures into earlier windows because
//! the chain's *aggregate* energy expenditure is publicly known.
//!
//! ## Substrate scope
//!
//! Real Bellare-Miner FSS uses RSA over composite N or pairing
//! over BLS12-381. Substrate uses domain-separated blake3 chain as
//! the one-way evolution function — sufficient for downstream
//! type-checking. Production swaps in the actual cryptographic FSS.
//!
//! ## Module map
//!
//! - [`key`] — `EgFssKey { period_index, key_material }` with
//!   `evolve(amount_of_energy_spent, threshold_per_period)`.
//! - [`sign`] — `sign(key, message) -> Signature`.
//! - [`verify`] — `verify(period_root, period_index, message, sig)`
//!   — uses the period root the chain remembers; old signatures
//!   stay verifiable.

pub mod key;
pub mod sign;
pub mod verify;

pub use key::{EgFssKey, KeyError};
pub use sign::{sign, Signature};
pub use verify::{verify, VerifyError};
