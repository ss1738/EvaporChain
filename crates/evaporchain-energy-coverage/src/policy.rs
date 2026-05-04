//! `Policy` — one coverage policy + claim lifecycle.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct PolicyId(pub [u8; 32]);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("non-monotone tick: incoming {incoming} ≤ last {last}")]
    NonMonotoneTick { incoming: u64, last: u64 },
    #[error("policy already closed")]
    AlreadyClosed,
    #[error("incident_at {incident} > now {now}")]
    IncidentInFuture { incident: u64, now: u64 },
    #[error("claim energy {energy} below floor {floor} — claim too stale")]
    ClaimEnergyBelowFloor { energy: u64, floor: u64 },
    #[error("requested payout {requested} exceeds remaining coverage {remaining}")]
    PayoutExceedsRemaining { requested: u128, remaining: u128 },
    #[error("arithmetic overflow")]
    Overflow,
    #[error("zero premium_per_epoch")]
    ZeroPremium,
    #[error("zero coverage_max")]
    ZeroCoverage,
    #[error("zero claim_floor")]
    ZeroClaimFloor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyState {
    Open,
    /// Closed by claim or by expiry / cancellation.
    Closed { closed_at_tick: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub id: PolicyId,
    pub holder: [u8; 32],
    /// Premium accumulated (in micros — 1.0 = 10⁶).
    pub premium_paid_micros: u128,
    pub premium_per_epoch_micros: u128,
    /// Total cap on payouts.
    pub coverage_max_micros: u128,
    /// Cumulative paid out across claims.
    pub paid_out_micros: u128,
    /// Minimum claim energy at file-time (post-decay) to admit a claim.
    pub claim_floor: u64,
    /// Last tick the chain advanced this policy.
    pub last_tick: u64,
    /// Tick at which the policy was opened.
    pub opened_at_tick: u64,
    pub state: PolicyState,
}

impl Policy {
    pub fn open(
        id: PolicyId,
        holder: [u8; 32],
        premium_per_epoch_micros: u128,
        coverage_max_micros: u128,
        claim_floor: u64,
        opened_at_tick: u64,
    ) -> Result<Self, PolicyError> {
        if premium_per_epoch_micros == 0 {
            return Err(PolicyError::ZeroPremium);
        }
        if coverage_max_micros == 0 {
            return Err(PolicyError::ZeroCoverage);
        }
        if claim_floor == 0 {
            return Err(PolicyError::ZeroClaimFloor);
        }
        Ok(Self {
            id,
            holder,
            premium_paid_micros: 0,
            premium_per_epoch_micros,
            coverage_max_micros,
            paid_out_micros: 0,
            claim_floor,
            last_tick: opened_at_tick,
            opened_at_tick,
            state: PolicyState::Open,
        })
    }

    pub fn remaining_coverage_micros(&self) -> u128 {
        self.coverage_max_micros.saturating_sub(self.paid_out_micros)
    }

    /// Advance the policy by one or more ticks. Charges premium
    /// `(now - last_tick) * premium_per_epoch`.
    pub fn tick(&mut self, now: u64) -> Result<(), PolicyError> {
        if matches!(self.state, PolicyState::Closed { .. }) {
            return Err(PolicyError::AlreadyClosed);
        }
        if now <= self.last_tick {
            return Err(PolicyError::NonMonotoneTick {
                incoming: now,
                last: self.last_tick,
            });
        }
        let elapsed = (now - self.last_tick) as u128;
        let added = elapsed
            .checked_mul(self.premium_per_epoch_micros)
            .ok_or(PolicyError::Overflow)?;
        self.premium_paid_micros = self
            .premium_paid_micros
            .checked_add(added)
            .ok_or(PolicyError::Overflow)?;
        self.last_tick = now;
        Ok(())
    }

    /// File a claim. The caller passes `claim_energy_at_now` —
    /// already energy-adjusted by the chain's decay model
    /// (initial_energy at incident, decayed by elapsed = now -
    /// incident_at). The policy gate verifies floor + payout
    /// computation.
    ///
    /// `requested_micros` is the claimant's requested payout.
    /// Pro-rated by `claim_energy_at_now / claim_floor` and
    /// capped by remaining coverage.
    ///
    /// On success: paid_out increments, state transitions to
    /// `Closed { closed_at_tick: now }`.
    pub fn file_claim(
        &mut self,
        now: u64,
        incident_at: u64,
        claim_energy_at_now: u64,
        requested_micros: u128,
    ) -> Result<u128, PolicyError> {
        if matches!(self.state, PolicyState::Closed { .. }) {
            return Err(PolicyError::AlreadyClosed);
        }
        if incident_at > now {
            return Err(PolicyError::IncidentInFuture {
                incident: incident_at,
                now,
            });
        }
        if claim_energy_at_now < self.claim_floor {
            return Err(PolicyError::ClaimEnergyBelowFloor {
                energy: claim_energy_at_now,
                floor: self.claim_floor,
            });
        }

        let remaining = self.remaining_coverage_micros();
        if requested_micros > remaining {
            return Err(PolicyError::PayoutExceedsRemaining {
                requested: requested_micros,
                remaining,
            });
        }

        // Pro-rate: `payout = requested · claim_energy / energy_ceil`
        // where `energy_ceil = max(claim_floor, claim_energy_at_now)`.
        // We pick `energy_ceil` such that energy at floor pays the
        // floor-pro-rated share. Concretely, we pro-rate against a
        // notional "full freshness" of `2 · claim_floor`: claims at
        // exactly floor pay 50%; claims at ≥ 2·floor pay 100%.
        let full_freshness: u128 = (self.claim_floor as u128) * 2;
        let cur_energy = claim_energy_at_now as u128;
        let scale = if cur_energy >= full_freshness {
            full_freshness
        } else {
            cur_energy
        };
        let payout = requested_micros
            .checked_mul(scale)
            .ok_or(PolicyError::Overflow)?
            / full_freshness;

        self.paid_out_micros = self
            .paid_out_micros
            .checked_add(payout)
            .ok_or(PolicyError::Overflow)?;
        self.state = PolicyState::Closed { closed_at_tick: now };
        Ok(payout)
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, PolicyState::Open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(b: u8) -> PolicyId { PolicyId([b; 32]) }
    fn holder(b: u8) -> [u8; 32] { [b; 32] }

    fn fresh(claim_floor: u64) -> Policy {
        Policy::open(
            pid(1),
            holder(0xAA),
            100_000,        // premium_per_epoch = 0.1
            10_000_000,     // coverage_max = 10.0
            claim_floor,
            0,
        )
        .unwrap()
    }

    // ── construction ─────────────────────────────────────────────

    #[test]
    fn open_initializes_zero_paid() {
        let p = fresh(100);
        assert_eq!(p.premium_paid_micros, 0);
        assert_eq!(p.paid_out_micros, 0);
        assert_eq!(p.remaining_coverage_micros(), 10_000_000);
        assert!(p.is_open());
    }

    #[test]
    fn open_rejects_zero_params() {
        assert_eq!(Policy::open(pid(1), holder(0), 0, 1000, 100, 0).unwrap_err(), PolicyError::ZeroPremium);
        assert_eq!(Policy::open(pid(1), holder(0), 100, 0, 100, 0).unwrap_err(), PolicyError::ZeroCoverage);
        assert_eq!(Policy::open(pid(1), holder(0), 100, 1000, 0, 0).unwrap_err(), PolicyError::ZeroClaimFloor);
    }

    // ── premium accumulation ─────────────────────────────────────

    #[test]
    fn tick_accumulates_premium() {
        let mut p = fresh(100);
        p.tick(10).unwrap();
        assert_eq!(p.premium_paid_micros, 10 * 100_000);
        p.tick(50).unwrap();
        assert_eq!(p.premium_paid_micros, 50 * 100_000);
    }

    #[test]
    fn premium_is_monotone_non_decreasing() {
        let mut p = fresh(100);
        let mut last = 0u128;
        for t in 1..=20 {
            p.tick(t).unwrap();
            assert!(p.premium_paid_micros >= last);
            last = p.premium_paid_micros;
        }
    }

    #[test]
    fn non_monotone_tick_rejected() {
        let mut p = fresh(100);
        p.tick(10).unwrap();
        let err = p.tick(10).unwrap_err();
        assert!(matches!(err, PolicyError::NonMonotoneTick { .. }));
        let err = p.tick(5).unwrap_err();
        assert!(matches!(err, PolicyError::NonMonotoneTick { .. }));
    }

    // ── claim filing ─────────────────────────────────────────────

    #[test]
    fn fresh_claim_pays_full() {
        // Claim energy = 2·floor (full freshness) → pays 100%.
        let mut p = fresh(100);
        let payout = p.file_claim(50, 49, 200, 1_000_000).unwrap();
        assert_eq!(payout, 1_000_000);
        assert_eq!(p.paid_out_micros, 1_000_000);
        assert!(matches!(p.state, PolicyState::Closed { .. }));
    }

    #[test]
    fn at_floor_claim_pays_half() {
        // Claim energy exactly at floor → pays 50% (proportional).
        let mut p = fresh(100);
        let payout = p.file_claim(50, 49, 100, 1_000_000).unwrap();
        // scale = min(100, 200) = 100. payout = 1M * 100 / 200 = 500_000.
        assert_eq!(payout, 500_000);
    }

    #[test]
    fn below_floor_claim_rejected() {
        let mut p = fresh(100);
        let err = p.file_claim(50, 49, 50, 1_000_000).unwrap_err();
        assert!(matches!(err, PolicyError::ClaimEnergyBelowFloor { .. }));
        // State is unchanged.
        assert!(p.is_open());
        assert_eq!(p.paid_out_micros, 0);
    }

    #[test]
    fn claim_above_remaining_rejected() {
        let mut p = fresh(100);
        let err = p
            .file_claim(50, 49, 200, p.coverage_max_micros + 1)
            .unwrap_err();
        assert!(matches!(err, PolicyError::PayoutExceedsRemaining { .. }));
    }

    #[test]
    fn incident_in_future_rejected() {
        let mut p = fresh(100);
        let err = p.file_claim(10, 50, 200, 1_000_000).unwrap_err();
        assert!(matches!(err, PolicyError::IncidentInFuture { .. }));
    }

    #[test]
    fn cannot_claim_after_closed() {
        let mut p = fresh(100);
        p.file_claim(50, 49, 200, 1_000_000).unwrap();
        let err = p.file_claim(60, 59, 200, 100_000).unwrap_err();
        assert_eq!(err, PolicyError::AlreadyClosed);
    }

    #[test]
    fn cannot_tick_after_closed() {
        let mut p = fresh(100);
        p.file_claim(50, 49, 200, 1_000_000).unwrap();
        let err = p.tick(60).unwrap_err();
        assert_eq!(err, PolicyError::AlreadyClosed);
    }

    // ── pro-rating math ───────────────────────────────────────────

    #[test]
    fn pro_rating_is_linear_between_floor_and_full() {
        // At energy = 1.5·floor = 150, scale = 150 → pays 75%.
        let mut p = fresh(100);
        let payout = p.file_claim(50, 49, 150, 1_000_000).unwrap();
        assert_eq!(payout, 750_000);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Energy-Clocked Coverage is the first decay-
        // native insurance primitive: premium accumulates with
        // policy age (cumulative cost grows), claims gate on
        // freshness (stale claims are unprovable). The protocol
        // pays full freshness 100%, near-floor 50%, below-floor
        // 0%."
        let mut p = fresh(100);

        // 50 epochs of premium accumulation.
        p.tick(50).unwrap();
        assert_eq!(p.premium_paid_micros, 50 * 100_000);

        // Same incident, three claimants with different freshness:
        let mut p_fresh = fresh(100);
        p_fresh.tick(50).unwrap();
        let pay_fresh = p_fresh.file_claim(50, 50, 200, 1_000_000).unwrap();

        let mut p_partial = fresh(100);
        p_partial.tick(50).unwrap();
        let pay_partial = p_partial.file_claim(50, 49, 100, 1_000_000).unwrap();

        let mut p_stale = fresh(100);
        p_stale.tick(50).unwrap();
        let stale_err = p_stale.file_claim(50, 30, 50, 1_000_000).unwrap_err();

        assert_eq!(pay_fresh, 1_000_000);   // full
        assert_eq!(pay_partial, 500_000);   // half
        assert!(matches!(stale_err, PolicyError::ClaimEnergyBelowFloor { .. })); // none
    }

    proptest::proptest! {
        #[test]
        fn property_premium_monotone_under_arbitrary_ticks(
            n in 1usize..50usize,
        ) {
            let mut p = fresh(100);
            let mut last = 0u128;
            for t in 1..=(n as u64) {
                p.tick(t).unwrap();
                proptest::prop_assert!(p.premium_paid_micros >= last);
                last = p.premium_paid_micros;
            }
        }

        #[test]
        fn property_payout_never_exceeds_request(
            energy in 100u64..2000u64,
            requested in 1u128..1_000_000u128,
        ) {
            let mut p = fresh(100);
            let payout = p.file_claim(50, 49, energy, requested).unwrap();
            proptest::prop_assert!(payout <= requested);
        }

        #[test]
        fn property_payout_zero_below_floor_is_an_error(
            energy in 0u64..99u64, // strictly below floor 100
        ) {
            let mut p = fresh(100);
            let result = p.file_claim(50, 49, energy, 1000);
            proptest::prop_assert!(result.is_err());
        }
    }
}
