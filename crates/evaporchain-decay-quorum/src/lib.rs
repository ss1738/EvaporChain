//! Decay-weighted quorum substrate primitive.
//!
//! A membership set where each member carries a voting `weight` that
//! **decays through `evaporchain_types::energy_at_epoch`**. A decision
//! is carried only when the approving members' *live* weight clears a
//! basis-point threshold of the *total live* weight — both measured at
//! decision time, after decay. Influence therefore reflects current
//! engagement: a member who stops refreshing watches their grip fade,
//! while active members' relative weight rises.
//!
//! This is the inverse of `evaporchain-conviction-vote` (which
//! integrates a single voter's conviction *upward* over a lock). Here
//! weight *evaporates*, and the quorum is multi-member. It composes
//! naturally with `evaporchain-decay-credential`: a credential keeps a
//! member eligible, this keeps their vote alive.
//!
//! Scope: this primitive is the decaying-weight aggregation + the
//! live-weight threshold test. It deliberately does **not** bake in
//! caller authorisation for add/remove/refresh — who may manage
//! membership is the embedding contract's policy, layered on top.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Member weight decays via `energy_at_epoch`.** Without refresh a
//!    member's weight is monotonically non-increasing.
//!
//! 2. **The threshold is on LIVE weight, both sides.** Pass iff
//!    `approval_live * 10_000 >= threshold_bps * total_live`. Because
//!    it is a ratio, *uniform* decay never changes an outcome — only
//!    *differential* engagement (some refresh, some don't) shifts who
//!    can carry a decision.
//!
//! 3. **A fully-decayed quorum carries nothing.** If total live weight
//!    is zero, no decision passes (no division-by-zero, no vacuous
//!    pass).
//!
//! ## Module map
//!
//! - [`member`] — [`WeightedMember`] decaying-weight cell + [`QuorumError`].
//! - [`quorum`] — [`DecayQuorum`]: membership + live-weight threshold test.

pub mod member;
pub mod quorum;

pub use member::{QuorumError, WeightedMember};
pub use quorum::DecayQuorum;

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// Doctrine claim asserted as a structural test.
    ///
    /// Press claim: "In a decay-weighted quorum, influence is current
    /// engagement, not history. Three equal members carry a 60%
    /// decision 2-of-3 at the start. Let two go dormant while one keeps
    /// refreshing: the dormant pair's combined weight decays below the
    /// threshold and they can no longer force the decision, while the
    /// active member now carries it alone."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let a = [0xAAu8; 32];
        let b = [0xBBu8; 32];
        let c = [0xCCu8; 32];

        // 60% threshold; three equal members, weight 100, half-life 10.
        let mut q = DecayQuorum::new(6000).unwrap();
        q.add_member(a, 100, 10, 0).unwrap();
        q.add_member(b, 100, 10, 0).unwrap();
        q.add_member(c, 100, 10, 0).unwrap();

        // At the start, any 2-of-3 carries it (200/300 = 66.7%).
        assert!(q.is_passed(&[a, b], 0));

        // a and b go dormant. c refreshes at t=20 back to full weight.
        // At t=20 (two half-lives) a,b have decayed to 25 each; c → 100.
        q.refresh_member(c, 75, 20).unwrap(); // c: live 25 + 75 = 100

        // The dormant pair can no longer carry it: 50 / 150 = 33%.
        assert!(!q.is_passed(&[a, b], 20));
        // The active member now carries it alone: 100 / 150 = 66.7%.
        assert!(q.is_passed(&[c], 20));
    }
}
