//! `Policy` — counter-decay insurance lifecycle.

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
    #[error("claim energy {energy} below floor {floor}")]
    ClaimEnergyBelowFloor { energy: u64, floor: u64 },
    #[error("requested payout {requested} exceeds current cap {cap}")]
    PayoutAboveCap { requested: u128, cap: u128 },
    #[error("arithmetic overflow")]
    Overflow,
    #[error("zero premium_per_epoch")]
    ZeroPremium,
    #[error("zero base_cap")]
    ZeroBaseCap,
    #[error("zero claim_floor")]
    ZeroFloor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyState {
    Open,
    Closed { closed_at_tick: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub id: PolicyId,
    pub holder: [u8; 32],
    /// Premium accumulated (in micros — 1.0 = 10⁶).
    pub premium_paid_micros: u128,
    pub premium_per_epoch_micros: u128,
    /// Cap = base + slope · age. Age is `last_tick - opened_at_tick`.
    pub base_cap_micros: u128,
    pub cap_slope_per_epoch_micros: u128,
    pub paid_out_micros: u128,
    pub claim_floor: u64,
    pub last_tick: u64,
    pub opened_at_tick: u64,
    pub state: PolicyState,
}

impl Policy {
    pub fn open(
        id: PolicyId,
        holder: [u8; 32],
        premium_per_epoch_micros: u128,
        base_cap_micros: u128,
        cap_slope_per_epoch_micros: u128,
        claim_floor: u64,
        opened_at_tick: u64,
    ) -> Result<Self, PolicyError> {
        if premium_per_epoch_micros == 0 {
            return Err(PolicyError::ZeroPremium);
        }
        if base_cap_micros == 0 {
            return Err(PolicyError::ZeroBaseCap);
        }
        if claim_floor == 0 {
            return Err(PolicyError::ZeroFloor);
        }
        Ok(Self {
            id,
            holder,
            premium_paid_micros: 0,
            premium_per_epoch_micros,
            base_cap_micros,
            cap_slope_per_epoch_micros,
            paid_out_micros: 0,
            claim_floor,
            last_tick: opened_at_tick,
            opened_at_tick,
            state: PolicyState::Open,
        })
    }

    /// Age in epochs at `at_tick`.
    pub fn age_at(&self, at_tick: u64) -> u64 {
        at_tick.saturating_sub(self.opened_at_tick)
    }

    /// Current cap at `at_tick` = base + slope · age.
    pub fn cap_at(&self, at_tick: u64) -> u128 {
        let age = self.age_at(at_tick) as u128;
        let growth = age.saturating_mul(self.cap_slope_per_epoch_micros);
        self.base_cap_micros.saturating_add(growth)
    }

    /// Remaining cap = cap_at(now) − paid_out.
    pub fn remaining_cap_at(&self, at_tick: u64) -> u128 {
        self.cap_at(at_tick).saturating_sub(self.paid_out_micros)
    }

    /// Advance the policy by elapsed epochs; charge premium.
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

    /// File a claim. Counter-decay shape: payout is bounded only
    /// by `cap_at(now)` (which GROWS with age) — no claim-staleness
    /// haircut. Claim freshness is still required to admit AT ALL.
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
        // Freshness gate (still required).
        if claim_energy_at_now < self.claim_floor {
            return Err(PolicyError::ClaimEnergyBelowFloor {
                energy: claim_energy_at_now,
                floor: self.claim_floor,
            });
        }
        let cap_now = self.cap_at(now);
        let remaining = cap_now.saturating_sub(self.paid_out_micros);
        if requested_micros > remaining {
            return Err(PolicyError::PayoutAboveCap {
                requested: requested_micros,
                cap: remaining,
            });
        }
        // Counter-decay: NO haircut. Pay the full requested amount
        // (bounded above by remaining cap).
        self.paid_out_micros = self
            .paid_out_micros
            .checked_add(requested_micros)
            .ok_or(PolicyError::Overflow)?;
        self.state = PolicyState::Closed { closed_at_tick: now };
        Ok(requested_micros)
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, PolicyState::Open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(b: u8) -> PolicyId { PolicyId([b; 32]) }
    fn holder() -> [u8; 32] { [0xAA; 32] }

    fn fresh() -> Policy {
        Policy::open(
            pid(1),
            holder(),
            100_000,        // premium_per_epoch = 0.1
            1_000_000,      // base_cap = 1.0
            10_000,         // cap_slope = 0.01 per epoch
            100,            // claim_floor
            0,
        )
        .unwrap()
    }

    // ── construction ─────────────────────────────────────────────

    #[test]
    fn open_initializes_zero_state() {
        let p = fresh();
        assert_eq!(p.premium_paid_micros, 0);
        assert_eq!(p.paid_out_micros, 0);
        assert!(p.is_open());
    }

    #[test]
    fn open_rejects_zero_params() {
        assert_eq!(
            Policy::open(pid(1), holder(), 0, 1000, 0, 1, 0).unwrap_err(),
            PolicyError::ZeroPremium
        );
        assert_eq!(
            Policy::open(pid(1), holder(), 1, 0, 0, 1, 0).unwrap_err(),
            PolicyError::ZeroBaseCap
        );
        assert_eq!(
            Policy::open(pid(1), holder(), 1, 1, 0, 0, 0).unwrap_err(),
            PolicyError::ZeroFloor
        );
    }

    // ── premium accumulation ─────────────────────────────────────

    #[test]
    fn tick_accumulates_premium_linearly() {
        let mut p = fresh();
        p.tick(10).unwrap();
        assert_eq!(p.premium_paid_micros, 10 * 100_000);
        p.tick(50).unwrap();
        assert_eq!(p.premium_paid_micros, 50 * 100_000);
    }

    #[test]
    fn premium_is_monotone() {
        let mut p = fresh();
        let mut last = 0u128;
        for t in 1..=20u64 {
            p.tick(t).unwrap();
            assert!(p.premium_paid_micros >= last);
            last = p.premium_paid_micros;
        }
    }

    // ── cap grows with age ───────────────────────────────────────

    #[test]
    fn cap_at_origin_is_base() {
        let p = fresh();
        assert_eq!(p.cap_at(0), 1_000_000);
    }

    #[test]
    fn cap_grows_linearly_with_age() {
        let p = fresh();
        // age 100 → cap = 1.0 + 100 · 0.01 = 2.0 → 2_000_000.
        assert_eq!(p.cap_at(100), 2_000_000);
        // age 1000 → cap = 1.0 + 10.0 = 11.0 → 11_000_000.
        assert_eq!(p.cap_at(1000), 11_000_000);
    }

    #[test]
    fn cap_is_monotone_non_decreasing() {
        let p = fresh();
        let mut last = 0u128;
        for t in 0..=100u64 {
            let c = p.cap_at(t);
            assert!(c >= last);
            last = c;
        }
    }

    // ── claim freshness gate ─────────────────────────────────────

    #[test]
    fn fresh_claim_pays_full_requested() {
        let mut p = fresh();
        p.tick(100).unwrap();
        let payout = p.file_claim(100, 100, 1000, 1_500_000).unwrap();
        // No haircut — counter-decay shape pays full request.
        assert_eq!(payout, 1_500_000);
    }

    #[test]
    fn claim_below_floor_rejected() {
        let mut p = fresh();
        let err = p.file_claim(100, 100, 50, 1_000_000).unwrap_err();
        assert!(matches!(err, PolicyError::ClaimEnergyBelowFloor { .. }));
    }

    #[test]
    fn claim_above_cap_rejected() {
        let mut p = fresh();
        // At t=0, cap is 1.0. Request 2.0 → exceeds cap.
        let err = p.file_claim(0, 0, 1000, 2_000_000).unwrap_err();
        assert!(matches!(err, PolicyError::PayoutAboveCap { .. }));
    }

    #[test]
    fn cap_growth_unlocks_larger_payouts() {
        // At t=0, max payable is 1.0. At t=100, max payable is 2.0.
        let mut p_young = fresh();
        let mut p_old = fresh();
        p_old.tick(100).unwrap();

        // Young: 1.5M would exceed cap of 1.0M → reject.
        let err = p_young.file_claim(0, 0, 1000, 1_500_000).unwrap_err();
        assert!(matches!(err, PolicyError::PayoutAboveCap { .. }));

        // Old: cap at t=100 is 2.0M; 1.5M is fine.
        let payout = p_old.file_claim(100, 100, 1000, 1_500_000).unwrap();
        assert_eq!(payout, 1_500_000);
    }

    // ── lifecycle ────────────────────────────────────────────────

    #[test]
    fn cannot_claim_after_closed() {
        let mut p = fresh();
        p.file_claim(100, 100, 1000, 500_000).unwrap();
        let err = p.file_claim(150, 150, 1000, 100_000).unwrap_err();
        assert_eq!(err, PolicyError::AlreadyClosed);
    }

    #[test]
    fn cannot_tick_after_closed() {
        let mut p = fresh();
        p.file_claim(100, 100, 1000, 500_000).unwrap();
        let err = p.tick(200).unwrap_err();
        assert_eq!(err, PolicyError::AlreadyClosed);
    }

    #[test]
    fn incident_in_future_rejected() {
        let mut p = fresh();
        let err = p.file_claim(100, 200, 1000, 500_000).unwrap_err();
        assert!(matches!(err, PolicyError::IncidentInFuture { .. }));
    }

    #[test]
    fn non_monotone_tick_rejected() {
        let mut p = fresh();
        p.tick(50).unwrap();
        let err = p.tick(50).unwrap_err();
        assert!(matches!(err, PolicyError::NonMonotoneTick { .. }));
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "SCDI is the first counter-decay insurance:
        // premium grows with policy age (loyalty cost), payout
        // cap ALSO grows with policy age (loyalty reward). Older
        // policies are higher-stake AND higher-payout. Stale
        // claims still rejected — the freshness gate is
        // independent of the cap-growth shape."

        // Two policies, different ages.
        let mut young = fresh();
        let mut old = fresh();
        old.tick(500).unwrap();

        // Both file claims with fresh proof, requesting 1.5M.
        // Young: cap at age 0 = 1.0M → rejects 1.5M.
        let young_err = young.file_claim(0, 0, 1000, 1_500_000).unwrap_err();
        assert!(matches!(young_err, PolicyError::PayoutAboveCap { .. }));

        // Old: cap at age 500 = 1.0 + 500·0.01 = 6.0M → accepts 1.5M.
        let old_payout = old.file_claim(500, 500, 1000, 1_500_000).unwrap();
        assert_eq!(old_payout, 1_500_000);

        // But: stale claim is still rejected, regardless of policy age.
        let mut older = fresh();
        older.tick(1000).unwrap();
        let stale_err = older.file_claim(1000, 999, 50, 100_000).unwrap_err();
        assert!(matches!(stale_err, PolicyError::ClaimEnergyBelowFloor { .. }));
    }

    proptest::proptest! {
        #[test]
        fn property_cap_monotone_non_decreasing_in_time(
            ages in proptest::collection::vec(0u64..1000u64, 1..50),
        ) {
            // For any policy and any sequence of evaluation times,
            // cap_at is monotone non-decreasing.
            let p = fresh();
            let mut sorted = ages;
            sorted.sort();
            let mut last = 0u128;
            for t in sorted {
                let c = p.cap_at(t);
                proptest::prop_assert!(c >= last);
                last = c;
            }
        }

        #[test]
        fn property_premium_strictly_grows_with_each_tick(
            ticks in 1u64..50u64,
        ) {
            let mut p = fresh();
            let mut prev = p.premium_paid_micros;
            for t in 1..=ticks {
                p.tick(t).unwrap();
                proptest::prop_assert!(p.premium_paid_micros > prev);
                prev = p.premium_paid_micros;
            }
        }
    }
}
