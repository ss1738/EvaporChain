//! Native demurrage on idle balances — the first consumer of
//! `evaporchain-energy-kernel`'s conservation domain.
//!
//! Per `research/INVENTION_STACK.md`:
//!
//! - **§4.1 #6 (Native Demurrage → Refresh Pool)** — *"Piecewise rate
//!   `r(balance) = max(0, λ_base · log(balance/threshold))`. Sink =
//!   protocol-controlled refresh pool. Closes the philosophy loop."*
//! - **§6 weeks 3–5** — *"Native Demurrage — passive accrual on
//!   `last_touched_height`. Trivial; unlocks Refresh Market and refresh
//!   pool."*
//!
//! ## Shape
//!
//! - Below `threshold`: zero demurrage regardless of how long the balance
//!   has been idle. Hoarding only kicks in past a chain-set threshold.
//! - Above `threshold`: per-epoch rate scales with `log_2(balance /
//!   threshold)` — bigger balances pay more, log-scaled. The integer
//!   approximation uses `bit_length` differences (saturating, deterministic,
//!   chain-safe).
//! - Accrual over `elapsed` epochs uses the linear approximation
//!   `demurrage ≈ balance · r · elapsed` (the second-order term is below
//!   the integer-arithmetic floor for any reasonable rate × elapsed).
//! - All side-effects go through `EnergyRedirect::Demurrage` so the
//!   energy-kernel's conservation invariant remains intact.
//!
//! ## Module map
//!
//! - [`params`] — `DemurrageParams { lambda_base_ppm, threshold }`.
//! - [`rate`] — pure computation of the per-epoch rate (in ppm).
//! - [`accrual`] — `demurrage_owed(balance, last_touched, current, params)`.
//! - [`apply`] — wires the owed amount through `EnergyRedirect::Demurrage`
//!   on an `EnergyAccumulator` + `RefreshPool`.

pub mod accrual;
pub mod apply;
pub mod params;
pub mod rate;

pub use accrual::demurrage_owed;
pub use apply::{apply_demurrage, ApplyError, ApplyOutcome};
pub use params::DemurrageParams;
pub use rate::{rate_ppm, MAX_LOG2_RATIO};
