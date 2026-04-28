//! TUR Liveness Detector — Barato-Seifert 2015.
//!
//! Per `research/INVENTION_STACK.md` §A1.3:
//!
//! > **TUR Liveness Detector** | Barato-Seifert 2015 (Thermodynamic
//! > Uncertainty Relation) | falsifiable thermodynamic liveness
//! > oracle: `Var(J)/⟨J⟩² ≥ 2/Σ`. Cheap passive monitor.
//!
//! ## What this gives the chain
//!
//! For any time-extensive current `J` in a non-equilibrium steady
//! state, the TUR says
//!
//! ```text
//!   Var(J) / ⟨J⟩² ≥ 2 / Σ
//! ```
//!
//! where `Σ` is the total entropy production over the same window.
//! In chain terms: take `J` = block production rate over a window
//! (or any other operational current — fee revenue rate, refresh
//! payouts, etc.) and `Σ` = the chain's accounting of total entropy
//! production from fees + slashing + demurrage.
//!
//! If a passive observer measures `Var(J)/⟨J⟩² > 2/Σ_observed`,
//! the chain is *more fluctuating than thermodynamics allows* —
//! i.e. some validator is generating excess fluctuation outside the
//! accounted-for entropy budget. Byzantine activity is flagged.
//!
//! Crucially this is **falsifiable** — the bound is exact (not a
//! one-sided inequality of unknown tightness), so a violation is a
//! formal proof of fault, not a heuristic alarm.
//!
//! ## Module map
//!
//! - [`stats`] — `mean`, `variance`, `relative_variance` integer
//!   estimators over a `&[u64]` sample window.
//! - [`bound`] — `tur_bound(sigma)` returns the TUR ratio `2/Σ` in
//!   fixed-point.
//! - [`check`] — `tur_check(samples, sigma)` returns `Verdict::Ok`
//!   or `Verdict::Violation { observed, bound }`.

pub mod bound;
pub mod check;
pub mod stats;

pub use bound::{tur_bound_fixed, FIXED_POINT_BITS, FIXED_POINT_SCALE};
pub use check::{tur_check, Verdict};
pub use stats::{mean, relative_variance_fixed, variance};
