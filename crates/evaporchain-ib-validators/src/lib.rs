//! Information-Bottleneck Validators — A4.3.1.
//!
//! **Companion: V2.** `evaporchain-ib-validators-v2` wraps this V1
//! IB-vote gate (`Commit | Abstain` from KL divergence) with three
//! structural rejection paths: CHSH-failed-window jail, energy-floor
//! jail, and explicit operator slash. V1 says nothing about validator
//! eligibility; V2 adds the structural-jail layer over the V1 vote
//! decision. V1 currently has 0 direct dependents in `Cargo.toml`
//! workspace members (V2 has 2); V1 stays as the standalone vote
//! primitive that V2 is built on top of.
//!
//! # Doctrine
//!
//! From INVENTION_STACK.md A4.3.1 (PROMISING):
//! > β = λ. Validators act as IB compressors: they compress chain state X
//! > into a bottleneck Z that retains maximum mutual information with future
//! > state Y.  The Lagrange multiplier β controlling compression vs relevance
//! > is tied to the chain's single λ.
//!
//! # Tishby-Pereira-Bialek 1999
//!
//! The IB objective (to maximise) is:
//!
//! ```text
//! L[p(Z|X)] = I(Z; Y) − β × I(Z; X)
//! ```
//!
//! - `I(Z; X)` = bits the bottleneck Z retains about the input X (compression
//!   cost).
//! - `I(Z; Y)` = bits Z retains about the output/future Y (relevance).
//! - `β` = Lagrange multiplier; small β = coarse compression; large β = fine.
//!
//! # Chain instantiation
//!
//! - **X** = a validator's local view of current chain state (recent account
//!   energy histogram, represented as a `StateSignature` — discretised into
//!   `N_BINS` buckets).
//! - **Y** = the *committed* chain root hash at the next checkpoint (the
//!   target a validator is trying to predict / agree on).
//! - **Z** = the validator's *vote* — a compressed representation.  We use a
//!   1-bit vote (`commit` / `abstain`) as the simplest bottleneck.
//! - **β** = `lambda_mb / 1_000_000` (λ in millibits → fraction).  As λ
//!   decreases the validator set becomes more selective (higher compression).
//!
//! # Practical computation
//!
//! For a 1-bit Z, the IB objective simplifies to choosing the vote that
//! maximises the mutual information gain.  We approximate this as:
//!
//! ```text
//! vote = commit  iff  I_gain(state_sig) > β_threshold
//! ```
//!
//! where `I_gain` is the per-validator *relevance score* computed from
//! how closely the validator's state signature matches the prior distribution
//! of state signatures that historically led to consensus.

pub mod ib;
pub mod signature;

pub use ib::{ib_vote, IbParams, IbVote};
pub use signature::{StateSignature, N_BINS};

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "IB Validators implement Tishby-Pereira-Bialek
    /// 1999 with β = lambda_mb / 1_000_000 tied to the chain's λ.
    /// Identical local/prior views (KL = 0) ALWAYS abstain — no
    /// information-free vote can ride the threshold. Strictly
    /// divergent views commit when KL exceeds lambda_mb. The
    /// boundary is strict (>, not ≥)."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let uniform_energies: Vec<u64> = (0..16).map(|i| i as u64 * 100).collect();
        let uniform = StateSignature::from_energies(&uniform_energies, 1_600);
        let concentrated = StateSignature::from_energies(&vec![0u64; 16], 1_000_000);

        // (a) Identical views: KL = 0 → ABSTAIN even at threshold=0.
        let p_zero = IbParams { lambda_mb: 0 };
        assert_eq!(ib_vote(&uniform, &uniform, &p_zero), IbVote::Abstain);

        // (b) Divergent views with low threshold → COMMIT.
        assert_eq!(ib_vote(&concentrated, &uniform, &p_zero), IbVote::Commit);

        // (c) High threshold forces abstain even on identical views.
        let p_high = IbParams { lambda_mb: 1_000_000_000 };
        assert_eq!(ib_vote(&uniform, &uniform, &p_high), IbVote::Abstain);

        // (d) Bottleneck width: N_BINS = 16 (the canonical signature
        // discretisation).
        assert_eq!(N_BINS, 16);
    }
}
