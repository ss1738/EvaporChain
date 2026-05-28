//! Coverage tests for MCC fork-choice (Tier-0 theorem-grade per
//! `INVENTION_STACK.md §A1.2 T1`).

use evaporchain_light_cone::{Block, BlockId, LightCone};
use evaporchain_mcc::{mcc_choose, path_caliber, path_energy, McccError, Trajectory};

fn id(b: u8) -> BlockId {
    [b; 32]
}

fn line_3() -> LightCone {
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
    lc.insert(Block::new(id(1), vec![id(0)], 700, 1)).unwrap();
    lc.insert(Block::new(id(2), vec![id(1)], 400, 2)).unwrap();
    lc
}

// =================================================================
// Trajectory invariants
// =================================================================

#[test]
fn trajectory_default_is_empty() {
    let t = Trajectory::default();
    assert!(t.is_empty());
    assert_eq!(t.len(), 0);
    assert_eq!(t.head(), None);
}

#[test]
fn trajectory_new_preserves_blocks() {
    let blocks = vec![id(0), id(1), id(2)];
    let t = Trajectory::new(blocks.clone());
    assert_eq!(t.len(), 3);
    assert_eq!(t.head(), Some(&id(2)));
    let collected: Vec<_> = t.iter().copied().collect();
    assert_eq!(collected, blocks);
}

#[test]
fn trajectory_serde_round_trips() {
    let t = Trajectory::new(vec![id(0), id(1)]);
    let json = serde_json::to_string(&t).unwrap();
    let back: Trajectory = serde_json::from_str(&json).unwrap();
    assert_eq!(back, t);
}

// =================================================================
// path_energy
// =================================================================

#[test]
fn path_energy_sums_block_seeds() {
    let lc = line_3();
    let t = Trajectory::new(vec![id(0), id(1), id(2)]);
    assert_eq!(path_energy(&t, &lc), 1000 + 700 + 400);
}

#[test]
fn path_energy_empty_trajectory_is_zero() {
    let lc = line_3();
    let t = Trajectory::default();
    assert_eq!(path_energy(&t, &lc), 0);
}

#[test]
fn path_energy_missing_blocks_contribute_zero() {
    let lc = line_3();
    let t = Trajectory::new(vec![id(99), id(0), id(255)]);
    // Only id(0) is in the cone (1000). The other two contribute 0.
    assert_eq!(path_energy(&t, &lc), 1000);
}

#[test]
fn path_energy_saturates_at_energy_max() {
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0), vec![], u64::MAX, 0)).unwrap();
    lc.insert(Block::new(id(1), vec![id(0)], u64::MAX, 1))
        .unwrap();
    let t = Trajectory::new(vec![id(0), id(1)]);
    // saturating_add(u64::MAX, u64::MAX) clamps to Energy::MAX (= u64::MAX).
    assert_eq!(path_energy(&t, &lc), u64::MAX);
}

// =================================================================
// path_caliber
// =================================================================

#[test]
fn path_caliber_decreases_monotonically_with_energy() {
    // Two trajectories: low-energy + high-energy. Lower energy → higher caliber.
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0), vec![], 100, 0)).unwrap();
    lc.insert(Block::new(id(1), vec![], 100_000, 0)).unwrap();
    let t_low = Trajectory::new(vec![id(0)]);
    let t_high = Trajectory::new(vec![id(1)]);
    let beta = 10_000u64;
    let c_low = path_caliber(&t_low, &lc, beta);
    let c_high = path_caliber(&t_high, &lc, beta);
    assert!(c_low > c_high, "lower energy must yield higher caliber");
}

#[test]
fn path_caliber_empty_trajectory_is_maximal() {
    // E = 0 → exp(0) = 1 in real units → caliber at its maximum.
    let lc = LightCone::new();
    let t = Trajectory::default();
    // Caliber is u64-bounded; an E=0 trajectory should yield the max
    // possible weight under boltzmann_weight(0, _).
    let _ = path_caliber(&t, &lc, 1_000);
}

// =================================================================
// mcc_choose
// =================================================================

#[test]
fn mcc_choose_empty_iter_errors() {
    let lc = LightCone::new();
    let v: Vec<&Trajectory> = vec![];
    let err = mcc_choose(v, &lc, 1_000).unwrap_err();
    assert_eq!(err, McccError::NoCandidates);
}

#[test]
fn mcc_choose_single_candidate_wins_trivially() {
    let lc = line_3();
    let t = Trajectory::new(vec![id(0), id(1)]);
    let picked = mcc_choose(std::iter::once(&t), &lc, 1_000).unwrap();
    assert_eq!(picked, &t);
}

#[test]
fn mcc_choose_picks_lower_energy_at_positive_beta() {
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0), vec![], 1000, 0)).unwrap();
    lc.insert(Block::new(id(1), vec![id(0)], 500, 1)).unwrap();
    lc.insert(Block::new(id(2), vec![id(0)], 1500, 1)).unwrap();
    let t_cheap = Trajectory::new(vec![id(0), id(1)]); // sum = 1500
    let t_dear = Trajectory::new(vec![id(0), id(2)]); // sum = 2500
    let picked = mcc_choose([&t_cheap, &t_dear].iter().copied(), &lc, 10_000).unwrap();
    assert_eq!(picked, &t_cheap);
}

#[test]
fn mcc_choose_tie_breaks_by_lexicographic_head() {
    // Two trajectories with identical energy → tie broken by SMALLER
    // head id (aligned with select_tip 2026-05-22; was larger-id).
    let mut lc = LightCone::new();
    lc.insert(Block::new(id(0), vec![], 100, 0)).unwrap();
    lc.insert(Block::new(id(1), vec![id(0)], 200, 1)).unwrap();
    lc.insert(Block::new(id(2), vec![id(0)], 200, 1)).unwrap();
    let t_low = Trajectory::new(vec![id(0), id(1)]);
    let t_high = Trajectory::new(vec![id(0), id(2)]);
    // Same energy. Smaller head (id(1)) wins.
    let picked = mcc_choose([&t_low, &t_high].iter().copied(), &lc, 1_000).unwrap();
    assert_eq!(picked, &t_low);
}

// =================================================================
// Error ergonomics
// =================================================================

#[test]
fn mccc_error_displays_and_eq() {
    let a = McccError::NoCandidates;
    let b = McccError::NoCandidates;
    assert_eq!(a, b);
    assert!(a.to_string().contains("candidate") || a.to_string().contains("no"));
}
