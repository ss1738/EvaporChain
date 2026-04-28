//! `path_energy` and `path_caliber` — the per-trajectory quantities
//! the MCC fork-choice picks over.

use evaporchain_cfm::boltzmann_weight;
use evaporchain_light_cone::LightCone;
use evaporchain_types::Energy;

use crate::trajectory::Trajectory;

/// Sum of seed energies along `traj`. Missing blocks (absent from
/// `lc`) contribute 0 — caller can detect such trajectories by
/// checking `lc.contains(id)` separately if strictness is required.
pub fn path_energy(traj: &Trajectory, lc: &LightCone) -> Energy {
    let mut total: u128 = 0;
    for id in traj.iter() {
        if let Some(b) = lc.get(id) {
            total = total.saturating_add(b.energy as u128);
        }
    }
    total.min(Energy::MAX as u128) as Energy
}

/// Path caliber = `Boltzmann(E_path, β)`. Higher caliber = more
/// favoured by MCC fork-choice. Same shift-based exp approximation as
/// `evaporchain-cfm::weight` so the temperature scale lines up across
/// the fee-market / fork-choice family.
pub fn path_caliber(traj: &Trajectory, lc: &LightCone, beta_mb: u64) -> u64 {
    let e = path_energy(traj, lc);
    boltzmann_weight(e, beta_mb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use evaporchain_light_cone::Block;

    fn id(b: u8) -> BlockId_ {
        [b; 32]
    }

    type BlockId_ = [u8; 32];

    fn line() -> LightCone {
        let mut lc = LightCone::new();
        lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
        lc.insert(Block::new(id(1), vec![id(0)], 700, 1)).unwrap();
        lc.insert(Block::new(id(2), vec![id(1)], 400, 2)).unwrap();
        lc
    }

    #[test]
    fn path_energy_sums_seeds() {
        let lc = line();
        let traj = Trajectory::new(vec![id(0), id(1), id(2)]);
        assert_eq!(path_energy(&traj, &lc), 1000 + 700 + 400);
    }

    #[test]
    fn empty_trajectory_zero_energy() {
        let lc = line();
        let traj = Trajectory::default();
        assert_eq!(path_energy(&traj, &lc), 0);
    }

    #[test]
    fn missing_block_contributes_zero() {
        let lc = line();
        let traj = Trajectory::new(vec![id(0), id(99)]);
        assert_eq!(path_energy(&traj, &lc), 1000);
    }

    #[test]
    fn higher_energy_path_lower_caliber() {
        // Boltzmann is monotone-decreasing in energy at positive β —
        // so a "heavier" path has *lower* caliber. MCC then picks the
        // *lowest-energy* trajectory at high β, which corresponds to
        // the closest-to-equilibrium (least dissipation) option.
        let lc = line();
        let beta_mb = 1_000;
        let short = Trajectory::new(vec![id(0)]);
        let long = Trajectory::new(vec![id(0), id(1), id(2)]);
        let c_short = path_caliber(&short, &lc, beta_mb);
        let c_long = path_caliber(&long, &lc, beta_mb);
        assert!(c_short >= c_long);
    }

    #[test]
    fn beta_zero_caliber_is_max() {
        let lc = line();
        let traj = Trajectory::new(vec![id(0), id(1)]);
        // β=0 → Boltzmann = MAX_WEIGHT regardless of energy.
        let c = path_caliber(&traj, &lc, 0);
        assert_eq!(c, 1u64 << 32);
    }
}
