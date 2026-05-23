//! Refresh Market — the chain's primary economic activity.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 7:
//!
//! > **Refresh Market** — AMM-priced rent per state object. Continuous
//! > keep-alive flow becomes the chain's primary economic activity.
//!
//! Source attribution (mechanism-design agent, round 2): "Merton-style
//! stochastic control."
//!
//! ## Mechanics
//!
//! Each namespace declares a *capacity* (max concurrently-active
//! state-object slots) at registration. The market price for one
//! epoch of rent at that namespace is an AMM curve over `(used,
//! capacity)`:
//!
//! ```text
//!   rent_rate(u, c) = base × (u + 1)² / c²
//! ```
//!
//! Quadratic in utilisation: empty namespace pays the floor; a
//! namespace at full capacity pays `base × (c+1)²/c² ≈ base`. The
//! `+1` keeps the marginal cost positive even at zero utilisation so
//! squatting on capacity has a price.
//!
//! ## Module map
//!
//! - [`namespace`] — `Namespace { id, capacity, used }`.
//! - [`pricing`] — `rent_rate(used, capacity, base)` AMM curve.
//! - [`market`] — `RefreshMarket` book of namespaces; `pay_rent` and
//!   `reserve_slot` operate against the kernel's `RefreshPool`.

pub mod market;
pub mod namespace;
pub mod pricing;

pub use market::{pay_rent, reserve_slot, MarketError, RefreshMarket, ReservationOutcome};
pub use namespace::{Namespace, NamespaceId};
pub use pricing::rent_rate;

#[cfg(test)]
mod press_claim_tests {
    //! The press claim lives as a test (INVENTION_STACK §4.1 row 7):
    //! "Refresh Market — AMM-priced rent per state object. Continuous
    //! keep-alive flow becomes the chain's primary economic activity."
    //!
    //! Source attribution (mechanism-design agent, round 2):
    //! "Merton-style stochastic control."
    //!
    //! Three invariants that MUST hold at the runtime level:
    //!
    //! 1. **Monotone in utilisation** — rent is non-decreasing as `used`
    //!    increases: more demand → higher price, no squatting incentive.
    //! 2. **Zero utilisation still costs** — the "+1" in `(used+1)²`
    //!    keeps rent positive even at `used=0`, so registering capacity
    //!    without using it still drains the operator's pool credit.
    //! 3. **Full namespace locked** — `reserve_slot` returns `NoCapacity`
    //!    when `used >= capacity`; the capacity ceiling is enforced by the
    //!    market, not by the caller.

    use evaporchain_energy_kernel::{EnergyAccumulator, RefreshPool};

    use crate::{rent_rate, reserve_slot, MarketError, RefreshMarket};

    fn ns(b: u8) -> Vec<u8> {
        vec![b; 4]
    }

    fn pool_with(id: Vec<u8>, amount: u64) -> RefreshPool {
        let mut p = RefreshPool::new();
        p.accrue(id, amount, 0);
        p
    }

    // ── 1. Monotone in utilisation ────────────────────────────────────

    #[test]
    fn rent_increases_monotonically_with_utilisation() {
        let base = 1_000_000;
        let cap = 100;
        let r0   = rent_rate(0,  cap, base).unwrap();
        let r25  = rent_rate(25, cap, base).unwrap();
        let r50  = rent_rate(50, cap, base).unwrap();
        let r100 = rent_rate(100, cap, base).unwrap();
        assert!(r25 > r0,  "rent at 25% must exceed floor");
        assert!(r50 > r25, "rent at 50% must exceed rent at 25%");
        assert!(r100 > r50,"rent at full capacity must be the highest");
    }

    // ── 2. Zero utilisation still costs ──────────────────────────────

    #[test]
    fn empty_namespace_pays_nonzero_rent() {
        // capacity=1 means (1+1)²/(1²) = 4 × base; even with large cap,
        // the "+1" guarantees at least 1 Energy unit.
        let r = rent_rate(0, 1_000_000, 1_000_000).unwrap();
        assert!(r >= 1, "zero utilisation must still pay rent (squatting penalty)");
    }

    // ── 3. Full namespace locked ──────────────────────────────────────

    #[test]
    fn full_namespace_reserve_rejected() {
        let mut m = RefreshMarket::new(1_000);
        let n = m.register(ns(1), 5);
        n.used = 5; // at capacity
        let mut pool = pool_with(ns(1), 1_000_000);
        let mut acc = EnergyAccumulator::default();
        let err = reserve_slot(&mut m, &mut pool, &mut acc, &ns(1), 0).unwrap_err();
        assert!(matches!(err, MarketError::NoCapacity { .. }),
            "full namespace must return NoCapacity, got {err:?}");
    }
}
