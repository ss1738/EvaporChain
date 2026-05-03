//! Maximum-Caliber Consensus (MCC) — Tier-0 fork-choice.
//!
//! Per `research/INVENTION_STACK.md` §A1.2 T1:
//!
//! > **Maximum-Caliber Consensus (MCC)** — fork-choice rule selecting
//! > the chain trajectory that maximizes path-entropy under
//! > `⟨ΔE⟩ = λ`. Jaynes 1980 (Maximum Caliber); Pressé-Ghosh-Lee-Dill
//! > *Rev. Mod. Phys.* 2013.
//! >
//! > "Our fork choice is the unique distribution maximizing path-
//! > entropy subject to one thermodynamic constraint, with closed-
//! > form Perron solution."
//!
//! ## Math note — "closed-form Perron solution" on a DAG
//!
//! Layer 2 of the doctrine punch list flagged this language as
//! mathematically vacuous on EvaporChain's actual structure: the
//! [`evaporchain_light_cone::LightCone`] is a strict DAG, so its
//! adjacency matrix `M` is nilpotent (every eigenvalue is 0) and no
//! nontrivial Perron-Frobenius eigenvector exists. The Perron
//! statement applies to *strongly-connected, irreducible* Markov
//! chains, not DAG fork-choice.
//!
//! The good news: the doctrine's *actual* mathematical commitment
//! ("the unique distribution maximizing path-entropy subject to one
//! thermodynamic constraint") is met by a **Boltzmann argmax** —
//! Jaynes' canonical closed form for MaxCal under a single
//! constraint. For a candidate set of trajectories `{τ_i}` with
//! path-energies `E_i`:
//!
//! ```text
//!   p(τ_i) ∝ exp(−β · E_i)   with β = 1/λ  (Lagrange multiplier)
//!   argmax_i p(τ_i)  =  argmin_i E_i  at large β
//! ```
//!
//! That is exactly what [`choose::mcc_choose`] computes via
//! [`evaporchain_cfm::boltzmann_weight`]. So the *code* matches the
//! *theorem*; the doctrine wording is what needs amending — "Perron
//! solution" should read "Lagrangian closed form" or "Boltzmann
//! distribution over candidate trajectories" once Satyawan signs off
//! on the §A1.2 T1 amendment.
//!
//! Until that amendment lands, treat the in-source code as the
//! authoritative spec for MCC, and read the doctrine line as
//! marketing-grade phrasing of the same underlying math.
//!
//! ## Operationalisation
//!
//! Maximum-caliber under the `⟨ΔE⟩ = λ` constraint puts a Boltzmann
//! weight on each trajectory: `w(traj) ∝ exp(−β · E(traj))` with
//! `β = 1/λ`. The fork-choice picks the trajectory with the maximum
//! caliber.
//!
//! Same algebra as `evaporchain-cfm`, lifted from the mempool to the
//! consensus path: instead of reweighting fee buckets, we reweight
//! candidate forks. We *reuse* `evaporchain-cfm::weight::boltzmann_weight`
//! and `beta_millibits_per_fee` so the temperature plumbing is shared
//! across the whole fee-market / fork-choice family.
//!
//! ## Module map
//!
//! - [`trajectory`] — `Trajectory` newtype = ordered `Vec<BlockId>`.
//! - [`caliber`] — `path_energy(traj, lc)` and `path_caliber(traj, lc,
//!   beta_mb)`.
//! - [`choose`] — `mcc_choose(forks, lc, beta_mb)` = argmax-caliber
//!   selection over a candidate set of trajectories.

pub mod caliber;
pub mod choose;
pub mod trajectory;

pub use caliber::{path_caliber, path_energy};
pub use choose::{mcc_choose, McccError};
pub use trajectory::Trajectory;
