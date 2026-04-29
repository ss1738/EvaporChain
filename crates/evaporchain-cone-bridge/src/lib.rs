//! Cone-Merged Bridges — Tier 2.
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Cone-Merged Bridges** — Bridges valid only inside intersection
//! > of both chains' decay cones; replay-immune by construction.
//!
//! ## What "decay cone" means here
//!
//! Each chain attaches an *energy cone* to a bridge tx:
//!
//! ```text
//!   Cone(chain) = { (epoch, energy) : energy ≥ chain.threshold(epoch) }
//! ```
//!
//! The bridge tx is valid on chain B iff **the queried `(epoch, energy)`
//! point lies inside the intersection** of A's cone and B's cone:
//!
//! - A's cone: A's λ-decayed energy at epoch ≥ A's threshold.
//! - B's cone: B's λ-decayed energy at epoch ≥ B's threshold.
//!
//! Replay across chains is impossible because A's energy after
//! crossing differs from B's (each chain's λ is its own), so the
//! tx is valid only inside the *time-bounded intersection window*.
//!
//! ## Substrate scope
//!
//! - [`cone`] — `EnergyCone { chain_lambda, threshold,
//!   committed_energy, observed_epoch }`. `is_inside(query_epoch)`
//!   checks if the queried point is inside the cone.
//! - [`bridge`] — `bridge_valid(cone_a, cone_b, query_epoch)` —
//!   true iff both cones contain the queried epoch.

pub mod bridge;
pub mod cone;

pub use bridge::bridge_valid;
pub use cone::EnergyCone;
