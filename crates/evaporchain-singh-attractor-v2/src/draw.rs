//! Bell-anchored draw + bounded drift.

use blake3::Hasher;
use evaporchain_singh_attractor::Attractor;
use evaporchain_types::Energy;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const DRAW_DOMAIN: &[u8] = b"evaporchain:singh-attractor:v2:draw\0";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DrawError {
    #[error("attractor list empty")]
    Empty,
    #[error("attractor at index {idx} has zero basin_radius and zero drift_rate — cannot make progress")]
    DegenerateAttractor { idx: usize },
}

/// V2 attractor: V1 fields plus a per-attractor `drift_rate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttractorV2 {
    pub center: Energy,
    pub basin_radius: Energy,
    /// Maximum per-epoch drift step magnitude. The chain applies
    /// `min(|center − state|, drift_rate)` toward the centre.
    pub drift_rate: u64,
}

impl AttractorV2 {
    pub const fn new(center: Energy, basin_radius: Energy, drift_rate: u64) -> Self {
        Self {
            center,
            basin_radius,
            drift_rate,
        }
    }

    pub fn as_v1(&self) -> Attractor {
        Attractor::new(self.center, self.basin_radius)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrawResult {
    pub selected_center: Energy,
    pub selected_index: usize,
    /// Signed drift toward the centre. Positive = increase state;
    /// negative = decrease. Magnitude bounded by `drift_rate`.
    pub drift: i128,
    pub used_fallback: bool,
}

/// Draw an attractor for the given state. If a basin contains the
/// state, V2 selects identically to V1 and the seed is unused.
/// Otherwise V2 weights the attractors by inverse energy-distance
/// and samples deterministically from the seed.
pub fn draw_attractor(
    state: Energy,
    attractors: &[AttractorV2],
    certificate_seed: &[u8; 32],
) -> Result<DrawResult, DrawError> {
    if attractors.is_empty() {
        return Err(DrawError::Empty);
    }
    for (idx, a) in attractors.iter().enumerate() {
        if a.basin_radius == 0 && a.drift_rate == 0 {
            return Err(DrawError::DegenerateAttractor { idx });
        }
    }

    // In-basin: select the first attractor that contains the state.
    if let Some((idx, a)) = attractors.iter().enumerate().find(|(_, a)| a.as_v1().contains(state)) {
        return Ok(DrawResult {
            selected_center: a.center,
            selected_index: idx,
            drift: drift_toward(state, a.center, a.drift_rate),
            used_fallback: false,
        });
    }

    // Out-of-basin: weighted sample with Bell-derived seed.
    let weights: Vec<u128> = attractors
        .iter()
        .map(|a| inverse_distance_weight(state, a.center))
        .collect();
    let total_weight: u128 = weights.iter().copied().sum();

    // Empty / zero-weight: fall back to nearest centre (V1
    // behaviour) so the function never panics.
    let selected_idx = if total_weight == 0 {
        nearest_index(state, attractors)
    } else {
        sample_weighted(certificate_seed, &weights, total_weight)
    };

    let a = &attractors[selected_idx];
    Ok(DrawResult {
        selected_center: a.center,
        selected_index: selected_idx,
        drift: drift_toward(state, a.center, a.drift_rate),
        used_fallback: true,
    })
}

/// Inverse distance + 1 (so equality gets weight 1, not infinity).
/// Returns u128 to avoid overflow in the sum.
fn inverse_distance_weight(state: Energy, center: Energy) -> u128 {
    // Larger distance → smaller weight. Use `2^32 / (1 + distance)`
    // so all weights live comfortably in u128.
    let d = state.abs_diff(center) as u128;
    let denom = 1u128 + d;
    // Numerator chosen so weights stay in u128 even with millions of
    // attractors and energies up to 2^60.
    (1u128 << 32) / denom
}

fn nearest_index(state: Energy, attractors: &[AttractorV2]) -> usize {
    attractors
        .iter()
        .enumerate()
        .min_by_key(|(_, a)| a.center.abs_diff(state))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Deterministic weighted sample from a 32-byte seed.
fn sample_weighted(seed: &[u8; 32], weights: &[u128], total: u128) -> usize {
    debug_assert!(total > 0);
    let mut h = Hasher::new();
    h.update(DRAW_DOMAIN);
    h.update(seed);
    let bytes = *h.finalize().as_bytes();
    // Take first 16 bytes as u128, then mod total.
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&bytes[..16]);
    let r = u128::from_le_bytes(buf) % total;
    let mut acc: u128 = 0;
    for (i, w) in weights.iter().enumerate() {
        acc = acc.saturating_add(*w);
        if r < acc {
            return i;
        }
    }
    weights.len() - 1
}

fn drift_toward(state: Energy, center: Energy, drift_rate: u64) -> i128 {
    if state == center || drift_rate == 0 {
        return 0;
    }
    let raw_step = (state.abs_diff(center)).min(drift_rate);
    let signed = raw_step as i128;
    if center > state {
        signed
    } else {
        -signed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(center: Energy, basin: Energy, drift: u64) -> AttractorV2 {
        AttractorV2::new(center, basin, drift)
    }

    fn seed(b: u8) -> [u8; 32] {
        [b; 32]
    }

    // ── shape errors ─────────────────────────────────────────────

    #[test]
    fn empty_attractors_rejected() {
        let r = draw_attractor(1000, &[], &seed(0));
        assert_eq!(r.unwrap_err(), DrawError::Empty);
    }

    #[test]
    fn degenerate_attractor_rejected() {
        let r = draw_attractor(1000, &[a(100, 0, 0)], &seed(0));
        assert!(matches!(r.unwrap_err(), DrawError::DegenerateAttractor { idx: 0 }));
    }

    // ── in-basin selection (V1 compat) ───────────────────────────

    #[test]
    fn in_basin_selects_first_basin_owner() {
        let attractors = [a(100, 50, 5), a(1000, 100, 10), a(10_000, 1000, 100)];
        let r = draw_attractor(1050, &attractors, &seed(0)).unwrap();
        assert_eq!(r.selected_center, 1000);
        assert_eq!(r.selected_index, 1);
        assert!(!r.used_fallback);
    }

    #[test]
    fn in_basin_selection_is_seed_invariant() {
        let attractors = [a(100, 50, 5), a(1000, 100, 10)];
        let r1 = draw_attractor(1050, &attractors, &seed(1)).unwrap();
        let r2 = draw_attractor(1050, &attractors, &seed(99)).unwrap();
        assert_eq!(r1.selected_center, r2.selected_center);
        assert_eq!(r1.drift, r2.drift);
    }

    // ── out-of-basin (V2 fallback) ───────────────────────────────

    #[test]
    fn out_of_basin_uses_seed() {
        // 500 is in no basin (basins at 100±10 and 1000±100).
        let attractors = [a(100, 10, 5), a(1000, 100, 10)];
        let r1 = draw_attractor(500, &attractors, &seed(1)).unwrap();
        let r2 = draw_attractor(500, &attractors, &seed(2)).unwrap();
        assert!(r1.used_fallback);
        assert!(r2.used_fallback);
        // Different seeds should sometimes pick different attractors.
        // Aggregate over multiple seeds and assert both indices appear.
        let mut seen_idx: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for s in 0u8..40 {
            let r = draw_attractor(500, &attractors, &seed(s)).unwrap();
            seen_idx.insert(r.selected_index);
            if seen_idx.len() == 2 {
                break;
            }
        }
        assert_eq!(seen_idx.len(), 2, "fallback should sample both attractors over varying seeds");
    }

    #[test]
    fn out_of_basin_is_validator_deterministic() {
        let attractors = [a(100, 10, 5), a(1000, 100, 10)];
        let r1 = draw_attractor(500, &attractors, &seed(7)).unwrap();
        let r2 = draw_attractor(500, &attractors, &seed(7)).unwrap();
        assert_eq!(r1, r2);
    }

    #[test]
    fn out_of_basin_weights_favor_closer_attractor() {
        // 110 is in no basin (basins at 100±5 and 1000±100).
        // Distance to centre 100 = 10, distance to centre 1000 = 890.
        // Inverse weights: ≈ 2³²/11 vs ≈ 2³²/891 — heavy favouritism
        // for centre 100. Across many seeds we should see ≥80% pick 100.
        let attractors = [a(100, 5, 1), a(1000, 100, 10)];
        let mut count_100 = 0u32;
        let mut total = 0u32;
        for s in 0u8..100 {
            let r = draw_attractor(110, &attractors, &seed(s)).unwrap();
            if r.selected_center == 100 {
                count_100 += 1;
            }
            total += 1;
        }
        assert!(
            count_100 * 100 >= total * 80,
            "closer attractor should win ≥80% of the time, got {count_100}/{total}"
        );
    }

    // ── drift ────────────────────────────────────────────────────

    #[test]
    fn drift_points_toward_center_when_state_below() {
        let attractors = [a(1000, 100, 50)];
        let r = draw_attractor(900, &attractors, &seed(0)).unwrap();
        assert_eq!(r.selected_center, 1000);
        assert_eq!(r.drift, 50);
    }

    #[test]
    fn drift_points_toward_center_when_state_above() {
        let attractors = [a(1000, 100, 50)];
        let r = draw_attractor(1100, &attractors, &seed(0)).unwrap();
        assert_eq!(r.selected_center, 1000);
        assert_eq!(r.drift, -50);
    }

    #[test]
    fn drift_clamped_to_distance_when_close_to_center() {
        // distance = 10, drift_rate = 50 → drift = 10, not 50.
        let attractors = [a(1000, 100, 50)];
        let r = draw_attractor(990, &attractors, &seed(0)).unwrap();
        assert_eq!(r.drift, 10);
    }

    #[test]
    fn drift_is_zero_at_center() {
        let attractors = [a(1000, 100, 50)];
        let r = draw_attractor(1000, &attractors, &seed(0)).unwrap();
        assert_eq!(r.drift, 0);
    }

    #[test]
    fn drift_zero_when_drift_rate_zero() {
        // basin_radius > 0 so attractor isn't degenerate.
        let attractors = [a(1000, 100, 0)];
        let r = draw_attractor(1100, &attractors, &seed(0)).unwrap();
        assert_eq!(r.drift, 0);
    }

    // ── lyapunov property ────────────────────────────────────────

    #[test]
    fn drift_strictly_decreases_distance_to_center_until_arrival() {
        let attractors = [a(1000, 100, 17)];
        let mut state: Energy = 700;
        let mut prev_distance = state.abs_diff(1000);
        for _ in 0..100 {
            let r = draw_attractor(state, &attractors, &seed(0)).unwrap();
            // Apply drift.
            let new_state_signed = state as i128 + r.drift;
            assert!(new_state_signed >= 0);
            state = new_state_signed as Energy;
            let new_distance = state.abs_diff(1000);
            // Distance must monotonically decrease to 0.
            assert!(new_distance <= prev_distance);
            prev_distance = new_distance;
            if state == 1000 {
                break;
            }
        }
        assert_eq!(state, 1000);
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Singh Attractor V2 anchors fallback selection to
        // a Bell-Certified Beacon seed. In-basin selection unchanged
        // from V1; out-of-basin fallback uses inverse-distance
        // weighted sampling seeded by the certificate, preventing
        // validators from predicting which attractor the chain
        // falls back to before the certificate is published. V2
        // also returns a Lyapunov-stable drift bounded by drift_rate."

        let attractors = [a(100, 10, 5), a(1000, 100, 10)];

        // (1) In-basin: seed-invariant.
        let r_in1 = draw_attractor(1050, &attractors, &seed(1)).unwrap();
        let r_in2 = draw_attractor(1050, &attractors, &seed(255)).unwrap();
        assert_eq!(r_in1.selected_center, r_in2.selected_center);
        assert!(!r_in1.used_fallback);

        // (2) Out-of-basin: seed-dependent (different seeds CAN pick
        // different attractors).
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for s in 0u8..40 {
            let r = draw_attractor(500, &attractors, &seed(s)).unwrap();
            assert!(r.used_fallback);
            seen.insert(r.selected_index);
        }
        assert!(seen.len() >= 2);

        // (3) Drift is Lyapunov-stable (decreases distance to centre).
        let r = draw_attractor(900, &[a(1000, 100, 50)], &seed(0)).unwrap();
        let new_state = (900i128 + r.drift) as Energy;
        assert!(new_state.abs_diff(1000) < 900u64.abs_diff(1000));
    }
}
