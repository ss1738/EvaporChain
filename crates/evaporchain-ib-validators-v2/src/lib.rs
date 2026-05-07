//! IB Validators V2 — structural jail layer over the V1 vote gate.
//!
//! V1 (`evaporchain-ib-validators`) ships `ib_vote(local, prior,
//! params) → Commit | Abstain` based on KL divergence. It says
//! nothing about *which* validators are eligible to vote — any
//! validator that runs the gate gets a verdict.
//!
//! V2 wraps the V1 gate with three structural rejection paths:
//!
//! 1. **CHSH-failed-window jail** — a validator that was active
//!    during an epoch where the chain's `BellCertificate` failed
//!    is jailed for `jail_epochs` epochs. Bell-Beacon V2 supplies
//!    the failure signal; this crate consumes the per-validator
//!    activity trace and toggles jail state.
//!
//! 2. **Energy-floor jail** — a validator whose energy has decayed
//!    below `energy_floor` cannot vote until refreshed. Pulls in
//!    EvaporChain's energy primitive at the validator-set layer.
//!
//! 3. **Explicit slash** — operator can jail a validator with a
//!    typed reason (double-sign, equivocation, manual ban). Same
//!    deterministic expiry rules.
//!
//! ## Validator-determinism
//!
//! - `JailState` stores entries in a `BTreeMap<ValidatorId, _>`
//!   so iteration order is canonical.
//! - Jail expiry is computed against a single `current_epoch` field;
//!   no wall-clock dependency.
//! - `ib_vote_v2` is a pure function of `(local, prior, params,
//!   validator_id, energy, jail_state, current_epoch)`.

pub mod jail;
pub mod vote;

pub use jail::{JailEntry, JailReason, JailState, ValidatorId};
pub use vote::{ib_vote_v2, VoteV2, VoteV2Error};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_ib_validators::{IbParams, StateSignature};
    use vote::{apply_chsh_failure_jail, apply_energy_jail};

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "IB Validators V2 layers a structural jail over
    /// the V1 IB vote gate. CHSH-failed-window jail blocks every
    /// listed participant for `jail_epochs` epochs; energy-floor jail
    /// blocks any validator whose energy decayed below floor; explicit
    /// slashes carry a typed reason. After jail expires, V1 semantics
    /// apply unchanged."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let id1: jail::ValidatorId = [1u8; 32];
        let id2: jail::ValidatorId = [2u8; 32];

        // High-KL local vs. uniform prior → V1 commits.
        let local = StateSignature::from_energies(&vec![0u64; 16], 1024);
        let prior_energies: Vec<u64> = (0..16).map(|i| i as u64 * 64).collect();
        let prior = StateSignature::from_energies(&prior_energies, 1024);
        let params = IbParams { lambda_mb: 100 };

        let mut js = JailState::new();

        // CHSH window jail covers id1 for 50 epochs from now=0.
        apply_chsh_failure_jail(&mut js, &[id1], 100, 200, 0, 50);
        let v = ib_vote_v2(&local, &prior, &params, &id1, 1_000, 10, &js, 10).unwrap();
        assert!(matches!(
            v,
            VoteV2::Jailed {
                reason: JailReason::ChshFailedWindow { .. }
            }
        ));

        // Past expiry → V1 vote semantics apply (high-KL → Commit).
        let v_free = ib_vote_v2(&local, &prior, &params, &id1, 1_000, 10, &js, 50).unwrap();
        assert_eq!(v_free, VoteV2::Commit);

        // Energy below floor jails on the spot without persisting state.
        let v_lo = ib_vote_v2(&local, &prior, &params, &id2, 5, 10, &JailState::new(), 0).unwrap();
        assert!(matches!(
            v_lo,
            VoteV2::Jailed {
                reason: JailReason::EnergyBelowFloor { .. }
            }
        ));

        // apply_energy_jail persists the entry idempotently.
        let mut js2 = JailState::new();
        assert!(apply_energy_jail(&mut js2, id2, 5, 10, 200));
        assert!(js2.is_jailed(&id2, 50));
        assert!(!apply_energy_jail(&mut js2, id1, 1_000, 10, 200));
        assert!(!js2.is_jailed(&id1, 50));

        // ZeroFloor rejected (must be > 0).
        let res = ib_vote_v2(&local, &prior, &params, &id1, 100, 0, &JailState::new(), 0);
        assert_eq!(res.unwrap_err(), VoteV2Error::ZeroFloor);
    }
}
