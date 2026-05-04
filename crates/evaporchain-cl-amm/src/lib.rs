//! Singh Pool — decay-aware constant-product AMM.
//!
//! ## What this crate is
//!
//! A constant-product AMM (`x · y = k`) where every LP share
//! carries an **energy tag**. Withdrawals are gated by energy:
//! shares whose energy has decayed below the pool's floor can
//! deposit / re-anchor but cannot withdraw. Mercenary capital
//! that briefly provides liquidity to farm subsidies and leaves
//! becomes structurally unprofitable because their LP shares
//! decay below floor before they can extract.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **`x · y` invariant under swap.** Standard constant-product
//!    math, integer-only with floor-division. After any swap
//!    `(Δx_in, Δy_out)`, `(x + Δx_in)·(y - Δy_out) ≥ x · y` —
//!    fees + integer floor make the post-state product ≥ the
//!    pre-state product. (Strictly equal in the no-fee real-
//!    valued limit.)
//!
//! 2. **LP-share total = sum of individual shares.** Mint /
//!    burn / decay all preserve the equation
//!    `total_shares == Σ holders.shares`. No phantom shares.
//!
//! 3. **Decay-floor withdrawal gate.** A holder with `energy <
//!    pool.energy_floor` calls `withdraw()` → returns
//!    `EnergyBelowFloor`, even if their share count is positive.
//!    This is the load-bearing claim.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT implement concentrated-liquidity (Uniswap v3).
//!   V1 is xy=k. Concentrated-liquidity ranges are V2.
//! - It does NOT model fees explicitly. V1 has a flat fee in
//!   basis points; routing / dynamic fees are V2.
//! - It does NOT do impermanent-loss accounting. The chain's
//!   higher layer can compute IL externally from the LP records.
//!
//! ## Module map
//!
//! - [`pool`] — [`SinghPool`] + swap / mint / burn / re-anchor.
//! - [`share`] — [`LpShare`] + the holder-keyed share table.

pub mod pool;
pub mod share;

pub use pool::{PoolError, SinghPool};
pub use share::{HolderId, LpShare};
