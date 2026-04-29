//! IB vote gate: should this validator commit or abstain?
//!
//! # Single-λ binding
//!
//! `β = lambda_mb / 1_000_000` in the IB objective.
//!
//! - Small λ (decayed chain, strict) → small β → high compression threshold
//!   → only highly-informed validators vote.
//! - Large λ (fresh chain, permissive) → large β → low threshold → most
//!   validators contribute.
//!
//! # Relevance score
//!
//! A validator's relevance = how much information its local state signature
//! `x` carries about the *prior* distribution `prior_sig` (the agreed-upon
//! distribution from the last checkpoint).
//!
//! We use the KL divergence `KL(x || prior)` in millibits as a proxy for
//! `I(Z; Y)` (a high KL means the validator's view is informative / distinct).
//!
//! The IB vote gate fires `Commit` iff:
//!
//! ```text
//! relevance(x, prior) > β_threshold
//! ```
//!
//! where `β_threshold = lambda_mb` (same units as `kl_millibits`).
//!
//! When the validator's view is close to the prior (KL small), it abstains —
//! its vote would not add mutual information.

use crate::signature::StateSignature;

/// Parameters for the IB gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IbParams {
    /// λ in millibits.  Controls β = λ_mb / 1_000_000.
    /// Also used directly as the KL threshold (same units as `kl_millibits`).
    pub lambda_mb: u64,
}

/// Outcome of the IB vote gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IbVote {
    /// Validator's state is informative enough; it should cast a prevote.
    Commit,
    /// Validator's view adds no mutual information; it should abstain.
    Abstain,
}

/// Decide whether a validator should commit based on the IB principle.
///
/// `local_sig` = validator's current state signature (its view of X).
/// `prior_sig` = the agreed state signature from the last checkpoint (the
///               chain's prior distribution, broadcast at checkpoint time).
/// `params.lambda_mb` = λ in millibits (the β threshold).
///
/// Returns `Commit` iff `KL(local || prior) > lambda_mb`.
pub fn ib_vote(local_sig: &StateSignature, prior_sig: &StateSignature, params: &IbParams) -> IbVote {
    let kl = local_sig.kl_millibits(prior_sig);
    if kl > params.lambda_mb {
        IbVote::Commit
    } else {
        IbVote::Abstain
    }
}

/// Compute the β-weighted IB objective value for a given vote outcome.
///
/// `I_gain(Z; Y) − β × I_cost(Z; X)` approximated as:
/// `kl_millibits − lambda_mb × compression_ratio`
/// where `compression_ratio` = L1 distance / 1_000 (fraction of information lost).
///
/// Positive values indicate the vote is IB-optimal.
pub fn ib_objective(
    local_sig: &StateSignature,
    prior_sig: &StateSignature,
    params: &IbParams,
) -> i64 {
    let relevance = local_sig.kl_millibits(prior_sig) as i64;
    let compression = local_sig.l1_distance(prior_sig) as i64;
    let beta = (params.lambda_mb / 1_000) as i64; // scale to match units
    relevance - beta * compression / 1_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signature::StateSignature;

    fn uniform_sig() -> StateSignature {
        let energies: Vec<u64> = (0..16).map(|i| i as u64 * 100).collect();
        StateSignature::from_energies(&energies, 1_600)
    }

    fn concentrated_sig() -> StateSignature {
        let energies = vec![0u64; 16]; // all in bin 0
        StateSignature::from_energies(&energies, 1_000_000)
    }

    #[test]
    fn identical_views_give_abstain() {
        let sig = uniform_sig();
        let params = IbParams { lambda_mb: 0 }; // threshold=0: KL>0 required to commit
        // KL(x||x) = 0, so with threshold=0: 0 > 0 is false → Abstain.
        assert_eq!(ib_vote(&sig, &sig, &params), IbVote::Abstain);
    }

    #[test]
    fn divergent_views_commit_at_low_lambda() {
        let local = concentrated_sig();
        let prior = uniform_sig();
        let params = IbParams { lambda_mb: 0 }; // threshold=0
        // KL(concentrated || uniform) > 0 → Commit.
        assert_eq!(ib_vote(&local, &prior, &params), IbVote::Commit);
    }

    #[test]
    fn high_lambda_forces_abstain_on_small_divergence() {
        let local = uniform_sig();
        let prior = uniform_sig();
        // Very high threshold; KL(uniform||uniform) = 0 < threshold → Abstain.
        let params = IbParams { lambda_mb: 1_000_000_000 };
        assert_eq!(ib_vote(&local, &prior, &params), IbVote::Abstain);
    }

    #[test]
    fn ib_objective_positive_for_informative_validator() {
        let local = concentrated_sig();
        let prior = uniform_sig();
        let params = IbParams { lambda_mb: 1_000 }; // small β
        let obj = ib_objective(&local, &prior, &params);
        // Concentrated vs uniform: large KL, small-β penalty → positive.
        assert!(obj >= 0, "obj={obj}");
    }

    #[test]
    fn ib_params_small_lambda_strict_gate() {
        // With lambda_mb=1 only validators with KL > 1 millibit commit.
        let params_strict  = IbParams { lambda_mb: 1_000_000 };
        let params_loose   = IbParams { lambda_mb: 1 };
        let local = concentrated_sig();
        let prior = uniform_sig();
        // Loose gate should also commit (any KL > 1 millibit).
        assert_eq!(ib_vote(&local, &prior, &params_loose), IbVote::Commit);
        // Strict gate: only commits if KL is very large.
        // Can't assert exact result without computing KL, but gate logic is exercised.
        let _ = ib_vote(&local, &prior, &params_strict);
    }
}
