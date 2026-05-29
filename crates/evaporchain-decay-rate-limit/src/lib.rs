//! Decay-pressure rate-limiting substrate primitive.
//!
//! The doctrine-native way to throttle actions. Instead of storing a
//! sliding window of timestamps, a [`DecayBucket`] holds a single
//! `pressure` value. Every action adds its `cost` to the pressure; the
//! pressure then **decays through `evaporchain_types::energy_at_epoch`**
//! — the same canonical halving curve the rest of the chain uses — so
//! the bucket recovers on its own. An action is admitted only while the
//! decayed pressure plus its cost stays within `capacity`.
//!
//! "Load you create evaporates, exactly like energy." This unifies the
//! chain's several ad-hoc rate limiters under the decay doctrine, and
//! the state is O(1): one `u64` of pressure plus the last-update epoch,
//! versus a per-action timestamp list.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Pressure decays monotonically between actions.** With no new
//!    actions the bucket strictly recovers toward empty (full
//!    allowance) along `energy_at_epoch` — recovery is automatic, not
//!    a manual reset.
//!
//! 2. **The capacity ceiling is never exceeded by an admitted action.**
//!    A `cost` is admitted only if `decayed_pressure + cost <=
//!    capacity`; on admission the pressure becomes exactly that sum.
//!    A denied action leaves the bucket unchanged.
//!
//! 3. **Half-life is the recovery rate.** A larger `half_life` means
//!    pressure lingers longer (slower recovery, stricter limiter); a
//!    smaller one recovers faster. Capacity is the burst size.
//!
//! ## Module map
//!
//! - [`bucket`] — [`DecayBucket`] single-stream limiter + [`RateError`].
//! - [`limiter`] — [`DecayRateLimiter`]: per-subject keyed limiter.

pub mod bucket;
pub mod limiter;

pub use bucket::{DecayBucket, RateError};
pub use limiter::DecayRateLimiter;

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// Doctrine claim asserted as a structural test.
    ///
    /// Press claim: "Rate limiting on EvaporChain is decay-native. A
    /// bucket admits a burst up to capacity, then refuses further
    /// actions until its pressure has decayed away — and the pressure
    /// decays through the same halving curve as energy. Recovery is
    /// automatic; one held burst does not block forever."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        // capacity 100, half-life 10 epochs.
        let mut b = DecayBucket::new(100, 10, 0).unwrap();

        // Burst: spend the whole capacity at t=0.
        b.try_consume(100, 0).unwrap();
        // Bucket is full → the next unit is refused.
        assert!(matches!(
            b.try_consume(1, 0),
            Err(RateError::RateLimited { .. })
        ));

        // After one half-life (t=10) pressure ≈ 50 → ~50 headroom.
        assert!(b.would_allow(50, 10));
        assert!(!b.would_allow(60, 10));

        // After many half-lives the bucket has fully recovered.
        assert!(b.would_allow(100, 1_000));
        b.try_consume(100, 1_000).unwrap();

        // A denied action never mutates the bucket.
        let before = b.pressure_at(1_000);
        let _ = b.try_consume(100, 1_000); // capacity already full → denied
        assert_eq!(b.pressure_at(1_000), before);
    }
}
