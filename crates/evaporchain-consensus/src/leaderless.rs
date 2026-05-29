//! `LeaderlessProposer` — the V1.5 leaderless block-production seam.
//!
//! V1 picks a single proposer per `(height, round)` by a deterministic
//! stake-weighted rotation. V1.5 (`docs/proposals/
//! leaderless-block-production-v15.md` §2.1) lifts that: ANY validator
//! becomes eligible to emit a block — whose `parents` form a valid
//! antichain of recent heads — when their VRF output for
//! `(height, chain_id, validator_id)` falls below an adaptive,
//! stake-weighted threshold tied to the recent block rate. Recipients
//! feed competing proposals to MCC fork-choice, which converges on the
//! highest-caliber tip.
//!
//! Compiled ONLY under the default-off `doctrine_v1_5` feature, so the
//! V1 leader-rotation hot path is wholly untouched until an operator
//! flips it at a fork epoch. This module mirrors the
//! [`crate::fork_choice::ForkChoice`] seam.
//!
//! **Phase 0 (this commit):** the trait + context + a disabled
//! placeholder so the seam compiles and is testable. **Phase 2** lands
//! the real VRF-eligibility implementation behind it.

/// Inputs to a leaderless eligibility decision for one validator at one
/// height, assembled by the consensus layer from chain state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibilityContext {
    /// Height being proposed for.
    pub height: u64,
    /// Consensus round at that height.
    pub round: u64,
    /// This validator's index in the active set.
    pub validator_id: u64,
    /// This validator's stake weight.
    pub stake_weight: u64,
    /// Total active stake — denominator for the stake-weighted threshold.
    pub total_stake: u64,
    /// Recent block-production rate (blocks over a trailing window).
    /// The eligibility threshold scales against this so the aggregate
    /// emission rate stays stable as the stake distribution shifts.
    pub recent_block_rate: u64,
    /// This validator's VRF output for `(height, chain_id, validator_id)`
    /// — the unpredictable-but-verifiable eligibility lottery ticket.
    /// `chain_id` is already bound into this value by the caller.
    pub vrf_output: [u8; 32],
}

/// V1.5 leaderless block-production strategy. Implementors decide
/// whether a validator may emit a block this height and which antichain
/// of parents to attach.
///
/// **Validator-determinism is the safety contract:** every honest
/// validator computing eligibility / parents for the same inputs and
/// DAG state MUST reach the same answer, or the network forks. (This is
/// the same determinism requirement the `ForkChoice` seam carries.)
pub trait LeaderlessProposer: Send + Sync {
    /// Whether the validator described by `ctx` is eligible to emit a
    /// block at `ctx.height`.
    fn is_eligible(&self, ctx: &EligibilityContext) -> bool;

    /// The antichain parent set to attach to an emitted block, chosen
    /// from the current DAG leaves (validator-deterministically, e.g.
    /// via `MccForkChoice::enumerate_candidate_heads`). Empty when the
    /// proposer has nothing valid to emit.
    fn propose_parents(&self, dag_leaves: &[[u8; 32]]) -> Vec<[u8; 32]>;

    /// Operator-visible label.
    fn name(&self) -> &'static str;
}

/// Phase 0 placeholder: never eligible, emits no parents. Keeps the
/// seam compilable + testable before Phase 2 lands the VRF-eligibility
/// implementation. Selecting this is equivalent to "leaderless off".
#[derive(Debug, Default, Clone, Copy)]
pub struct DisabledLeaderlessProposer;

impl LeaderlessProposer for DisabledLeaderlessProposer {
    fn is_eligible(&self, _ctx: &EligibilityContext) -> bool {
        false
    }
    fn propose_parents(&self, _dag_leaves: &[[u8; 32]]) -> Vec<[u8; 32]> {
        Vec::new()
    }
    fn name(&self) -> &'static str {
        "disabled"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> EligibilityContext {
        EligibilityContext {
            height: 10,
            round: 0,
            validator_id: 3,
            stake_weight: 100,
            total_stake: 500,
            recent_block_rate: 1,
            vrf_output: [0u8; 32],
        }
    }

    #[test]
    fn disabled_proposer_is_never_eligible() {
        let p = DisabledLeaderlessProposer;
        assert!(!p.is_eligible(&ctx()));
        assert!(p.propose_parents(&[[1u8; 32], [2u8; 32]]).is_empty());
        assert_eq!(p.name(), "disabled");
    }

    #[test]
    fn usable_as_trait_object() {
        let p: &dyn LeaderlessProposer = &DisabledLeaderlessProposer;
        assert_eq!(p.name(), "disabled");
        assert!(!p.is_eligible(&ctx()));
    }
}
