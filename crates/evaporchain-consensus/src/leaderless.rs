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

/// VRF-threshold leaderless proposer (V1.5 §2.1 — the chosen
/// eligibility model). A validator is eligible at a height when their
/// VRF lottery ticket falls below a stake-weighted threshold, so on
/// average `target_eligible_per_height` proposers (weighted by stake)
/// emit a block — any of them, with no leader. Algorand/Praos-style
/// VRF sortition adapted to EvaporChain's per-block VRF beacon.
///
/// Eligibility probability for a validator with stake fraction `s` is
/// `p = min(1, target_eligible_per_height * s)`. The threshold is `p`
/// scaled into the `[0, 2^64)` ticket space; the ticket is the top 8
/// bytes (big-endian) of the validator's VRF output.
#[derive(Debug, Clone, Copy)]
pub struct VrfLeaderlessProposer {
    /// Expected number of eligible proposers per height — the emission
    /// knob (governance-tunable). Higher = more concurrent proposers.
    pub target_eligible_per_height: u64,
}

impl VrfLeaderlessProposer {
    /// Mirrors `evaporchain_light_cone`'s SUB-N6 `MAX_PARENTS_PER_BLOCK`
    /// parent cap (kept local to avoid coupling on a non-exported const).
    const MAX_PARENTS: usize = 16;

    pub fn new(target_eligible_per_height: u64) -> Self {
        Self {
            // A target of 0 would make nobody eligible (dead chain);
            // clamp to ≥1 so there is always some emission pressure.
            target_eligible_per_height: target_eligible_per_height.max(1),
        }
    }

    /// Eligibility threshold in the `[0, u64::MAX]` ticket space: a
    /// validator is eligible iff their VRF ticket is strictly below it.
    /// `threshold / 2^64` is the eligibility probability. Overflow-safe
    /// (u128, saturating) and clamped to `u64::MAX` at ≥100% probability.
    /// Public so the predicate is unit-testable in isolation.
    pub fn threshold(&self, stake_weight: u64, total_stake: u64) -> u64 {
        if total_stake == 0 {
            return 0;
        }
        // threshold = target * stake_weight * 2^64 / total_stake.
        let num = (self.target_eligible_per_height as u128)
            .saturating_mul(stake_weight as u128)
            .saturating_mul(1u128 << 64);
        let scaled = num / (total_stake as u128);
        if scaled >= (1u128 << 64) {
            u64::MAX
        } else {
            scaled as u64
        }
    }
}

impl LeaderlessProposer for VrfLeaderlessProposer {
    fn is_eligible(&self, ctx: &EligibilityContext) -> bool {
        if ctx.stake_weight == 0 || ctx.total_stake == 0 {
            return false;
        }
        // Liveness floor (spec §1, liveness-on-leader-failure): if no
        // blocks have been produced recently the chain is stalling — let
        // every staked validator emit so the DAG can recover. Without
        // this a low-stake-fraction network could starve.
        if ctx.recent_block_rate == 0 {
            return true;
        }
        let mut t = [0u8; 8];
        t.copy_from_slice(&ctx.vrf_output[..8]);
        let ticket = u64::from_be_bytes(t);
        ticket < self.threshold(ctx.stake_weight, ctx.total_stake)
    }

    fn propose_parents(&self, dag_leaves: &[[u8; 32]]) -> Vec<[u8; 32]> {
        // DAG leaves are mutually concurrent (a valid antichain); adopt
        // up to the SUB-N6 parent cap, preserving the caller's
        // deterministic ordering.
        dag_leaves
            .iter()
            .take(Self::MAX_PARENTS)
            .copied()
            .collect()
    }

    fn name(&self) -> &'static str {
        "vrf"
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

    // ── VRF leaderless proposer (Phase 2) ────────────────────────────

    fn vrf_with_ticket(t: u64) -> [u8; 32] {
        let mut v = [0u8; 32];
        v[..8].copy_from_slice(&t.to_be_bytes());
        v
    }

    #[test]
    fn vrf_zero_stake_is_ineligible() {
        let p = VrfLeaderlessProposer::new(1);
        let mut c = ctx();
        c.stake_weight = 0;
        c.vrf_output = vrf_with_ticket(0); // best possible ticket
        assert!(!p.is_eligible(&c));
    }

    #[test]
    fn vrf_zero_total_stake_is_ineligible() {
        let p = VrfLeaderlessProposer::new(1);
        let mut c = ctx();
        c.total_stake = 0;
        assert!(!p.is_eligible(&c));
        assert_eq!(p.threshold(c.stake_weight, 0), 0);
    }

    #[test]
    fn vrf_ticket_below_threshold_is_eligible() {
        // stake fraction 1/100, target 1 → threshold ≈ 2^64/100.
        let p = VrfLeaderlessProposer::new(1);
        let mut c = ctx();
        c.stake_weight = 1;
        c.total_stake = 100;
        c.recent_block_rate = 1; // not stalled
        c.vrf_output = vrf_with_ticket(0);
        assert!(p.is_eligible(&c));
    }

    #[test]
    fn vrf_ticket_above_threshold_is_ineligible() {
        let p = VrfLeaderlessProposer::new(1);
        let mut c = ctx();
        c.stake_weight = 1;
        c.total_stake = 100;
        c.recent_block_rate = 1;
        c.vrf_output = vrf_with_ticket(u64::MAX); // worst ticket
        assert!(!p.is_eligible(&c));
    }

    #[test]
    fn vrf_threshold_is_monotone_in_stake() {
        let p = VrfLeaderlessProposer::new(1);
        assert!(p.threshold(50, 100) > p.threshold(10, 100));
        // full stake at target 1 → certainty (clamped to u64::MAX).
        assert_eq!(p.threshold(100, 100), u64::MAX);
    }

    #[test]
    fn vrf_liveness_floor_when_stalled() {
        // recent_block_rate == 0 → any staked validator is eligible,
        // even with the worst possible ticket.
        let p = VrfLeaderlessProposer::new(1);
        let mut c = ctx();
        c.stake_weight = 1;
        c.total_stake = 1_000_000;
        c.recent_block_rate = 0;
        c.vrf_output = vrf_with_ticket(u64::MAX);
        assert!(p.is_eligible(&c));
    }

    #[test]
    fn vrf_new_clamps_zero_target_to_one() {
        assert_eq!(VrfLeaderlessProposer::new(0).target_eligible_per_height, 1);
    }

    #[test]
    fn vrf_propose_parents_caps_at_max_and_preserves_order() {
        let p = VrfLeaderlessProposer::new(1);
        let leaves: Vec<[u8; 32]> = (0..20u8).map(|i| [i; 32]).collect();
        let parents = p.propose_parents(&leaves);
        assert_eq!(parents.len(), 16); // SUB-N6 cap
        assert_eq!(parents[0], [0u8; 32]);
        assert_eq!(parents[15], [15u8; 32]);
        // a small leaf set passes through unchanged.
        let few = vec![[1u8; 32], [2u8; 32]];
        assert_eq!(p.propose_parents(&few), few);
    }

    #[test]
    fn vrf_name_is_vrf() {
        assert_eq!(VrfLeaderlessProposer::new(1).name(), "vrf");
    }
}
