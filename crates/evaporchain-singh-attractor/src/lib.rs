//! Singh Attractor Consensus (Tier 2).
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
