//! Sentinel — autonomic chain-parameter governance.
//!
//! Per `research/INVENTION_STACK.md` Amendment 2 §A2.5:
//!
//! > **Sentinel** — autonomic parameter governance via decay-weighted
//! > LLSA voting within hard-coded bounds.
//! >
//! > "EvaporChain is the first chain that governs itself the way a
//! > body does — through homeostasis, not legislators."
//!
//! ## What homeostasis means here
//!
//! Each governable parameter has hard-coded `(min, max)` bounds set
//! at genesis. Within those bounds, validators vote continuously on
//! the parameter's value. Vote weights *decay* with the chain's
//! global λ (recent votes dominate; ancient ones evaporate). The
//! sentinel controller continuously moves the parameter toward the
//! decay-weighted-average vote, subject to a per-tick max-step cap.
//!
//! No legislators, no proposals, no time-bounded voting periods —
//! the chain finds its set-point continuously. Like body
//! temperature, like blood pH, like a thermostat. Hence "homeostasis,
//! not legislators."
//!
//! Pairs with `evaporchain-llsa`: any *bounds change* (i.e. moving
//! the `(min, max)` envelope itself) requires a Coq-checked LLSA
//! proof that the new bounds preserve chain invariants.
//!
//! ## Substrate
//!
//! - [`parameter`] — `BoundedParameter { id, current, min, max }`.
//! - [`vote`] — `Vote { validator_id, target, observed_epoch }`.
//! - [`controller`] — `SentinelController::propose_adjustment(param,
//!   votes, λ, current_epoch, max_step)` returns the new parameter
//!   value clamped within bounds and within the per-tick step cap.

pub mod controller;
pub mod parameter;
pub mod vote;

pub use controller::{propose_adjustment, ControllerError};
pub use parameter::{BoundedParameter, ParameterError};
pub use vote::Vote;

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_energy_kernel::{ChainLambda, Lambda};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Sentinel governs chain parameters by
    /// homeostasis, not legislators. Vote weights decay with chain
    /// λ; the parameter moves toward the decay-weighted average,
    /// capped per tick by `max_step`, and is always clamped to the
    /// hard-coded `(min, max)` bounds. Empty vote sets are rejected
    /// via `ControllerError::NoVotes` rather than silently leaving
    /// the parameter unchanged."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let param = BoundedParameter::new(1, 50, 0, 100).unwrap();
        let lambda = ChainLambda::new(Lambda::from_epochs(100));

        // Empty votes → typed error.
        assert!(matches!(
            propose_adjustment(&param, &[], lambda, 0, 5),
            Err(ControllerError::NoVotes)
        ));

        // Three recent unanimous votes for 80 → param moves toward 80
        // but is capped by max_step=5 → new = 55.
        let votes = vec![
            Vote::new(1, 80, 0),
            Vote::new(2, 80, 0),
            Vote::new(3, 80, 0),
        ];
        let new = propose_adjustment(&param, &votes, lambda, 0, 5).unwrap();
        assert_eq!(new, 55);

        // Bounds enforced: votes for 999 with max_step huge → clamps to max=100.
        let votes_high = vec![Vote::new(1, 999, 0)];
        let new = propose_adjustment(&param, &votes_high, lambda, 0, 1_000).unwrap();
        assert_eq!(new, 100);
    }
}
