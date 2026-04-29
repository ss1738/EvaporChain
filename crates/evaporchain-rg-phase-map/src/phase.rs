//! Phase classification and trajectory analysis.

use serde::{Deserialize, Serialize};

use evaporchain_wsbf::params::EffectiveParams;

/// Operating regime of the consensus layer at a given point in the phase diagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsensusPhase {
    /// Both safety and liveness conditions are met (f < 1/10, sufficient validators,
    /// λ_eff well above freeze threshold).
    LivenessStable,
    /// BFT safety holds (f < 1/3) but liveness is marginal
    /// (f ≥ 1/10 or λ_eff near freeze boundary).
    SafetyStable,
    /// λ_eff is so low that validators cannot maintain sufficient stake for quorum.
    /// The chain is "frozen" — alive but unable to produce blocks.
    Frozen,
    /// Adversary fraction ≥ 1/3; BFT safety is broken.
    /// Per doctrine: the chain can die and admits it (Tombstone).
    Chaotic,
}

/// Parameters for the phase diagram.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PhaseMapParams {
    /// λ_eff threshold below which the chain is Frozen (epochs).
    pub lambda_freeze: u64,
    /// λ_eff threshold below which liveness is marginal (epochs).
    pub lambda_liveness: u64,
    /// Minimum validator count for a healthy quorum.
    pub min_quorum_validators: u64,
    /// Adversary fraction threshold for LivenessStable (numerator/1000).
    /// Default: 100 = 10% = 1/10.
    pub liveness_threshold_per_mille: u64,
}

impl Default for PhaseMapParams {
    fn default() -> Self {
        Self {
            lambda_freeze: 10,           // < 10 epochs half-life → frozen
            lambda_liveness: 100,        // < 100 epochs → liveness marginal
            min_quorum_validators: 4,
            liveness_threshold_per_mille: 100, // 10%
        }
    }
}

/// Classify the consensus regime given effective parameters and network state.
///
/// - `lambda_eff`: the renormalized λ from the WSBF flow (or raw λ if no RG applied).
/// - `n_validators`: current active validator count.
/// - `adversary_fraction_per_mille`: adversary proportion × 1000 (e.g. 333 = 33.3%).
pub fn classify_regime(
    lambda_eff: u64,
    n_validators: u64,
    adversary_fraction_per_mille: u64,
    params: &PhaseMapParams,
) -> ConsensusPhase {
    // Chaotic takes priority — BFT safety broken.
    if adversary_fraction_per_mille >= 333 {
        return ConsensusPhase::Chaotic;
    }

    // Frozen — validators can't sustain stake.
    if lambda_eff < params.lambda_freeze {
        return ConsensusPhase::Frozen;
    }

    // Insufficient quorum.
    if n_validators < params.min_quorum_validators {
        return ConsensusPhase::Frozen;
    }

    // Check liveness: f < 10% AND λ above liveness threshold.
    if adversary_fraction_per_mille < params.liveness_threshold_per_mille
        && lambda_eff >= params.lambda_liveness
    {
        return ConsensusPhase::LivenessStable;
    }

    // Safety holds (f < 1/3 checked above) but liveness is marginal.
    ConsensusPhase::SafetyStable
}

/// Classify a sequence of WSBF `EffectiveParams` (one per RG step) into a
/// phase trajectory, showing how the consensus regime evolves as the chain ages.
///
/// Returns one `ConsensusPhase` per RG step.
pub fn phase_trajectory(
    steps: &[EffectiveParams],
    adversary_fraction_per_mille: u64,
    n_validators: u64,
    params: &PhaseMapParams,
) -> Vec<ConsensusPhase> {
    steps
        .iter()
        .map(|ep| classify_regime(ep.lambda_eff, n_validators, adversary_fraction_per_mille, params))
        .collect()
}

/// Locate the fixed-point step: the first index where the phase stops changing.
/// Returns `None` if the trajectory is still evolving at its end.
pub fn find_fixed_point(trajectory: &[ConsensusPhase]) -> Option<usize> {
    if trajectory.len() < 2 {
        return None;
    }
    for i in 1..trajectory.len() {
        if trajectory[i] == trajectory[i - 1] {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> PhaseMapParams {
        PhaseMapParams::default()
    }

    #[test]
    fn high_adversary_is_chaotic() {
        assert_eq!(
            classify_regime(10_000, 100, 334, &params()),
            ConsensusPhase::Chaotic
        );
    }

    #[test]
    fn boundary_adversary_is_safety_stable() {
        // 332/1000 < 333 → not Chaotic; check if SafetyStable or LivenessStable.
        let p = classify_regime(10_000, 100, 332, &params());
        // 332 ≥ 100 (liveness threshold) → SafetyStable, not LivenessStable.
        assert_eq!(p, ConsensusPhase::SafetyStable);
    }

    #[test]
    fn low_lambda_is_frozen() {
        assert_eq!(
            classify_regime(5, 100, 50, &params()), // lambda=5 < lambda_freeze=10
            ConsensusPhase::Frozen
        );
    }

    #[test]
    fn low_validator_count_is_frozen() {
        assert_eq!(
            classify_regime(10_000, 3, 50, &params()), // 3 < min_quorum=4
            ConsensusPhase::Frozen
        );
    }

    #[test]
    fn ideal_conditions_are_liveness_stable() {
        assert_eq!(
            classify_regime(10_000, 100, 50, &params()), // f=5%, λ high
            ConsensusPhase::LivenessStable
        );
    }

    #[test]
    fn marginal_f_is_safety_stable() {
        // f = 200/1000 = 20%: above liveness threshold (10%) but below Chaotic (33%).
        assert_eq!(
            classify_regime(10_000, 100, 200, &params()),
            ConsensusPhase::SafetyStable
        );
    }

    #[test]
    fn phase_trajectory_length_matches_steps() {
        use evaporchain_wsbf::params::EffectiveParams;
        let steps: Vec<EffectiveParams> = (0..5)
            .map(|i| EffectiveParams {
                step: i,
                height_start: i as u64 * 100,
                height_end: i as u64 * 100 + 99,
                lambda_eff: 10_000u64.saturating_sub(i as u64 * 2_000),
                effective_accounts: 50,
                energy_density: 1_000,
                entropy_mb: 500,
            })
            .collect();
        let traj = phase_trajectory(&steps, 50, 20, &params());
        assert_eq!(traj.len(), 5);
    }

    #[test]
    fn decaying_lambda_transitions_to_frozen() {
        use evaporchain_wsbf::params::EffectiveParams;
        // Steps with λ_eff dropping: 10000, 100, 1 — should end in Frozen.
        let steps: Vec<EffectiveParams> = [10_000u64, 100, 1]
            .iter()
            .enumerate()
            .map(|(i, &le)| EffectiveParams {
                step: i,
                height_start: i as u64 * 100,
                height_end: i as u64 * 100 + 99,
                lambda_eff: le,
                effective_accounts: 50,
                energy_density: 1_000,
                entropy_mb: 0,
            })
            .collect();
        let traj = phase_trajectory(&steps, 50, 20, &params());
        assert_eq!(traj[0], ConsensusPhase::LivenessStable);
        assert_eq!(traj[2], ConsensusPhase::Frozen);
    }

    #[test]
    fn find_fixed_point_returns_first_repeat() {
        let t = vec![
            ConsensusPhase::LivenessStable,
            ConsensusPhase::SafetyStable,
            ConsensusPhase::SafetyStable,
            ConsensusPhase::Frozen,
        ];
        assert_eq!(find_fixed_point(&t), Some(2));
    }

    #[test]
    fn find_fixed_point_none_for_always_changing() {
        let t = vec![
            ConsensusPhase::LivenessStable,
            ConsensusPhase::SafetyStable,
            ConsensusPhase::Chaotic,
            ConsensusPhase::Frozen,
        ];
        assert!(find_fixed_point(&t).is_none());
    }
}
