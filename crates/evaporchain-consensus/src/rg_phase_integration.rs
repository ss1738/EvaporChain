//! RG Phase Map integration — wires `evaporchain-rg-phase-map` into the
//! consensus layer.
//!
//! After each WSBF RG step the chain uses `classify_regime` to determine
//! which of four operating regimes it is in:
//!
//! - `LivenessStable` — f < 10%, λ_eff high: both safety and liveness met.
//! - `SafetyStable`  — f < 33%, λ_eff acceptable: BFT safety but marginal liveness.
//! - `Frozen`        — λ_eff too low or validator count too small to form quorum.
//! - `Chaotic`       — f ≥ 33%: BFT safety broken; chain may invoke Tombstone.
//!
//! The current phase is stored on `TendermintConsensus` and exposed for:
//! - Light clients: skip liveness-sensitive operations in `Frozen`/`Chaotic`.
//! - Governance: automatic parameter adjustments triggered by phase transitions.
//! - Slashing: `Chaotic` triggers enhanced evidence collection.

use evaporchain_rg_phase_map::{classify_regime, ConsensusPhase, PhaseMapParams};
use evaporchain_wsbf::params::EffectiveParams;
use tracing::{info, warn};

pub use evaporchain_rg_phase_map::ConsensusPhase as Phase;

/// Classify the consensus regime from a completed WSBF `EffectiveParams`.
///
/// `adversary_fraction_per_mille` is caller-supplied — typically derived
/// from the slashing evidence tracker (0 = no known adversaries).
pub fn classify_from_effective_params(
    ep: &EffectiveParams,
    n_validators: u64,
    adversary_fraction_per_mille: u64,
    params: &PhaseMapParams,
) -> ConsensusPhase {
    classify_regime(ep.lambda_eff, n_validators, adversary_fraction_per_mille, params)
}

/// Log a phase transition at INFO/WARN level.  Called when `current_phase`
/// changes after an RG step.
pub fn log_phase_transition(prev: ConsensusPhase, next: ConsensusPhase, height: u64) {
    if prev == next {
        return;
    }
    match next {
        ConsensusPhase::Chaotic => warn!(
            height,
            prev = ?prev,
            "consensus phase → Chaotic (f ≥ 1/3 — BFT safety broken)"
        ),
        ConsensusPhase::Frozen => warn!(
            height,
            prev = ?prev,
            "consensus phase → Frozen (λ_eff too low or quorum too small)"
        ),
        _ => info!(
            height,
            prev = ?prev,
            next = ?next,
            "consensus phase transition"
        ),
    }
}

/// True iff `phase` allows normal block production.
///
/// `Frozen` and `Chaotic` both block the proposer from producing new blocks
/// in a fully-integrated deployment.  At the substrate integration stage this
/// is advisory — the proposer is not hard-gated yet.
pub fn is_producing(phase: ConsensusPhase) -> bool {
    matches!(phase, ConsensusPhase::LivenessStable | ConsensusPhase::SafetyStable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(lambda_eff: u64) -> EffectiveParams {
        EffectiveParams {
            step: 0,
            height_start: 0,
            height_end: 99,
            lambda_eff,
            effective_accounts: 100,
            energy_density: 50,
            entropy_mb: 0,
        }
    }

    #[test]
    fn high_lambda_low_adversary_is_liveness_stable() {
        let params = PhaseMapParams::default();
        let phase = classify_from_effective_params(&ep(1_000), 10, 0, &params);
        assert_eq!(phase, ConsensusPhase::LivenessStable);
    }

    #[test]
    fn low_lambda_gives_frozen() {
        let params = PhaseMapParams::default();
        // lambda_freeze default = 10; λ_eff = 5 → Frozen.
        let phase = classify_from_effective_params(&ep(5), 10, 0, &params);
        assert_eq!(phase, ConsensusPhase::Frozen);
    }

    #[test]
    fn high_adversary_fraction_gives_chaotic() {
        let params = PhaseMapParams::default();
        let phase = classify_from_effective_params(&ep(1_000), 10, 400, &params);
        assert_eq!(phase, ConsensusPhase::Chaotic);
    }

    #[test]
    fn marginal_adversary_gives_safety_stable() {
        let params = PhaseMapParams::default();
        // 15% adversary (150‰) > 10% liveness threshold but < 33% chaos threshold.
        let phase = classify_from_effective_params(&ep(1_000), 10, 150, &params);
        assert_eq!(phase, ConsensusPhase::SafetyStable);
    }

    #[test]
    fn frozen_and_chaotic_are_not_producing() {
        assert!(!is_producing(ConsensusPhase::Frozen));
        assert!(!is_producing(ConsensusPhase::Chaotic));
    }

    #[test]
    fn liveness_and_safety_stable_are_producing() {
        assert!(is_producing(ConsensusPhase::LivenessStable));
        assert!(is_producing(ConsensusPhase::SafetyStable));
    }
}
