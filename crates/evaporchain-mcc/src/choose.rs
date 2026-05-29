//! `mcc_choose` — argmax-caliber selection over candidate forks.
//!
//! Selection order (deterministic across replicas, no extra rounds):
//!   1. **higher caliber** wins;
//!   2. on a caliber tie — **lower path-energy** wins;
//!   3. on an energy tie too — **smaller `head()` block id** wins.
//!
//! Rule 2 (`#461`, energy-first) matters under *caliber saturation*:
//! caliber is a shift-quantised `Boltzmann(E, β)`, so two paths with
//! distinct energies can collapse to the *same* caliber (quantisation,
//! or the floor where both shifts ≥ 32 → caliber 0). MaxCaliber's whole
//! intent is to pick the lowest-energy / least-dissipation path; under
//! saturation the quantisation hides that, and a pure id tie-break could
//! pick the *heavier* path. Comparing `path_energy` first restores the
//! doctrine where saturation defeats it. Rule 3 keeps it deterministic
//! when energies are exactly equal.
//!
//! This order MUST match `MccForkChoice::select_tip` /
//! `enumerate_with_caliber` (`evaporchain-consensus/src/fork_choice.rs`):
//! `select_tip` chooses the proposer's build tip while `mcc_choose` (via
//! `evaluate`) accepts/rejects a peer's block — a mismatch would let
//! validators reject a correctly-proposed tip (the `#461` liveness
//! hazard). The id direction was aligned 2026-05-22 (smaller-id, matches
//! `select_tip`); the energy-first refinement was added for `#461` and
//! applied to both seams together.

use thiserror::Error;

use evaporchain_light_cone::LightCone;
use evaporchain_types::Energy;

use crate::caliber::{path_caliber, path_energy};
use crate::trajectory::Trajectory;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum McccError {
    #[error("no candidate trajectories supplied")]
    NoCandidates,
}

/// Pick the trajectory with the maximum caliber. Deterministic
/// tie-break: caliber desc, then path-energy asc, then head-block id
/// asc (matches `MccForkChoice::select_tip` / `enumerate_with_caliber`).
pub fn mcc_choose<'a, I>(
    forks: I,
    lc: &LightCone,
    beta_mb: u64,
) -> Result<&'a Trajectory, McccError>
where
    I: IntoIterator<Item = &'a Trajectory>,
{
    // best = (trajectory, caliber, path-energy)
    let mut best: Option<(&Trajectory, u64, Energy)> = None;
    for t in forks {
        let e = path_energy(t, lc);
        let c = path_caliber(t, lc, beta_mb);
        let replace = match best {
            None => true,
            Some((prev_t, prev_c, prev_e)) => {
                if c != prev_c {
                    c > prev_c // higher caliber wins
                } else if e != prev_e {
                    e < prev_e // caliber tie → lower energy wins (#461)
                } else {
                    // energy tie too → smaller head id (determinism)
                    let prev_head = prev_t.head().copied().unwrap_or([0u8; 32]);
                    let new_head = t.head().copied().unwrap_or([0u8; 32]);
                    new_head < prev_head
                }
            }
        };
        if replace {
            best = Some((t, c, e));
        }
    }
    best.map(|(t, _, _)| t).ok_or(McccError::NoCandidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_light_cone::Block;

    fn id(b: u8) -> [u8; 32] {
        [b; 32]
    }

    fn lc_with_two_forks() -> LightCone {
        // Genesis → A; Genesis → B; both A and B as siblings.
        // Then A → A2; B → B2.
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 700, 1)).unwrap(); // A
        lc.insert(Block::new(id(2), vec![id(0)], 700, 1)).unwrap(); // B
        lc.insert(Block::new(id(3), vec![id(1)], 400, 2)).unwrap(); // A2
        lc.insert(Block::new(id(4), vec![id(2)], 600, 2)).unwrap(); // B2 (heavier)
        lc
    }

    #[test]
    fn no_candidates_errs() {
        let lc = LightCone::new();
        let v: Vec<&Trajectory> = vec![];
        let err = mcc_choose(v, &lc, 1_000).unwrap_err();
        assert_eq!(err, McccError::NoCandidates);
    }

    #[test]
    fn picks_lower_energy_path_at_positive_beta() {
        let lc = lc_with_two_forks();
        let path_a = Trajectory::new(vec![id(0), id(1), id(3)]); // sum=2100
        let path_b = Trajectory::new(vec![id(0), id(2), id(4)]); // sum=2300
                                                                 // β chosen so shift_a (= β·2100/1_000_000) and
                                                                 // shift_b (= β·2300/1_000_000) are distinct AND both fit
                                                                 // inside the 32-bit caliber headroom. β = 10_000 →
                                                                 // shift_a=21, shift_b=23 → caliber_a=2^11, caliber_b=2^9
                                                                 // → a wins. (Was β=10 under the millibits scale; Layer 0
                                                                 // item 5 moved CFM β to microbits.)
        let chosen = mcc_choose(vec![&path_a, &path_b], &lc, 10_000).unwrap();
        assert_eq!(chosen, &path_a);
    }

    #[test]
    fn beta_zero_tie_resolved_by_lower_energy() {
        let lc = lc_with_two_forks();
        // β=0 → both paths share the (max) caliber → energy-first
        // tie-break. path_a energy 2100 < path_b energy 2300 → a wins.
        let path_a = Trajectory::new(vec![id(0), id(1), id(3)]);
        let path_b = Trajectory::new(vec![id(0), id(2), id(4)]);
        let chosen = mcc_choose(vec![&path_a, &path_b], &lc, 0).unwrap();
        assert_eq!(chosen, &path_a);
    }

    #[test]
    fn tie_break_independent_of_input_order() {
        // The energy-first winner must not depend on input order — both
        // orderings pick the lower-energy path_a (2100 < 2300).
        let lc = lc_with_two_forks();
        let path_a = Trajectory::new(vec![id(0), id(1), id(3)]);
        let path_b = Trajectory::new(vec![id(0), id(2), id(4)]);
        let ab = mcc_choose(vec![&path_a, &path_b], &lc, 0).unwrap();
        let ba = mcc_choose(vec![&path_b, &path_a], &lc, 0).unwrap();
        assert_eq!(ab, &path_a);
        assert_eq!(ba, &path_a);
    }

    /// #461 — under a caliber tie, LOWER ENERGY must beat SMALLER ID.
    /// Build a fork where the lower-energy path has the *larger* head id,
    /// so the old id-only rule would pick the wrong (heavier) path.
    #[test]
    fn energy_first_beats_smaller_id_on_caliber_tie() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 0, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 900, 1)).unwrap(); // smaller id, HEAVY
        lc.insert(Block::new(id(2), vec![id(0)], 100, 1)).unwrap(); // larger id, LIGHT
        let heavy = Trajectory::new(vec![id(0), id(1)]); // energy 900, head id(1)
        let light = Trajectory::new(vec![id(0), id(2)]); // energy 100, head id(2)
        // β=0 → caliber tie. Energy-first must pick `light` (head id(2)),
        // i.e. the LARGER id — proving energy wins over id.
        let chosen = mcc_choose(vec![&heavy, &light], &lc, 0).unwrap();
        assert_eq!(chosen, &light);
        // order-independent.
        let chosen2 = mcc_choose(vec![&light, &heavy], &lc, 0).unwrap();
        assert_eq!(chosen2, &light);
    }

    /// When caliber AND energy both tie, fall back to smaller head id.
    #[test]
    fn id_fallback_when_caliber_and_energy_tie() {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 0, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 500, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(0)], 500, 1)).unwrap();
        let p1 = Trajectory::new(vec![id(0), id(1)]); // energy 500, head id(1)
        let p2 = Trajectory::new(vec![id(0), id(2)]); // energy 500, head id(2)
        // β=0 caliber tie + equal energy → smaller id (id(1)) wins.
        let chosen = mcc_choose(vec![&p2, &p1], &lc, 0).unwrap();
        assert_eq!(chosen, &p1);
    }

    #[test]
    fn singleton_input_returns_that_trajectory() {
        let lc = lc_with_two_forks();
        let only = Trajectory::new(vec![id(0), id(1)]);
        let chosen = mcc_choose(vec![&only], &lc, 1_000).unwrap();
        assert_eq!(chosen, &only);
    }
}
