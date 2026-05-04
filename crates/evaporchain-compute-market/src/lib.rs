//! Energy-Budgeted Compute Marketplace.
//!
//! ## Lifecycle
//!
//! 1. **Post.** Buyer posts a `Job { spec_hash, bounty, bond_required,
//!    deadline_epochs, proof_energy_floor }` at epoch `t₀`.
//! 2. **Claim.** Worker calls `claim(job, bond)` at epoch `t_claim ≤ t₀
//!    + deadline_epochs`, locking their bond. Each job has at most one
//!    claimant in `Claimed` state.
//! 3. **Submit.** Claimant submits a result at epoch `t_sub` with
//!    `proof_energy` — the chain's higher layer computes this from the
//!    proof's freshness (typically initial work-energy decayed from
//!    `t_claim` to `t_sub`).
//! 4. **Settle.**
//!    - If `t_sub ≤ t₀ + deadline_epochs` AND `proof_energy ≥
//!      proof_energy_floor`: bounty + bond returned to worker.
//!    - Else: bond slashed to buyer; bounty refunded to buyer.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Bond locks the worker into the deadline.** Once claimed, the
//!    bond is held until either successful submission or deadline
//!    expiry. The chain calls `expire_overdue` to slash bonds of
//!    workers who sat on jobs.
//!
//! 2. **Proof-energy floor structurally rejects stale workers.** A
//!    submission whose proof-energy has decayed below the floor is
//!    rejected even before the deadline check. Workers cannot wait
//!    until the last second hoping the buyer's check is lazy.
//!
//! 3. **One-shot terminal states.** Once settled (Paid or Slashed),
//!    the job cannot be re-claimed. Validators agree.
//!
//! ## Module map
//!
//! - [`job`] — [`Job`] state machine + lifecycle errors.

pub mod job;

pub use job::{Job, JobError, JobId, JobState};
