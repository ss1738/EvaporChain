//! [`ReputationLedger`] — per-subject keyed reputation with a
//! leaderboard and dormant-entry pruning.

use std::collections::HashMap;

use crate::reputation::{RepError, Reputation};

/// A ledger of [`Reputation`] scores keyed by subject, sharing one
/// half-life. Entries are created lazily on first record.
#[derive(Debug, Clone)]
pub struct ReputationLedger {
    half_life: u64,
    scores: HashMap<[u8; 32], Reputation>,
}

impl ReputationLedger {
    pub fn new(half_life: u64) -> Result<Self, RepError> {
        if half_life == 0 {
            return Err(RepError::ZeroHalfLife);
        }
        Ok(Self {
            half_life,
            scores: HashMap::new(),
        })
    }

    fn entry_mut(&mut self, subject: [u8; 32], now: u64) -> &mut Reputation {
        let hl = self.half_life;
        self.scores
            .entry(subject)
            .or_insert_with(|| Reputation::new(hl, now).expect("half-life validated in new"))
    }

    /// Credit `amount` of merit to `subject` at `now`.
    pub fn record_merit(
        &mut self,
        subject: [u8; 32],
        amount: u64,
        now: u64,
    ) -> Result<(), RepError> {
        self.entry_mut(subject, now).record_merit(amount, now)
    }

    /// Charge `amount` of demerit to `subject` at `now`.
    pub fn record_demerit(
        &mut self,
        subject: [u8; 32],
        amount: u64,
        now: u64,
    ) -> Result<(), RepError> {
        self.entry_mut(subject, now).record_demerit(amount, now)
    }

    /// Net signed reputation for `subject` at `now` (0 if never seen).
    pub fn net(&self, subject: &[u8; 32], now: u64) -> i128 {
        self.scores.get(subject).map(|r| r.net_at(now)).unwrap_or(0)
    }

    pub fn merit(&self, subject: &[u8; 32], now: u64) -> u64 {
        self.scores.get(subject).map(|r| r.merit_at(now)).unwrap_or(0)
    }

    pub fn demerit(&self, subject: &[u8; 32], now: u64) -> u64 {
        self.scores
            .get(subject)
            .map(|r| r.demerit_at(now))
            .unwrap_or(0)
    }

    pub fn is_positive(&self, subject: &[u8; 32], now: u64) -> bool {
        self.net(subject, now) > 0
    }

    /// All subjects ranked by net reputation at `now`, highest first;
    /// ties broken by subject id for determinism.
    pub fn leaderboard(&self, now: u64) -> Vec<([u8; 32], i128)> {
        let mut v: Vec<([u8; 32], i128)> = self
            .scores
            .keys()
            .map(|k| (*k, self.net(k, now)))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// Drop entries whose merit and demerit have both fully decayed to
    /// zero at `now` — dormant standing carries no information.
    /// Returns the number pruned.
    pub fn prune_dormant(&mut self, now: u64) -> usize {
        let before = self.scores.len();
        self.scores.retain(|_, r| !r.is_dormant_at(now));
        before - self.scores.len()
    }

    pub fn half_life(&self) -> u64 {
        self.half_life
    }

    pub fn tracked(&self) -> usize {
        self.scores.len()
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
    fn c() -> [u8; 32] {
        [0xCC; 32]
    }

    #[test]
    fn new_rejects_zero_half_life() {
        assert_eq!(ReputationLedger::new(0).unwrap_err(), RepError::ZeroHalfLife);
    }

    #[test]
    fn unseen_subject_is_neutral() {
        let l = ReputationLedger::new(10).unwrap();
        assert_eq!(l.net(&a(), 0), 0);
        assert_eq!(l.merit(&a(), 0), 0);
        assert!(!l.is_positive(&a(), 0));
        assert_eq!(l.tracked(), 0);
    }

    #[test]
    fn record_and_net() {
        let mut l = ReputationLedger::new(10).unwrap();
        l.record_merit(a(), 500, 0).unwrap();
        l.record_demerit(a(), 200, 0).unwrap();
        assert_eq!(l.net(&a(), 0), 300);
        assert_eq!(l.tracked(), 1);
    }

    #[test]
    fn subjects_are_independent() {
        let mut l = ReputationLedger::new(10).unwrap();
        l.record_merit(a(), 100, 0).unwrap();
        l.record_demerit(b(), 100, 0).unwrap();
        assert_eq!(l.net(&a(), 0), 100);
        assert_eq!(l.net(&b(), 0), -100);
    }

    #[test]
    fn leaderboard_ranks_by_net_desc() {
        let mut l = ReputationLedger::new(10).unwrap();
        l.record_merit(a(), 100, 0).unwrap();
        l.record_merit(b(), 300, 0).unwrap();
        l.record_demerit(c(), 50, 0).unwrap();
        let board = l.leaderboard(0);
        assert_eq!(board[0], (b(), 300));
        assert_eq!(board[1], (a(), 100));
        assert_eq!(board[2], (c(), -50));
    }

    #[test]
    fn prune_drops_dormant_keeps_active() {
        let mut l = ReputationLedger::new(10).unwrap();
        l.record_merit(a(), 100, 0).unwrap();
        l.record_merit(b(), 100, 0).unwrap();
        // At t=10 both still have weight → nothing pruned.
        assert_eq!(l.prune_dormant(10), 0);
        // Refresh a() at t=10; by t=100_000 both fully decay.
        l.record_merit(a(), 100, 10).unwrap();
        assert_eq!(l.prune_dormant(100_000), 2);
        assert_eq!(l.tracked(), 0);
    }

    #[test]
    fn e2e_recency_across_subjects() {
        let mut l = ReputationLedger::new(10).unwrap();
        // c earns early, b earns the same amount much later.
        l.record_merit(c(), 100, 0).unwrap();
        l.record_merit(b(), 100, 20).unwrap();
        // At t=20 the recent earner outranks the stale one.
        let board = l.leaderboard(20);
        assert_eq!(board[0].0, b());
        assert!(board[0].1 > board[1].1);
    }

    proptest::proptest! {
        /// net == merit - demerit for any record sequence, and merit /
        /// demerit are non-increasing between records.
        #[test]
        fn property_net_is_merit_minus_demerit(
            m in 0u64..1_000_000u64,
            d in 0u64..1_000_000u64,
            half_life in 1u64..1_000u64,
            t_rec in 0u64..50_000u64,
            dt in 0u64..50_000u64,
        ) {
            let mut l = ReputationLedger::new(half_life).unwrap();
            l.record_merit(a(), m, t_rec).unwrap();
            l.record_demerit(a(), d, t_rec).unwrap();
            let now = t_rec + dt;
            let net = l.net(&a(), now);
            let merit = l.merit(&a(), now) as i128;
            let demerit = l.demerit(&a(), now) as i128;
            proptest::prop_assert_eq!(net, merit - demerit);
            // decay is non-increasing on each side
            proptest::prop_assert!(l.merit(&a(), now) <= l.merit(&a(), t_rec));
            proptest::prop_assert!(l.demerit(&a(), now) <= l.demerit(&a(), t_rec));
        }
    }
}
