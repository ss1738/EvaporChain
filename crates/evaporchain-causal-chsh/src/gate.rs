//! Synthetic gate — runs the CHSH statistic against an HONEST sample
//! and a CARTEL sample, checks the doctrine's three thresholds.
//!
//! Real-Ethereum gate is a follow-up lane (O.2) that pulls a sample of
//! concurrent pairs from a real LightCone trace + a synthetic cartel
//! injection and writes `research/causal-chsh/GATE_RESULT.md` in the
//! same shape as MERA's gate did.
//!
//! This module ships the math + a smoke-test gate runnable from unit
//! tests, with no external data dependencies.

use crate::chsh::{compute_chsh_s, ConcurrentPairSamples};
use serde::{Deserialize, Serialize};

/// Doctrine-locked thresholds for the gate verdict.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GateThresholds {
    /// Maximum S allowed under honest traffic. If actual `S_honest >=`
    /// this, the inequality is vacuous (honest traffic already trips
    /// the bound — too noisy). Doctrine: 1.8.
    pub honest_ceiling: f64,
    /// Minimum S required under cartel traffic. If actual `S_cartel <=`
    /// this, the inequality fails to discriminate cartel from honest.
    /// Doctrine: 2.2.
    pub cartel_floor: f64,
    /// Minimum gap `S_cartel − S_honest` required. Even if both
    /// thresholds are individually OK, a small gap means weak
    /// signal-to-noise. Doctrine: 0.4.
    pub min_gap: f64,
}

impl GateThresholds {
    /// The doctrine-locked thresholds as committed in the crate
    /// docstring + INVENTION_STACK reservation. Don't change without
    /// a full doctrine amendment — these are part of the gate's
    /// pre-commitment, MERA-style.
    pub const fn doctrine() -> Self {
        Self {
            honest_ceiling: 1.8,
            cartel_floor: 2.2,
            min_gap: 0.4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GateVerdict {
    /// All three thresholds passed. The Causal-CHSH inequality
    /// empirically discriminates honest from cartel traffic.
    /// Doctrine commits: ship as a Tier-0-supporting primitive.
    Pass {
        s_honest: f64,
        s_cartel: f64,
        gap: f64,
    },
    /// One or more thresholds failed. Doctrine commits: drop the
    /// primitive. The crate is retained as a research artefact.
    Fail {
        s_honest: f64,
        s_cartel: f64,
        gap: f64,
        reasons: Vec<String>,
    },
    /// One of the inputs was malformed (empty sample, non-binary
    /// observable). Operator must fix the input and re-run. Not a
    /// doctrine verdict.
    InputError(String),
}

/// Run the gate against a paired (honest, cartel) sample. Returns the
/// verdict per the doctrine thresholds.
///
/// Inputs are four-sample CHSH bundles drawn from two LightCone traces:
/// - `honest_samples`: from a chain trace where validators do not
///   coordinate (real Ethereum, real EvaporChain testnet, or a
///   synthetic chain produced under independent randomness)
/// - `cartel_samples`: from a chain trace with deliberately-coordinated
///   producers (synthetic injection)
pub fn run_synthetic_gate(
    honest_samples: &ConcurrentPairSamples,
    cartel_samples: &ConcurrentPairSamples,
    thresholds: GateThresholds,
) -> GateVerdict {
    let s_honest = match compute_chsh_s(honest_samples) {
        Ok(s) => s,
        Err(e) => return GateVerdict::InputError(format!("honest sample: {e}")),
    };
    let s_cartel = match compute_chsh_s(cartel_samples) {
        Ok(s) => s,
        Err(e) => return GateVerdict::InputError(format!("cartel sample: {e}")),
    };
    let gap = s_cartel - s_honest;

    let mut reasons: Vec<String> = Vec::new();
    if s_honest >= thresholds.honest_ceiling {
        reasons.push(format!(
            "S_honest ({s_honest:.4}) ≥ ceiling ({:.2}) — inequality vacuous under honest traffic",
            thresholds.honest_ceiling
        ));
    }
    if s_cartel <= thresholds.cartel_floor {
        reasons.push(format!(
            "S_cartel ({s_cartel:.4}) ≤ floor ({:.2}) — inequality fails to discriminate cartel",
            thresholds.cartel_floor
        ));
    }
    if gap <= thresholds.min_gap {
        reasons.push(format!(
            "gap ({gap:.4}) ≤ min_gap ({:.2}) — discrimination has insufficient signal-to-noise",
            thresholds.min_gap
        ));
    }

    if reasons.is_empty() {
        GateVerdict::Pass {
            s_honest,
            s_cartel,
            gap,
        }
    } else {
        GateVerdict::Fail {
            s_honest,
            s_cartel,
            gap,
            reasons,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chsh::ConcurrentPairSamples;

    /// Honest model: each setting-pair sample is balanced (equal +1/−1).
    /// All four expectations → 0; S → 0.
    fn honest_samples() -> ConcurrentPairSamples {
        let bal = vec![1i8, -1, 1, -1, 1, -1, 1, -1];
        ConcurrentPairSamples {
            samples_ab: bal.clone(),
            samples_ab_prime: bal.clone(),
            samples_a_prime_b: bal.clone(),
            samples_a_prime_b_prime: bal,
        }
    }

    /// Cartel model: independently-rigged samples push S to algebraic
    /// max 4. Setting-pair (A,B), (A,B'), (A',B) all rigged to +1
    /// products; setting-pair (A',B') rigged to −1 products.
    /// S = |1 + 1 + 1 − (−1)| = 4. Beyond Tsirelson (2√2 ≈ 2.83);
    /// reflects deterministic coordination, not quantum entanglement.
    fn cartel_samples() -> ConcurrentPairSamples {
        ConcurrentPairSamples {
            samples_ab: vec![1; 8],
            samples_ab_prime: vec![1; 8],
            samples_a_prime_b: vec![1; 8],
            samples_a_prime_b_prime: vec![-1; 8],
        }
    }

    #[test]
    fn honest_yields_s_near_zero() {
        let s = compute_chsh_s(&honest_samples()).unwrap();
        assert!(s.abs() < 1e-9);
    }

    #[test]
    fn cartel_violates_bell_bound() {
        let s = compute_chsh_s(&cartel_samples()).unwrap();
        assert!(s > 2.0);
    }

    #[test]
    fn synthetic_gate_passes_doctrine_thresholds() {
        let v = run_synthetic_gate(
            &honest_samples(),
            &cartel_samples(),
            GateThresholds::doctrine(),
        );
        match v {
            GateVerdict::Pass {
                s_honest,
                s_cartel,
                gap,
            } => {
                assert!(s_honest < 1.8);
                assert!(s_cartel > 2.2);
                assert!(gap > 0.4);
                assert!((s_honest - 0.0).abs() < 1e-9);
                assert!((s_cartel - 4.0).abs() < 1e-9);
            }
            other => panic!("expected Pass, got {:?}", other),
        }
    }

    #[test]
    fn synthetic_gate_fails_when_honest_already_trips_bound() {
        // If "honest" sample is itself coordinated, the gate fails.
        let v = run_synthetic_gate(
            &cartel_samples(),
            &cartel_samples(),
            GateThresholds::doctrine(),
        );
        match v {
            GateVerdict::Fail { reasons, .. } => {
                assert!(reasons.iter().any(|r| r.contains("honest")));
            }
            other => panic!("expected Fail, got {:?}", other),
        }
    }

    #[test]
    fn synthetic_gate_input_error_on_empty() {
        let empty = ConcurrentPairSamples {
            samples_ab: vec![],
            samples_ab_prime: vec![1],
            samples_a_prime_b: vec![1],
            samples_a_prime_b_prime: vec![1],
        };
        let v = run_synthetic_gate(&empty, &cartel_samples(), GateThresholds::doctrine());
        assert!(matches!(v, GateVerdict::InputError(_)));
    }
}
