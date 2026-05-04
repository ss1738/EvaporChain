//! Energy-Clocked Coverage — decay-native insurance.
//!
//! ## Lifecycle
//!
//! 1. **Open** a policy at epoch t₀ with `premium_per_epoch` and
//!    `coverage_max`. Premium accumulates linearly.
//! 2. **Tick** epochs forward. Each tick adds `premium_per_epoch`
//!    to `premium_paid`; the chain charges this externally.
//! 3. **File a claim** at epoch t_claim referencing an `incident_at`
//!    epoch with `claim_energy_at_incident` initial energy. The
//!    claim's energy decays from `incident_at` toward `t_claim`
//!    under the chain's λ; the file gate requires
//!    `decayed_energy ≥ claim_floor`.
//! 4. **Pay out** up to `coverage_max`; pro-rated by `decayed_energy
//!    / claim_floor` (so very-fresh claims pay full coverage,
//!    near-floor claims pay less).
//! 5. **Close** the policy after a claim or at expiry.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Premium is monotone.** Each tick adds; never subtracts.
//!    The longer you hold coverage, the more you've paid.
//!
//! 2. **Claim-energy floor is structural.** A stale claim
//!    (`decayed_energy < claim_floor`) is rejected even with full
//!    coverage available. Insurers don't pay for incidents the
//!    chain can no longer freshly attest.
//!
//! 3. **Payout is bounded.** Cannot exceed `coverage_max` minus
//!    already-paid claims. Pro-rating uses integer arithmetic.
//!
//! ## Module map
//!
//! - [`policy`] — [`Policy`] state machine + claim filing.

pub mod policy;

pub use policy::{Policy, PolicyError, PolicyId, PolicyState};
