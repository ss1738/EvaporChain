//! Refresh Market — the chain's primary economic activity.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 7:
//!
//! > **Refresh Market** — AMM-priced rent per state object. Continuous
//! > keep-alive flow becomes the chain's primary economic activity.
//!
//! Source attribution (mechanism-design agent, round 2): "Merton-style
//! stochastic control."
//!
//! ## Mechanics
//!
//! Each namespace declares a *capacity* (max concurrently-active
//! state-object slots) at registration. The market price for one
//! epoch of rent at that namespace is an AMM curve over `(used,
//! capacity)`:
//!
//! ```text
//!   rent_rate(u, c) = base × (u + 1)² / c²
//! ```
//!
//! Quadratic in utilisation: empty namespace pays the floor; a
//! namespace at full capacity pays `base × (c+1)²/c² ≈ base`. The
//! `+1` keeps the marginal cost positive even at zero utilisation so
//! squatting on capacity has a price.
//!
//! ## Module map
//!
//! - [`namespace`] — `Namespace { id, capacity, used }`.
//! - [`pricing`] — `rent_rate(used, capacity, base)` AMM curve.
//! - [`market`] — `RefreshMarket` book of namespaces; `pay_rent` and
//!   `reserve_slot` operate against the kernel's `RefreshPool`.

pub mod market;
pub mod namespace;
pub mod pricing;

pub use market::{
    pay_rent, reserve_slot, MarketError, RefreshMarket, ReservationOutcome,
};
pub use namespace::{Namespace, NamespaceId};
pub use pricing::rent_rate;
