//! Singh Attractor Consensus (Tier 2).
//!
//! **Companion: V2.** `evaporchain-singh-attractor-v2` closes the
//! deterministic-fallback grinding gap by drawing the no-basin
//! fallback attractor via energy-distance-weighted sampling against
//! a 32-byte certificate seed (typically from
//! `evaporchain-bell-beacon-v2::BellCertificate.seed`). V1 ships
//! basin-membership + nearest-centre fallback; V2 swaps the
//! deterministic fallback for an unpredictable-until-published one.
//! V1 and V2 are peers, both live.
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Singh Attractor Consensus** — Folds into Singh-Lyapunov Fee
//! > Controller's stability framework.
//!
//! ## What this adds on top of the fee controller
//!
//! `evaporchain-fee-controller` proves a *single* equilibrium V(E) =
//! ½(E−E*)² with monotone drift. Singh Attractor extends this to
//! *multiple* equilibria — each `Attractor { center, basin_radius }`
//! captures a stable operating point and its basin of attraction in
//! the energy state space.
//!
//! Use case: a chain that must support distinct steady-state regimes
//! (e.g. quiet hours vs DEX-volume hours) needs more than one
//! equilibrium. Singh Attractor lets the consensus rule pick the
//! attractor whose basin contains the *current* state and apply
//! `evaporchain-fee-controller::FeeController::step` against that
//! attractor's center.

use serde::{Deserialize, Serialize};

use evaporchain_types::Energy;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attractor {
    pub center: Energy,
    /// Half-width of the basin around `center`. State in `[center -
    /// radius, center + radius]` is "in this basin".
    pub basin_radius: Energy,
}

impl Attractor {
    pub const fn new(center: Energy, basin_radius: Energy) -> Self {
        Self {
            center,
            basin_radius,
        }
    }

    /// True iff `state_energy` is inside this attractor's basin.
    pub fn contains(&self, state_energy: Energy) -> bool {
        let lo = self.center.saturating_sub(self.basin_radius);
        let hi = self.center.saturating_add(self.basin_radius);
        state_energy >= lo && state_energy <= hi
    }
}

/// `select_attractor(state_energy, attractors)` picks the *first*
/// attractor whose basin contains `state_energy`. If none contain
/// it, returns the attractor with center nearest to `state_energy`
/// (graceful fallback — caller can detect the no-basin case by
/// checking `contains` before applying the result).
pub fn select_attractor(state_energy: Energy, attractors: &[Attractor]) -> Option<&Attractor> {
    if attractors.is_empty() {
        return None;
    }
    if let Some(a) = attractors.iter().find(|a| a.contains(state_energy)) {
        return Some(a);
    }
    // Fallback: nearest center.
    attractors
        .iter()
        .min_by_key(|a| a.center.abs_diff(state_energy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_inside_basin() {
        let a = Attractor::new(1000, 100);
        assert!(a.contains(900));
        assert!(a.contains(1000));
        assert!(a.contains(1100));
    }

    #[test]
    fn contains_outside_basin() {
        let a = Attractor::new(1000, 100);
        assert!(!a.contains(800));
        assert!(!a.contains(1101));
    }

    #[test]
    fn select_picks_basin_owner() {
        let attractors = [
            Attractor::new(100, 10),
            Attractor::new(1000, 100),
            Attractor::new(10_000, 1000),
        ];
        let a = select_attractor(1050, &attractors).unwrap();
        assert_eq!(a.center, 1000);
    }

    #[test]
    fn select_falls_back_to_nearest_when_no_basin() {
        let attractors = [Attractor::new(100, 10), Attractor::new(1000, 100)];
        // 500 is not in any basin; nearest center is 1000 (distance
        // 500) vs 100 (distance 400). 100 wins.
        let a = select_attractor(500, &attractors).unwrap();
        assert_eq!(a.center, 100);
    }

    #[test]
    fn select_empty_returns_none() {
        assert!(select_attractor(1000, &[]).is_none());
    }
}

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Singh Attractor V1 generalises the single-
    /// equilibrium Lyapunov fee controller to multi-equilibrium
    /// regime selection. `select_attractor` returns the FIRST basin
    /// containing the state, with deterministic nearest-centre
    /// fallback when none does. Empty attractor list returns None."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let attractors = [
            Attractor::new(100, 10),
            Attractor::new(1_000, 100),
            Attractor::new(10_000, 1_000),
        ];

        // In-basin: 1050 lands inside the 1000±100 basin.
        let a = select_attractor(1_050, &attractors).unwrap();
        assert_eq!(a.center, 1_000);
        assert!(a.contains(1_050));

        // Out-of-basin fallback: 500 has no basin; closer to 100
        // (dist 400) than 1000 (dist 500), so 100 wins.
        let f = select_attractor(500, &attractors).unwrap();
        assert_eq!(f.center, 100);
        assert!(!f.contains(500));

        // Empty list → None.
        assert!(select_attractor(1_000, &[]).is_none());

        // contains is symmetric around centre.
        let c = Attractor::new(1_000, 100);
        assert!(c.contains(900));
        assert!(c.contains(1_100));
        assert!(!c.contains(899));
        assert!(!c.contains(1_101));
    }
}
