//! [`DecayQuorum`] — membership set + live-weight threshold test.

use std::collections::{HashMap, HashSet};

use crate::member::{QuorumError, WeightedMember};

/// A decay-weighted quorum. Members' weights decay; a decision passes
/// when approver live-weight clears `threshold_bps` of total live-weight.
#[derive(Debug, Clone)]
pub struct DecayQuorum {
    members: HashMap<[u8; 32], WeightedMember>,
    threshold_bps: u32,
}

impl DecayQuorum {
    /// Create an empty quorum with a basis-point pass threshold
    /// (1..=10_000, i.e. 0.01%..=100%).
    pub fn new(threshold_bps: u32) -> Result<Self, QuorumError> {
        if threshold_bps == 0 || threshold_bps > 10_000 {
            return Err(QuorumError::ThresholdOutOfRange { bps: threshold_bps });
        }
        Ok(Self {
            members: HashMap::new(),
            threshold_bps,
        })
    }

    /// Add a member with an initial weight + half-life.
    pub fn add_member(
        &mut self,
        addr: [u8; 32],
        weight: u64,
        half_life: u64,
        now: u64,
    ) -> Result<(), QuorumError> {
        if self.members.contains_key(&addr) {
            return Err(QuorumError::DuplicateMember);
        }
        let member = WeightedMember::new(weight, half_life, now)?;
        self.members.insert(addr, member);
        Ok(())
    }

    /// Remove a member.
    pub fn remove_member(&mut self, addr: &[u8; 32]) -> Result<(), QuorumError> {
        self.members
            .remove(addr)
            .map(|_| ())
            .ok_or(QuorumError::MemberNotFound)
    }

    /// Top up a member's decayed weight and reset their decay clock.
    pub fn refresh_member(
        &mut self,
        addr: [u8; 32],
        top_up: u64,
        now: u64,
    ) -> Result<(), QuorumError> {
        self.members
            .get_mut(&addr)
            .ok_or(QuorumError::MemberNotFound)?
            .refresh(top_up, now)
    }

    /// A member's live weight at `now` (0 if not a member).
    pub fn member_weight(&self, addr: &[u8; 32], now: u64) -> u64 {
        self.members
            .get(addr)
            .map(|m| m.current_weight(now))
            .unwrap_or(0)
    }

    /// Total live weight across all members at `now`.
    pub fn total_weight(&self, now: u64) -> u64 {
        self.members
            .values()
            .map(|m| m.current_weight(now))
            .fold(0u64, u64::saturating_add)
    }

    /// Combined live weight of the distinct `approvers` that are
    /// members at `now`. Non-members and duplicates are ignored.
    pub fn approval_weight(&self, approvers: &[[u8; 32]], now: u64) -> u64 {
        let unique: HashSet<&[u8; 32]> = approvers.iter().collect();
        unique
            .iter()
            .filter_map(|a| self.members.get(*a))
            .map(|m| m.current_weight(now))
            .fold(0u64, u64::saturating_add)
    }

    /// Whether `approvers` carry a decision at `now`:
    /// `approval_live * 10_000 >= threshold_bps * total_live`. A quorum
    /// whose total live weight is zero carries nothing.
    pub fn is_passed(&self, approvers: &[[u8; 32]], now: u64) -> bool {
        let total = self.total_weight(now);
        if total == 0 {
            return false;
        }
        let approval = self.approval_weight(approvers, now);
        (approval as u128) * 10_000u128 >= (self.threshold_bps as u128) * (total as u128)
    }

    pub fn threshold_bps(&self) -> u32 {
        self.threshold_bps
    }

    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    pub fn is_member(&self, addr: &[u8; 32]) -> bool {
        self.members.contains_key(addr)
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
    fn d() -> [u8; 32] {
        [0xDD; 32]
    }

    fn three_equal() -> DecayQuorum {
        let mut q = DecayQuorum::new(6000).unwrap(); // 60%
        q.add_member(a(), 100, 10, 0).unwrap();
        q.add_member(b(), 100, 10, 0).unwrap();
        q.add_member(c(), 100, 10, 0).unwrap();
        q
    }

    // ── construction / membership ────────────────────────────────

    #[test]
    fn new_rejects_bad_threshold() {
        assert!(matches!(
            DecayQuorum::new(0),
            Err(QuorumError::ThresholdOutOfRange { .. })
        ));
        assert!(matches!(
            DecayQuorum::new(10_001),
            Err(QuorumError::ThresholdOutOfRange { .. })
        ));
        assert!(DecayQuorum::new(10_000).is_ok());
    }

    #[test]
    fn add_member_rejects_duplicate_and_bad_params() {
        let mut q = DecayQuorum::new(5000).unwrap();
        q.add_member(a(), 100, 10, 0).unwrap();
        assert_eq!(q.add_member(a(), 1, 1, 0).unwrap_err(), QuorumError::DuplicateMember);
        assert_eq!(q.add_member(b(), 0, 10, 0).unwrap_err(), QuorumError::ZeroWeight);
        assert_eq!(q.add_member(b(), 1, 0, 0).unwrap_err(), QuorumError::ZeroHalfLife);
        assert_eq!(q.member_count(), 1);
    }

    #[test]
    fn remove_and_refresh_missing_member() {
        let mut q = DecayQuorum::new(5000).unwrap();
        assert_eq!(q.remove_member(&a()), Err(QuorumError::MemberNotFound));
        assert_eq!(
            q.refresh_member(a(), 1, 0),
            Err(QuorumError::MemberNotFound)
        );
    }

    // ── weights ──────────────────────────────────────────────────

    #[test]
    fn total_and_member_weight() {
        let q = three_equal();
        assert_eq!(q.total_weight(0), 300);
        assert_eq!(q.member_weight(&a(), 0), 100);
        assert_eq!(q.member_weight(&d(), 0), 0); // non-member
        assert_eq!(q.total_weight(10), 150); // each halves
    }

    #[test]
    fn approval_weight_dedups_and_ignores_non_members() {
        let q = three_equal();
        // duplicate a() counted once; d() is not a member.
        assert_eq!(q.approval_weight(&[a(), a(), d()], 0), 100);
        assert_eq!(q.approval_weight(&[a(), b()], 0), 200);
    }

    // ── threshold ────────────────────────────────────────────────

    #[test]
    fn two_of_three_passes_one_of_three_fails() {
        let q = three_equal();
        assert!(q.is_passed(&[a(), b()], 0)); // 200/300 = 66.7% ≥ 60%
        assert!(!q.is_passed(&[a()], 0)); // 100/300 = 33% < 60%
    }

    #[test]
    fn uniform_decay_preserves_the_outcome() {
        let q = three_equal();
        // No refreshes: all members decay equally, so the ratio holds.
        assert!(q.is_passed(&[a(), b()], 0));
        assert!(q.is_passed(&[a(), b()], 50));
        assert!(!q.is_passed(&[a()], 0));
        assert!(!q.is_passed(&[a()], 50));
    }

    #[test]
    fn differential_engagement_shifts_the_outcome() {
        let mut q = three_equal();
        // a,b dormant; c refreshes to full at t=20 (live 25 + 75).
        q.refresh_member(c(), 75, 20).unwrap();
        assert!(!q.is_passed(&[a(), b()], 20)); // 50/150 = 33%
        assert!(q.is_passed(&[c()], 20)); // 100/150 = 66.7%
    }

    #[test]
    fn fully_decayed_quorum_carries_nothing() {
        let q = three_equal();
        // After many half-lives all weights are 0 → total 0 → no pass.
        assert_eq!(q.total_weight(100_000), 0);
        assert!(!q.is_passed(&[a(), b(), c()], 100_000));
    }

    #[test]
    fn removing_a_member_lowers_the_bar() {
        let mut q = three_equal();
        // a alone is 100/300 = 33% → fails.
        assert!(!q.is_passed(&[a()], 0));
        // Remove c: now a is 100/200 = 50% → still < 60%.
        q.remove_member(&c()).unwrap();
        assert!(!q.is_passed(&[a()], 0));
        // Remove b too: a is 100/100 = 100% → passes.
        q.remove_member(&b()).unwrap();
        assert!(q.is_passed(&[a()], 0));
    }

    #[test]
    fn e2e_living_committee() {
        let mut q = DecayQuorum::new(5000).unwrap(); // 50%
        q.add_member(a(), 200, 20, 0).unwrap();
        q.add_member(b(), 100, 20, 0).unwrap();
        // a alone = 200/300 = 66% ≥ 50% → passes.
        assert!(q.is_passed(&[a()], 0));
        // b refreshes hard at t=20 to overtake; a goes dormant.
        // t=20: a live = 100, b live = 50; b refresh +150 → 200.
        q.refresh_member(b(), 150, 20).unwrap();
        // total = 100 + 200 = 300; b alone = 200/300 = 66% ≥ 50%.
        assert!(q.is_passed(&[b()], 20));
        // a alone now = 100/300 = 33% < 50% → fails.
        assert!(!q.is_passed(&[a()], 20));
    }

    #[test]
    fn serde_roundtrip_of_a_member() {
        let q = three_equal();
        // WeightedMember is serializable (the public cell type).
        let m = WeightedMember::new(123, 7, 3).unwrap();
        let json = serde_json::to_string(&m).unwrap();
        let back: WeightedMember = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
        let _ = q; // silence unused in this focused test
    }

    proptest::proptest! {
        /// Approval weight never exceeds total weight, and a pass
        /// verdict always implies the threshold inequality holds.
        #[test]
        fn property_approval_le_total_and_pass_consistent(
            wa in 1u64..1_000_000u64,
            wb in 1u64..1_000_000u64,
            wc in 1u64..1_000_000u64,
            half_life in 1u64..1_000u64,
            bps in 1u32..=10_000u32,
            now in 0u64..100_000u64,
        ) {
            let mut q = DecayQuorum::new(bps).unwrap();
            q.add_member(a(), wa, half_life, 0).unwrap();
            q.add_member(b(), wb, half_life, 0).unwrap();
            q.add_member(c(), wc, half_life, 0).unwrap();

            let total = q.total_weight(now);
            let approval = q.approval_weight(&[a(), b()], now);
            proptest::prop_assert!(approval <= total);

            if q.is_passed(&[a(), b()], now) {
                proptest::prop_assert!(total > 0);
                proptest::prop_assert!(
                    (approval as u128) * 10_000u128 >= (bps as u128) * (total as u128)
                );
            }
        }
    }
}
