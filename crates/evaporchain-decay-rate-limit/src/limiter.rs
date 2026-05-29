//! [`DecayRateLimiter`] — a per-subject keyed decay-pressure limiter.
//!
//! Wraps one [`DecayBucket`] per subject (e.g. an account address),
//! created lazily on first use with a shared capacity + half-life.
//! Idle subjects whose pressure has fully recovered can be pruned to
//! reclaim memory.

use std::collections::HashMap;

use crate::bucket::{DecayBucket, RateError};

/// Per-subject rate limiter keyed by a 32-byte identifier.
#[derive(Debug, Clone)]
pub struct DecayRateLimiter {
    capacity: u64,
    half_life: u64,
    buckets: HashMap<[u8; 32], DecayBucket>,
}

impl DecayRateLimiter {
    /// Create a limiter with a shared capacity + half-life for every
    /// subject. Validates the parameters once up front.
    pub fn new(capacity: u64, half_life: u64) -> Result<Self, RateError> {
        // Validate via a probe bucket; discard it.
        DecayBucket::new(capacity, half_life, 0)?;
        Ok(Self {
            capacity,
            half_life,
            buckets: HashMap::new(),
        })
    }

    fn bucket_mut(&mut self, subject: [u8; 32], now: u64) -> &mut DecayBucket {
        let cap = self.capacity;
        let hl = self.half_life;
        self.buckets
            .entry(subject)
            .or_insert_with(|| DecayBucket::new(cap, hl, now).expect("params validated in new"))
    }

    /// Attempt to admit `cost` for `subject` at `now`. Lazily creates
    /// the subject's bucket on first use.
    pub fn try_consume(
        &mut self,
        subject: [u8; 32],
        cost: u64,
        now: u64,
    ) -> Result<(), RateError> {
        self.bucket_mut(subject, now).try_consume(cost, now)
    }

    /// Whether `cost` would be admitted for `subject` at `now`. A
    /// never-seen subject is treated as an empty bucket.
    pub fn would_allow(&self, subject: &[u8; 32], cost: u64, now: u64) -> bool {
        match self.buckets.get(subject) {
            Some(b) => b.would_allow(cost, now),
            None => cost <= self.capacity,
        }
    }

    /// Current decayed pressure for `subject` (0 if never seen).
    pub fn pressure(&self, subject: &[u8; 32], now: u64) -> u64 {
        self.buckets
            .get(subject)
            .map(|b| b.pressure_at(now))
            .unwrap_or(0)
    }

    /// Headroom for `subject` (full capacity if never seen).
    pub fn available(&self, subject: &[u8; 32], now: u64) -> u64 {
        self.buckets
            .get(subject)
            .map(|b| b.available(now))
            .unwrap_or(self.capacity)
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    pub fn half_life(&self) -> u64 {
        self.half_life
    }

    /// Number of subjects currently holding a bucket.
    pub fn tracked(&self) -> usize {
        self.buckets.len()
    }

    /// Drop buckets that have fully recovered (pressure 0) at `now`,
    /// reclaiming memory for idle subjects — recovered pressure has
    /// evaporated, so the bucket carries no state worth keeping.
    /// Returns the number of buckets pruned.
    pub fn prune_recovered(&mut self, now: u64) -> usize {
        let before = self.buckets.len();
        self.buckets.retain(|_, b| b.pressure_at(now) > 0);
        before - self.buckets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a() -> [u8; 32] {
        [0xAA; 32]
    }
    fn b() -> [u8; 32] {
        [0xBB; 32]
    }

    #[test]
    fn new_rejects_bad_params() {
        assert_eq!(DecayRateLimiter::new(0, 10).unwrap_err(), RateError::ZeroCapacity);
        assert_eq!(DecayRateLimiter::new(100, 0).unwrap_err(), RateError::ZeroHalfLife);
    }

    #[test]
    fn unseen_subject_has_full_allowance() {
        let rl = DecayRateLimiter::new(100, 10).unwrap();
        assert!(rl.would_allow(&a(), 100, 0));
        assert_eq!(rl.available(&a(), 0), 100);
        assert_eq!(rl.pressure(&a(), 0), 0);
        assert_eq!(rl.tracked(), 0);
    }

    #[test]
    fn subjects_are_isolated() {
        let mut rl = DecayRateLimiter::new(100, 10).unwrap();
        rl.try_consume(a(), 100, 0).unwrap();
        // a() is now full; b() is untouched.
        assert!(matches!(
            rl.try_consume(a(), 1, 0),
            Err(RateError::RateLimited { .. })
        ));
        assert!(rl.would_allow(&b(), 100, 0));
        rl.try_consume(b(), 100, 0).unwrap();
        assert_eq!(rl.tracked(), 2);
    }

    #[test]
    fn subject_recovers_over_time() {
        let mut rl = DecayRateLimiter::new(100, 10).unwrap();
        rl.try_consume(a(), 100, 0).unwrap();
        assert_eq!(rl.pressure(&a(), 10), 50);
        assert!(rl.would_allow(&a(), 50, 10));
        assert!(rl.would_allow(&a(), 100, 10_000));
    }

    #[test]
    fn prune_drops_recovered_keeps_loaded() {
        let mut rl = DecayRateLimiter::new(100, 10).unwrap();
        rl.try_consume(a(), 100, 0).unwrap(); // a fully loaded
        rl.try_consume(b(), 100, 0).unwrap(); // b fully loaded
        assert_eq!(rl.tracked(), 2);

        // At t=10 both are at pressure 50 → nothing pruned.
        assert_eq!(rl.prune_recovered(10), 0);
        assert_eq!(rl.tracked(), 2);

        // Re-load a() at t=10. By t=10_000 both have fully decayed to 0.
        rl.try_consume(a(), 50, 10).unwrap();
        let pruned = rl.prune_recovered(10_000);
        assert_eq!(pruned, 2);
        assert_eq!(rl.tracked(), 0);
    }

    #[test]
    fn e2e_two_subjects_independent_throttling() {
        let mut rl = DecayRateLimiter::new(10, 5).unwrap();
        // a() bursts 10, gets limited; b() acts freely.
        rl.try_consume(a(), 10, 0).unwrap();
        assert!(matches!(
            rl.try_consume(a(), 1, 0),
            Err(RateError::RateLimited { .. })
        ));
        rl.try_consume(b(), 4, 0).unwrap();
        assert_eq!(rl.available(&b(), 0), 6);
        // a() recovers after a half-life and can act again.
        assert!(rl.would_allow(&a(), 5, 5));
        rl.try_consume(a(), 5, 5).unwrap();
    }

    #[test]
    fn serde_roundtrip_of_a_bucket() {
        let mut rl = DecayRateLimiter::new(100, 10).unwrap();
        rl.try_consume(a(), 40, 0).unwrap();
        let bucket = rl.buckets.get(&a()).unwrap();
        let json = serde_json::to_string(bucket).unwrap();
        let back: DecayBucket = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, bucket);
    }
}
