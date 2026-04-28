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
