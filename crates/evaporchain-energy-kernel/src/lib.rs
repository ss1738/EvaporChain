//! Single-λ substrate for EvaporChain.
//!
//! Per `research/INVENTION_STACK.md`:
//!
//! - **§1.1 Single-λ Principle**: the chain has *one* fundamental constant —
//!   λ (the decay rate). Every layer (consensus, mempool, time, gas, stake,
//!   governance, capabilities, identity, demurrage) reads from this one
//!   number. This crate owns it.
//! - **§1.2 Conservation Invariant**: total energy across
//!   {accounts + stake + refresh_pool + slashed_pool} decreases monotonically
//!   only via λ — never by destruction in any other transition. This crate
//!   defines the conservation domain (`Compartment`), the accumulator that
//!   tracks each compartment, and the verifier that audits transitions.
//!
//! The `energy_at_epoch` decay function itself is owned by
//! `evaporchain-types::energy_at_epoch` (mechanized in
//! `research/coq/EnergyDecayMonotonicity.v`); this crate re-exports it so
//! every chain layer reaches it through the same canonical entry point.

pub mod compartment;
pub mod conservation;
pub mod lambda;
pub mod redirect;
pub mod refresh_pool;

pub use compartment::{Compartment, EnergyAccumulator};
pub use conservation::{ConservationCheck, ConservationViolation};
pub use evaporchain_types::{energy_at_epoch, Energy, HalfLife};
pub use lambda::{ChainLambda, Lambda, DEFAULT_LAMBDA};
pub use redirect::{EnergyRedirect, RedirectKind};
pub use refresh_pool::{RefreshCredit, RefreshPool};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "evaporchain-energy-kernel owns the chain's
    /// single-λ conservation domain. (a) `EnergyAccumulator.total()`
    /// is the sum of {Accounts, Stake, RefreshPool, SlashedPool} —
    /// the conservation quantity audited every block. (b) `debit`
    /// fails closed when a compartment is short (no negative energy,
    /// no silent wrap-around). (c) ConservationCheck::redirect
    /// detects total drift between two accumulator states."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Construct an accumulator with known per-bucket totals.
        let acc = EnergyAccumulator::new(1_000, 500, 200, 100);
        assert_eq!(acc.total(), 1_800);

        // ALL canonical compartment order.
        assert_eq!(Compartment::ALL.len(), 4);
        assert_eq!(Compartment::ALL[0], Compartment::Accounts);

        // Credit + debit round-trip.
        let mut a = EnergyAccumulator::new(1_000, 0, 0, 0);
        a.credit(Compartment::Stake, 500);
        a.debit(Compartment::Accounts, 100).unwrap();
        assert_eq!(a.total(), 1_400);

        // Debit underflow fails closed (no silent wrap-around).
        let mut underflow = EnergyAccumulator::new(50, 0, 0, 0);
        assert!(underflow.debit(Compartment::Accounts, 100).is_err());
        // Compartment unchanged on failed debit.
        assert_eq!(underflow.total(), 50);

        // ConservationCheck::redirect rejects total drift.
        let before = EnergyAccumulator::new(1_000, 0, 0, 0);
        let after_ok = EnergyAccumulator::new(900, 100, 0, 0); // total preserved
        let after_bad = EnergyAccumulator::new(900, 0, 0, 0); // 100 disappeared
        ConservationCheck::redirect(&before, &after_ok).unwrap();
        assert!(matches!(
            ConservationCheck::redirect(&before, &after_bad),
            Err(ConservationViolation::RedirectChangedTotal { .. })
        ));
    }
}
