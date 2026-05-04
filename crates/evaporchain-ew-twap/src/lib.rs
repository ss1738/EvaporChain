//! Energy-Weighted Time-Weighted Average Price (EW-TWAP) oracle.
//!
//! ## Standard TWAP vs EW-TWAP
//!
//! Standard TWAP — used by Uniswap, Sushiswap, every "manipulation-
//! resistant" oracle since 2018 — averages each observation
//! weighted by elapsed time:
//!
//!   TWAP(window) = Σ price_i · Δt_i / Σ Δt_i
//!
//! EW-TWAP weights by energy-at-observation:
//!
//!   EW_TWAP(window) = Σ price_i · energy_i / Σ energy_i
//!
//! ## Why this matters
//!
//! Standard TWAP can be manipulated by sustaining a manipulated
//! price across the window: the longer you hold it, the more it
//! weighs. EW-TWAP makes manipulation structurally harder because
//! observations' energy decays toward floor — keeping a
//! manipulated price weighted requires *re-attesting* it every
//! epoch, which costs the attacker fresh stake / energy per
//! tick. The chain's single-λ binds the manipulation cost to the
//! decay rate.
//!
//! ## Three structural decisions enforced as invariants
//!
//! 1. **Pure-integer fixed-point.** Prices in micros (1.0 = 10⁶);
//!    energies as u64; the EW-TWAP returns micros. Validators
//!    agree byte-for-byte. No floats.
//!
//! 2. **Sliding window with explicit eviction.** A `(timestamp,
//!    energy, price_micros)` triple older than `window_epochs`
//!    is dropped on next update. The window is bounded; the
//!    oracle's state is O(window_epochs).
//!
//! 3. **Empty-window EW-TWAP is an error, not 0.** If every
//!    observation has decayed out of the window or never existed,
//!    the oracle returns `OracleError::NoObservations`. Clients
//!    must handle this; it is *NOT* a 0-price.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT supply prices. Caller submits `(timestamp, price,
//!   energy)`; the oracle weighs and averages.
//! - It does NOT model the chain's decay model. Caller passes
//!   the energy-at-observation directly; the oracle treats it
//!   as authoritative.
//! - It does NOT do confidence intervals or median fallbacks.
//!   V1 ships the weighted mean only.
//!
//! ## Module map
//!
//! - [`obs`] — [`Observation`] + price/energy types.
//! - [`oracle`] — [`EwTwapOracle`] + observe / read / prune.

pub mod obs;
pub mod oracle;

pub use obs::{Observation, PriceMicros};
pub use oracle::{EwTwapOracle, OracleError};
