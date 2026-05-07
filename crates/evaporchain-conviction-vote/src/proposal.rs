//! `Proposal` — one conviction-vote-target's state machine.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fixed-point unit: 1.0 = 1_000_000.
pub const MICROS: u128 = 1_000_000;

/// Default α (conviction-growth ratio) = 0.9 → 900_000 micros.
/// Standard Commons Stack value.
pub const ALPHA_MICROS_DEFAULT: u64 = 900_000;

/// 32-byte chain-wide handle for a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ProposalId(pub [u8; 32]);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProposalError {
    #[error("alpha must be in (0, 1_000_000) — given {0}")]
    InvalidAlpha(u64),
    #[error("threshold must be > 0")]
    InvalidThreshold,
    #[error("conviction integration overflowed u128")]
    Overflow,
    #[error("non-monotone tick: incoming {incoming} ≤ last {last}")]
    NonMonotoneTick { incoming: u64, last: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalState {
    /// Conviction has not yet crossed threshold.
    Pending,
    /// Conviction crossed threshold at the given tick. One-shot;
    /// subsequent decay does not revert.
    Passed { passed_at_tick: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: ProposalId,
    /// α in micros (e.g., 900_000 = 0.9). Higher α = slower
    /// conviction growth + more weight to historical state.
    pub alpha_micros: u64,
    /// Conviction threshold in micros. When current conviction
    /// crosses this, the proposal transitions to Passed.
    pub threshold_micros: u128,
    /// Current accumulated conviction (in micros).
    pub conviction_micros: u128,
    /// Total stake currently allocated (energy-adjusted; the
    /// caller passes this in via `tick`).
    pub current_stake_micros: u128,
    /// Last tick at which `tick()` was called.
    pub last_tick: u64,
    pub state: ProposalState,
}

impl Proposal {
    pub fn new(
        id: ProposalId,
        alpha_micros: u64,
        threshold_micros: u128,
        genesis_tick: u64,
    ) -> Result<Self, ProposalError> {
        if alpha_micros == 0 || alpha_micros >= MICROS as u64 {
            return Err(ProposalError::InvalidAlpha(alpha_micros));
        }
        if threshold_micros == 0 {
            return Err(ProposalError::InvalidThreshold);
        }
        Ok(Self {
            id,
            alpha_micros,
            threshold_micros,
            conviction_micros: 0,
            current_stake_micros: 0,
            last_tick: genesis_tick,
            state: ProposalState::Pending,
        })
    }

    /// Advance the conviction by one tick using the *energy-decayed*
    /// stake at the current tick.
    ///
    /// Update rule (Commons Stack 2018):
    ///
    ///   c(t+1) = α · c(t) + stake(t)
    ///
    /// Asymptote at constant stake: `stake / (1 - α)`. With α = 0.9,
    /// asymptote = 10·stake.
    ///
    /// In micros:
    ///
    ///   c' = (alpha · c) / MICROS + stake
    ///
    /// If the new conviction crosses the threshold (and the
    /// proposal is currently Pending), transitions to Passed.
    pub fn tick(
        &mut self,
        current_tick: u64,
        decayed_stake_micros: u128,
    ) -> Result<(), ProposalError> {
        if current_tick <= self.last_tick {
            return Err(ProposalError::NonMonotoneTick {
                incoming: current_tick,
                last: self.last_tick,
            });
        }
        let alpha = self.alpha_micros as u128;

        // c' = α · c / MICROS + stake
        let c_alpha_term = self
            .conviction_micros
            .checked_mul(alpha)
            .ok_or(ProposalError::Overflow)?
            / MICROS;
        let new_conviction = c_alpha_term
            .checked_add(decayed_stake_micros)
            .ok_or(ProposalError::Overflow)?;

        self.conviction_micros = new_conviction;
        self.current_stake_micros = decayed_stake_micros;
        self.last_tick = current_tick;

        if matches!(self.state, ProposalState::Pending) && new_conviction >= self.threshold_micros {
            self.state = ProposalState::Passed {
                passed_at_tick: current_tick,
            };
        }
        Ok(())
    }

    /// Asymptotic ceiling at the *current* stake level: `stake / (1 - α)`.
    /// In an idealised continuous limit the conviction approaches this
    /// from below; in our integer one-step recurrence the asymptote is
    /// the same.
    pub fn asymptote(&self) -> u128 {
        let one_minus_alpha = MICROS - self.alpha_micros as u128;
        if one_minus_alpha == 0 {
            return u128::MAX;
        }
        self.current_stake_micros
            .checked_mul(MICROS)
            .map(|n| n / one_minus_alpha)
            .unwrap_or(u128::MAX)
    }

    pub fn is_passed(&self) -> bool {
        matches!(self.state, ProposalState::Passed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(b: u8) -> ProposalId {
        ProposalId([b; 32])
    }

    fn fresh(threshold: u128) -> Proposal {
        Proposal::new(pid(1), ALPHA_MICROS_DEFAULT, threshold, 0).unwrap()
    }

    // ── construction ─────────────────────────────────────────────

    #[test]
    fn rejects_alpha_zero() {
        let err = Proposal::new(pid(1), 0, 1_000_000, 0).unwrap_err();
        assert!(matches!(err, ProposalError::InvalidAlpha(0)));
    }

    #[test]
    fn rejects_alpha_one_or_more() {
        let err = Proposal::new(pid(1), 1_000_000, 1_000_000, 0).unwrap_err();
        assert!(matches!(err, ProposalError::InvalidAlpha(_)));
        let err = Proposal::new(pid(1), 1_500_000, 1_000_000, 0).unwrap_err();
        assert!(matches!(err, ProposalError::InvalidAlpha(_)));
    }

    #[test]
    fn rejects_zero_threshold() {
        let err = Proposal::new(pid(1), ALPHA_MICROS_DEFAULT, 0, 0).unwrap_err();
        assert_eq!(err, ProposalError::InvalidThreshold);
    }

    // ── conviction integration ───────────────────────────────────

    #[test]
    fn empty_stake_keeps_conviction_zero() {
        let mut p = fresh(1_000_000);
        for t in 1..=10 {
            p.tick(t, 0).unwrap();
        }
        assert_eq!(p.conviction_micros, 0);
        assert!(!p.is_passed());
    }

    #[test]
    fn constant_stake_grows_conviction_toward_asymptote() {
        // stake = 1_000_000 (1.0); α = 0.9 → asymptote = 1.0/0.1 = 10.0 = 10_000_000.
        let mut p = fresh(50_000_000); // very high — never passes
        let mut prev = 0u128;
        for t in 1..=100 {
            p.tick(t, 1_000_000).unwrap();
            assert!(
                p.conviction_micros >= prev,
                "conviction should be monotone non-decreasing under constant stake"
            );
            prev = p.conviction_micros;
        }
        // Should be near the asymptote.
        let asymptote = p.asymptote();
        // After 100 ticks at α=0.9, conviction should be > 99% of asymptote.
        assert!(p.conviction_micros > asymptote * 99 / 100);
    }

    #[test]
    fn high_stake_passes_threshold() {
        let mut p = fresh(2_000_000); // threshold = 2.0
        for t in 1..=20 {
            p.tick(t, 1_000_000).unwrap(); // stake = 1.0 each tick
        }
        // After enough ticks at stake=1.0, conviction approaches 10.0 — well above 2.0.
        assert!(p.is_passed());
    }

    // ── decay-native: the load-bearing story ─────────────────────

    #[test]
    fn decaying_stake_caps_conviction() {
        // Stake decays from 1.0 toward 0 over 20 ticks. Conviction
        // peaks then declines — never reaches a high asymptote.
        let mut p = fresh(50_000_000);
        let mut max_conv: u128 = 0;
        for t in 1..=50 {
            // Linear decay: stake = max(0, 1_000_000 - 20_000 * t).
            let stake = 1_000_000u128.saturating_sub(20_000 * t as u128);
            p.tick(t, stake).unwrap();
            max_conv = max_conv.max(p.conviction_micros);
        }
        // After the stake hits zero, the conviction goes down (multiplied by α each tick).
        // So the final conviction must be less than the peak.
        assert!(p.conviction_micros < max_conv);
    }

    #[test]
    fn abandoned_proposal_never_passes() {
        // High threshold; voter deposits once with high stake, then
        // disappears (stake decays to 0). Conviction grows briefly
        // then decays toward zero — never reaches threshold.
        let mut p = fresh(10_000_000);
        // Tick 1: voter alive, full stake.
        p.tick(1, 1_000_000).unwrap();
        // Subsequent ticks: voter gone, stake = 0.
        for t in 2..=200 {
            p.tick(t, 0).unwrap();
        }
        assert!(!p.is_passed());
        // Conviction should be very small after 199 ticks of α=0.9 multiplication.
        // 0.9^199 ≈ 5.6e-10 — essentially zero in micros.
        assert!(
            p.conviction_micros < 100,
            "conviction should decay toward 0, got {}",
            p.conviction_micros
        );
    }

    #[test]
    fn reanchored_stake_keeps_conviction_alive() {
        // Voter re-anchors the stake every tick. Conviction grows
        // to asymptote and crosses threshold.
        let mut p = fresh(5_000_000);
        for t in 1..=100 {
            p.tick(t, 1_000_000).unwrap(); // re-anchored each tick
        }
        assert!(p.is_passed());
    }

    // ── pass-state stickiness ────────────────────────────────────

    #[test]
    fn passed_proposal_stays_passed_after_decay() {
        let mut p = fresh(2_000_000);
        // Pump conviction over threshold.
        for t in 1..=30 {
            p.tick(t, 1_000_000).unwrap();
        }
        assert!(p.is_passed());
        let passed_at = match p.state {
            ProposalState::Passed { passed_at_tick } => passed_at_tick,
            _ => panic!(),
        };
        // Now stake disappears; conviction decays. The pass state
        // sticks (one-shot transition).
        for t in 31..=100 {
            p.tick(t, 0).unwrap();
        }
        assert!(p.is_passed());
        assert_eq!(
            p.state,
            ProposalState::Passed {
                passed_at_tick: passed_at
            }
        );
    }

    // ── monotone tick ─────────────────────────────────────────────

    #[test]
    fn non_monotone_tick_rejected() {
        let mut p = fresh(1_000_000);
        p.tick(5, 100).unwrap();
        let err = p.tick(5, 100).unwrap_err();
        assert_eq!(
            err,
            ProposalError::NonMonotoneTick {
                incoming: 5,
                last: 5
            }
        );
        let err = p.tick(3, 100).unwrap_err();
        assert!(matches!(err, ProposalError::NonMonotoneTick { .. }));
    }

    // ── asymptote ────────────────────────────────────────────────

    #[test]
    fn asymptote_at_constant_stake() {
        let mut p = fresh(50_000_000);
        p.tick(1, 1_000_000).unwrap();
        // α=0.9 → 1/(1-0.9) = 10 → asymptote = 10·stake = 10_000_000.
        assert_eq!(p.asymptote(), 10_000_000);
    }

    // ── doctrine claim ────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Evaporating Conviction Vote is the first
        // governance primitive where conviction has TWO time
        // scales — integration and decay. A voter who deposits
        // and disappears cannot pass a proposal because their
        // generating stake decays before the conviction can
        // build to threshold. **Sustained governance requires
        // sustained engagement.**"
        //
        // Threshold = 5·stake_0; engaged asymptote is 10·stake_0
        // so passes; abandoned voter delivers c_max = stake_0 so
        // never reaches threshold.
        let mut abandoned = Proposal::new(pid(1), ALPHA_MICROS_DEFAULT, 5_000_000, 0).unwrap();
        let mut engaged = Proposal::new(pid(2), ALPHA_MICROS_DEFAULT, 5_000_000, 0).unwrap();

        // Tick 1: both alive at full stake.
        abandoned.tick(1, 1_000_000).unwrap();
        engaged.tick(1, 1_000_000).unwrap();

        // Subsequent ticks: abandoned voter has decayed to zero
        // (modelled here as stake=0 from the chain's perspective);
        // engaged voter keeps re-anchoring at full stake.
        for t in 2..=200 {
            abandoned.tick(t, 0).unwrap();
            engaged.tick(t, 1_000_000).unwrap();
        }
        assert!(!abandoned.is_passed(), "abandoned should not pass");
        assert!(engaged.is_passed(), "engaged should pass");
    }

    proptest::proptest! {
        #[test]
        fn property_zero_stake_only_decreases_conviction(
            initial_conv in 0u128..1_000_000_000u128,
        ) {
            // Set up a proposal with some non-zero conviction by
            // running ticks at a non-zero stake.
            let mut p = Proposal::new(pid(1), ALPHA_MICROS_DEFAULT, u128::MAX / 2, 0).unwrap();
            // Inject the initial conviction by direct construction.
            p.conviction_micros = initial_conv;
            // Tick at zero stake. Conviction must not increase.
            p.tick(1, 0).unwrap();
            proptest::prop_assert!(p.conviction_micros <= initial_conv);
        }

        #[test]
        fn property_constant_stake_is_monotone_non_decreasing(
            stake in 1u128..10_000_000u128,
        ) {
            // Conviction under constant stake must be monotone
            // non-decreasing.
            let mut p = Proposal::new(pid(1), ALPHA_MICROS_DEFAULT, u128::MAX / 2, 0).unwrap();
            let mut prev = 0u128;
            for t in 1..=20u64 {
                p.tick(t, stake).unwrap();
                proptest::prop_assert!(p.conviction_micros >= prev);
                prev = p.conviction_micros;
            }
        }
    }
}
