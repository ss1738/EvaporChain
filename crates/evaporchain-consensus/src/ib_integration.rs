//! Information-Bottleneck Validator integration — wires
//! `evaporchain-ib-validators` into the consensus layer.
//!
//! Per INVENTION_STACK.md §A4.3.1, each validator applies the IB principle
//! (Tishby-Pereira-Bialek 1999) to its vote decision:
//!
//! > Commit iff the KL divergence between the local state signature and the
//! > prior (network-wide) state signature exceeds λ_mb (the IB β parameter).
//!
//! This replaces the classical "vote for any valid block" rule with an
//! energy-information criterion: a validator only commits when its local view
//! carries strictly more relevant information than the shared prior.
//!
//! # Integration points
//!
//! `build_validator_signature(stakes)` — builds the local state signature
//! from the current validator stake distribution.  Called before prevote.
//!
//! `compute_ib_vote(local, prior, params)` — returns `IbVote::Commit` or
//! `IbVote::Abstain`.  Abstain is advisory in the substrate; the
//! existing Tendermint vote path is not hard-gated yet.

use evaporchain_ib_validators::{
    ib::{ib_vote, IbParams, IbVote},
    signature::StateSignature,
};
use tracing::debug;

/// Default IB λ_mb — threshold KL divergence in millibits above which a
/// validator Commits.  0 = always Commit (bootstrap / governance default).
pub const DEFAULT_LAMBDA_MB: u64 = 0;

/// Build a `StateSignature` from a slice of validator stake values.
///
/// The stake slice is treated as the energy distribution X.  The scale
/// parameter is set to `max_stake.max(1)` so the top bin captures the
/// highest-stake validator and all bins are proportionally filled.
pub fn build_validator_signature(stakes: &[u64]) -> StateSignature {
    let scale = stakes.iter().copied().max().unwrap_or(1).max(1);
    StateSignature::from_energies(stakes, scale)
}

/// Compute the IB vote given the local and prior state signatures.
///
/// Returns `IbVote::Commit` or `IbVote::Abstain`.  With `DEFAULT_LAMBDA_MB
/// = 0` this always returns Commit (no information threshold required).
pub fn compute_ib_vote(local: &StateSignature, prior: &StateSignature, lambda_mb: u64) -> IbVote {
    let params = IbParams { lambda_mb };
    let vote = ib_vote(local, prior, &params);
    debug!(
        kl_approx = local.kl_millibits(prior),
        lambda_mb,
        vote = ?vote,
        "IB vote computed"
    );
    vote
}

/// Build local + prior signatures from two stake distributions and return
/// the IB vote.  Convenience wrapper for the consensus tick.
pub fn ib_vote_from_stakes(local_stakes: &[u64], prior_stakes: &[u64], lambda_mb: u64) -> IbVote {
    let local = build_validator_signature(local_stakes);
    let prior = build_validator_signature(prior_stakes);
    compute_ib_vote(&local, &prior, lambda_mb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_ib_validators::signature::N_BINS;

    fn uniform_stakes(n: usize, val: u64) -> Vec<u64> {
        vec![val; n]
    }

    #[test]
    fn zero_lambda_always_commits() {
        let local = uniform_stakes(4, 100);
        let prior = uniform_stakes(4, 200);
        let vote = ib_vote_from_stakes(&local, &prior, 0);
        assert_eq!(vote, IbVote::Commit);
    }

    #[test]
    fn identical_distributions_abstains_at_high_lambda() {
        // KL(X||X) ≈ 0 — below any positive threshold → Abstain.
        let stakes = uniform_stakes(8, 500);
        let vote = ib_vote_from_stakes(&stakes, &stakes, 1_000_000);
        assert_eq!(vote, IbVote::Abstain);
    }

    #[test]
    fn divergent_distributions_commit_at_moderate_lambda() {
        // Truly divergent distributions: every value lives in a different
        // bin. The earlier version of this test used [1000, 1, 1, 1, ..]
        // vs [1, 1, 1, .., 1000] — those bin identically (7 ones at bin 0
        // + one entry at bin 15) because StateSignature is order-agnostic.
        let local = vec![100, 100, 100, 100, 100, 100, 100, 100];
        let prior = vec![900, 900, 900, 900, 900, 900, 900, 900];
        // lambda_mb = 1 — tiny threshold should still commit.
        let vote = ib_vote_from_stakes(&local, &prior, 1);
        assert_eq!(vote, IbVote::Commit);
    }

    #[test]
    fn empty_stakes_builds_zero_signature() {
        let sig = build_validator_signature(&[]);
        assert_eq!(sig.bins, [0u32; N_BINS]);
    }

    #[test]
    fn single_stake_fills_top_bin() {
        let sig = build_validator_signature(&[1000]);
        // With scale=1000 the single value 1000 maps to the top bin.
        assert!(sig.bins[N_BINS - 1] > 0);
    }
}
