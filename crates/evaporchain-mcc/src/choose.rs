//! `mcc_choose` — argmax-caliber selection over candidate forks.
//!
//! Tie-breaks deterministically: when multiple trajectories share the
//! top caliber, the one with the larger `head()` block id (in
//! lexicographic order) wins. This keeps the fork-choice deterministic
//! across replicas without needing additional consensus rounds.

use thiserror::Error;

use evaporchain_light_cone::LightCone;

use crate::caliber::path_caliber;
use crate::trajectory::Trajectory;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum McccError {
    #[error("no candidate trajectories supplied")]
    NoCandidates,
}

/// Pick the trajectory with the maximum caliber. Deterministic tie-
/// break by head-block id (lexicographic, larger wins).
pub fn mcc_choose<'a, I>(
    forks: I,
    lc: &LightCone,
    beta_mb: u64,
) -> Result<&'a Trajectory, McccError>
where
    I: IntoIterator<Item = &'a Trajectory>,
{
    let mut best: Option<(&Trajectory, u64)> = None;
    for t in forks {
        let c = path_caliber(t, lc, beta_mb);
        match best {
            None => best = Some((t, c)),
            Some((_, prev_c)) if c > prev_c => best = Some((t, c)),
            Some((prev_t, prev_c)) if c == prev_c => {
                // Tie-break by head id, lexicographic larger.
                let prev_head = prev_t.head().copied().unwrap_or([0u8; 32]);
                let new_head = t.head().copied().unwrap_or([0u8; 32]);
                if new_head > prev_head {
                    best = Some((t, c));
                }
            }
            _ => {}
        }
    }
    best.map(|(t, _)| t).ok_or(McccError::NoCandidates)
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
    fn beta_zero_ties_resolved_lexicographically() {
        let lc = lc_with_two_forks();
        // β=0 → both paths have equal caliber → tie-break by head id.
        let path_a = Trajectory::new(vec![id(0), id(1), id(3)]);
        let path_b = Trajectory::new(vec![id(0), id(2), id(4)]);
        let chosen = mcc_choose(vec![&path_a, &path_b], &lc, 0).unwrap();
        // head(b) = id(4) > head(a) = id(3) → b wins the tie.
        assert_eq!(chosen, &path_b);
    }

    #[test]
    fn singleton_input_returns_that_trajectory() {
        let lc = lc_with_two_forks();
        let only = Trajectory::new(vec![id(0), id(1)]);
        let chosen = mcc_choose(vec![&only], &lc, 1_000).unwrap();
        assert_eq!(chosen, &only);
    }
}
