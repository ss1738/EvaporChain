//! Singh-Shamir Cells (HLTS) — Half-Life Threshold Shares.
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Singh-Shamir Cells (HLTS)** — Half-life threshold shares;
//! > secret recoverable only by surviving high-energy quorum.
//!
//! ## Mechanics
//!
//! Take a `(k, n)`-Shamir-style threshold scheme and attach an energy
//! to each share. Energies decay under λ. Reconstruction is possible
//! iff at least `k` of the `n` shares are still **above** the chain-
//! set survival threshold at the query epoch.
//!
//! As shares decay, the set of alive shares shrinks. When it drops
//! below `k`, the secret is permanently lost — a *time-bounded*
//! capability that the chain enforces by counting alive shares.
//!
//! ## Substrate scope
//!
//! Real Shamir reconstruction (polynomial interpolation over a finite
//! field) lives outside this crate; substrate exposes the **share-
//! survival accounting** that gates whether reconstruction is even
//! attempted. Production swaps in a Lagrange-interpolation impl over
//! `bls12_381::Scalar` or similar.
//!
//! ## Module map
//!
//! - [`share`] — `Share { idx, energy, observed_epoch }`.
//! - [`survival`] — `is_alive(share, λ, query_epoch, threshold)` and
//!   `count_alive(shares, λ, query_epoch, threshold)`.
//! - [`quorum`] — `quorum_alive(shares, k, λ, query_epoch, threshold)`
//!   returns true iff ≥ k of n are alive.

pub mod quorum;
pub mod share;
pub mod survival;

pub use quorum::quorum_alive;
pub use share::Share;
pub use survival::{count_alive, is_alive};
