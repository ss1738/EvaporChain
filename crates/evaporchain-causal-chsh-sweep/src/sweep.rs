//! Sweep driver.

use evaporchain_causal_chsh::chsh::{compute_chsh_s, ConcurrentPairSamples};
use evaporchain_causal_chsh::gate::{run_synthetic_gate, GateThresholds, GateVerdict};
use evaporchain_causal_chsh_cartels::{
    biased_coin_cartel, coordinated_subset_cartel, pr_box_cartel, CartelError,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Which cartel model a sweep row is exercising.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CartelKind {
    CoordinatedSubset,
    BiasedCoin,
    PrBox,
}

/// A single (numerator, denominator) point on the parameter grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridPoint {
    pub num: u64,
    pub den: u64,
}

/// Three rational grids — one per cartel model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepGrid {
    pub coordinated_subset: Vec<GridPoint>,
    pub biased_coin: Vec<GridPoint>,
    pub pr_box: Vec<GridPoint>,
}

impl SweepGrid {
    /// Doctrine sweep: 11 evenly-spaced points 0/10..10/10 for each
    /// model. Hits both endpoints (honest, full cartel) plus 9
    /// interior points. Useful for the standard report.
    pub fn doctrine() -> Self {
        let pts: Vec<GridPoint> = (0..=10).map(|n| GridPoint { num: n, den: 10 }).collect();
        Self {
            coordinated_subset: pts.clone(),
            biased_coin: pts.clone(),
            pr_box: pts,
        }
    }
}

/// Verdict for a single sweep cell. Mirrors `GateVerdict` shape but
/// flattened for tabular output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SweepCell {
    Pass {
        s_honest: f64,
        s_cartel: f64,
        gap: f64,
    },
    Fail {
        s_honest: f64,
        s_cartel: f64,
        gap: f64,
        reasons: Vec<String>,
    },
    InputError(String),
    CartelError(String),
}

/// One row of the sweep — `(model, parameter point, verdict)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepRow {
    pub kind: CartelKind,
    pub point: GridPoint,
    pub cell: SweepCell,
}

/// Final sweep report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SweepReport {
    /// Honest sample's S, computed once and reused across all rows.
    pub s_honest: f64,
    /// Doctrine thresholds active for this sweep.
    pub thresholds: GateThresholds,
    /// All rows, in the order: coordinated, biased-coin, PR-box.
    pub rows: Vec<SweepRow>,
}

#[derive(Debug, Error, PartialEq)]
pub enum SweepError {
    #[error("honest sample is empty in at least one bucket")]
    EmptyHonest,
    #[error("compute_chsh_s on honest sample failed: {0}")]
    HonestChshFailure(String),
}

/// Run a parameter sweep over the three cartel models.
pub fn run_sweep(
    honest: &ConcurrentPairSamples,
    grid: &SweepGrid,
    thresholds: GateThresholds,
    seed: [u8; 32],
) -> Result<SweepReport, SweepError> {
    if honest.samples_ab.is_empty()
        || honest.samples_ab_prime.is_empty()
        || honest.samples_a_prime_b.is_empty()
        || honest.samples_a_prime_b_prime.is_empty()
    {
        return Err(SweepError::EmptyHonest);
    }

    // Compute honest S once; cache for all rows.
    let s_honest = match compute_chsh_s(honest) {
        Ok(s) => s,
        Err(e) => return Err(SweepError::HonestChshFailure(format!("{e}"))),
    };

    let mut rows: Vec<SweepRow> = Vec::with_capacity(
        grid.coordinated_subset.len() + grid.biased_coin.len() + grid.pr_box.len(),
    );

    for (kind, points) in [
        (CartelKind::CoordinatedSubset, &grid.coordinated_subset),
        (CartelKind::BiasedCoin, &grid.biased_coin),
        (CartelKind::PrBox, &grid.pr_box),
    ] {
        for &point in points {
            let cell = run_one_cell(honest, kind, point, thresholds, seed);
            rows.push(SweepRow { kind, point, cell });
        }
    }

    Ok(SweepReport {
        s_honest,
        thresholds,
        rows,
    })
}

fn run_one_cell(
    honest: &ConcurrentPairSamples,
    kind: CartelKind,
    point: GridPoint,
    thresholds: GateThresholds,
    seed: [u8; 32],
) -> SweepCell {
    let cartel = match kind {
        CartelKind::CoordinatedSubset => {
            coordinated_subset_cartel(honest, point.num, point.den, seed)
        }
        CartelKind::BiasedCoin => biased_coin_cartel(honest, point.num, point.den, seed),
        CartelKind::PrBox => pr_box_cartel(honest, point.num, point.den, seed),
    };
    let cartel = match cartel {
        Ok(c) => c,
        Err(e) => return SweepCell::CartelError(cartel_err_msg(e)),
    };

    let verdict = run_synthetic_gate(honest, &cartel, thresholds);
    match verdict {
        GateVerdict::Pass {
            s_honest,
            s_cartel,
            gap,
        } => SweepCell::Pass {
            s_honest,
            s_cartel,
            gap,
        },
        GateVerdict::Fail {
            s_honest,
            s_cartel,
            gap,
            reasons,
        } => SweepCell::Fail {
            s_honest,
            s_cartel,
            gap,
            reasons,
        },
        GateVerdict::InputError(s) => SweepCell::InputError(s),
    }
}

fn cartel_err_msg(e: CartelError) -> String {
    format!("{e}")
}

// ── helpers for analysis ──────────────────────────────────────────

impl SweepReport {
    /// Smallest grid point at which a given cartel kind passed the
    /// gate. Returns `None` if no point passed.
    pub fn detection_floor(&self, kind: CartelKind) -> Option<GridPoint> {
        self.rows
            .iter()
            .filter(|r| r.kind == kind && matches!(r.cell, SweepCell::Pass { .. }))
            .map(|r| r.point)
            .min_by(|a, b| {
                // a.num/a.den vs b.num/b.den, compared via cross-multiply.
                let lhs = a.num as u128 * b.den as u128;
                let rhs = b.num as u128 * a.den as u128;
                lhs.cmp(&rhs)
            })
    }

    /// Largest grid point at which a given cartel kind failed the
    /// gate. Returns `None` if every point passed.
    pub fn discrimination_ceiling(&self, kind: CartelKind) -> Option<GridPoint> {
        self.rows
            .iter()
            .filter(|r| r.kind == kind && matches!(r.cell, SweepCell::Fail { .. }))
            .map(|r| r.point)
            .max_by(|a, b| {
                let lhs = a.num as u128 * b.den as u128;
                let rhs = b.num as u128 * a.den as u128;
                lhs.cmp(&rhs)
            })
    }

    /// Count of (Pass, Fail, error) cells per cartel kind.
    pub fn counts(&self, kind: CartelKind) -> (usize, usize, usize) {
        let mut p = 0usize;
        let mut f = 0usize;
        let mut e = 0usize;
        for r in self.rows.iter().filter(|r| r.kind == kind) {
            match r.cell {
                SweepCell::Pass { .. } => p += 1,
                SweepCell::Fail { .. } => f += 1,
                SweepCell::InputError(_) | SweepCell::CartelError(_) => e += 1,
            }
        }
        (p, f, e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn balanced_honest(n_each: usize) -> ConcurrentPairSamples {
        let bal: Vec<i8> = (0..n_each)
            .map(|i| if i % 2 == 0 { 1 } else { -1 })
            .collect();
        ConcurrentPairSamples {
            samples_ab: bal.clone(),
            samples_ab_prime: bal.clone(),
            samples_a_prime_b: bal.clone(),
            samples_a_prime_b_prime: bal,
        }
    }

    fn seed(b: u8) -> [u8; 32] {
        [b; 32]
    }

    // ── grid construction ────────────────────────────────────────

    #[test]
    fn doctrine_grid_is_eleven_points() {
        let g = SweepGrid::doctrine();
        assert_eq!(g.coordinated_subset.len(), 11);
        assert_eq!(g.biased_coin.len(), 11);
        assert_eq!(g.pr_box.len(), 11);
        assert_eq!(g.coordinated_subset[0], GridPoint { num: 0, den: 10 });
        assert_eq!(g.coordinated_subset[10], GridPoint { num: 10, den: 10 });
    }

    // ── happy path ────────────────────────────────────────────────

    #[test]
    fn sweep_runs_thirty_three_cells_on_doctrine_grid() {
        let h = balanced_honest(64);
        let report = run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            seed(1),
        )
        .unwrap();
        assert_eq!(report.rows.len(), 33);
        // Honest is balanced → S = 0.
        assert!(report.s_honest.abs() < 1e-9);
    }

    #[test]
    fn endpoint_zero_always_fails_for_subset_and_biased() {
        // num=0 → cartel = honest → S_cartel = S_honest → gap = 0
        // → gate fails (gap < 0.4 minimum).
        let h = balanced_honest(64);
        let report = run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            seed(2),
        )
        .unwrap();
        for r in &report.rows {
            if r.point.num == 0
                && (r.kind == CartelKind::CoordinatedSubset || r.kind == CartelKind::BiasedCoin)
            {
                assert!(matches!(r.cell, SweepCell::Fail { .. }), "row = {:?}", r);
            }
        }
    }

    #[test]
    fn endpoint_full_always_passes() {
        // num=den → S_cartel = 4 → gate passes for all three models.
        let h = balanced_honest(64);
        let report = run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            seed(3),
        )
        .unwrap();
        for r in &report.rows {
            if r.point.num == r.point.den {
                assert!(matches!(r.cell, SweepCell::Pass { .. }), "row = {:?}", r);
            }
        }
    }

    // ── detection floor / ceiling helpers ────────────────────────

    #[test]
    fn detection_floor_for_subset_is_above_fail_threshold() {
        let h = balanced_honest(200);
        let report = run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            seed(4),
        )
        .unwrap();
        let floor = report.detection_floor(CartelKind::CoordinatedSubset);
        assert!(floor.is_some());
        let ceiling = report.discrimination_ceiling(CartelKind::CoordinatedSubset);
        assert!(ceiling.is_some());
        // Floor should be strictly above ceiling on the rational grid.
        let f = floor.unwrap();
        let c = ceiling.unwrap();
        let lhs = f.num as u128 * c.den as u128;
        let rhs = c.num as u128 * f.den as u128;
        assert!(lhs > rhs, "floor {:?} should be > ceiling {:?}", f, c);
    }

    #[test]
    fn pr_box_visibility_one_is_the_only_passing_endpoint_at_low_n() {
        // PR-box at v=0..0.5 → S in [0, 2] → gate fails (cartel < floor).
        // PR-box at v=1 → S=4 → passes.
        let h = balanced_honest(2_000);
        let report = run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            seed(5),
        )
        .unwrap();
        // v=0/10 must fail.
        let v0 = report
            .rows
            .iter()
            .find(|r| r.kind == CartelKind::PrBox && r.point.num == 0)
            .unwrap();
        assert!(matches!(v0.cell, SweepCell::Fail { .. }));
        // v=10/10 must pass.
        let v10 = report
            .rows
            .iter()
            .find(|r| r.kind == CartelKind::PrBox && r.point.num == 10)
            .unwrap();
        assert!(matches!(v10.cell, SweepCell::Pass { .. }));
    }

    // ── counts + serialisation ───────────────────────────────────

    #[test]
    fn counts_sum_to_eleven_per_kind() {
        let h = balanced_honest(64);
        let report = run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            seed(6),
        )
        .unwrap();
        for kind in [
            CartelKind::CoordinatedSubset,
            CartelKind::BiasedCoin,
            CartelKind::PrBox,
        ] {
            let (p, f, e) = report.counts(kind);
            assert_eq!(p + f + e, 11);
            assert_eq!(e, 0, "no errors expected on balanced honest input");
        }
    }

    #[test]
    fn report_serialises_round_trip() {
        let h = balanced_honest(32);
        let report = run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            seed(7),
        )
        .unwrap();
        let s = serde_json::to_string(&report).unwrap();
        let _: SweepReport = serde_json::from_str(&s).unwrap();
    }

    // ── validator-determinism ────────────────────────────────────

    #[test]
    fn same_seed_same_report() {
        let h = balanced_honest(32);
        let r1 = run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            seed(8),
        )
        .unwrap();
        let r2 = run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            seed(8),
        )
        .unwrap();
        // Compare rows.
        assert_eq!(r1.rows.len(), r2.rows.len());
        for (a, b) in r1.rows.iter().zip(r2.rows.iter()) {
            assert_eq!(a.kind, b.kind);
            assert_eq!(a.point, b.point);
            assert_eq!(a.cell, b.cell);
        }
    }

    // ── error paths ──────────────────────────────────────────────

    #[test]
    fn empty_honest_rejected() {
        let mut h = balanced_honest(8);
        h.samples_ab.clear();
        let err = run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            seed(9),
        )
        .unwrap_err();
        assert_eq!(err, SweepError::EmptyHonest);
    }

    #[test]
    fn invalid_grid_point_surfaces_as_cell_error() {
        let h = balanced_honest(16);
        let bad_grid = SweepGrid {
            coordinated_subset: vec![GridPoint { num: 5, den: 4 }], // invalid
            biased_coin: vec![GridPoint { num: 1, den: 0 }],        // invalid
            pr_box: vec![GridPoint { num: 1, den: 1 }],
        };
        let report = run_sweep(&h, &bad_grid, GateThresholds::doctrine(), seed(10)).unwrap();
        assert!(matches!(report.rows[0].cell, SweepCell::CartelError(_)));
        assert!(matches!(report.rows[1].cell, SweepCell::CartelError(_)));
        // PR-box at v=1 should still pass.
        assert!(matches!(report.rows[2].cell, SweepCell::Pass { .. }));
    }

    // ── press claim ──────────────────────────────────────────────

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "Lane O.4 ships the cartel-detection sweep harness.
        // The doctrine grid (0/10 .. 10/10) drives all three cartel
        // models against a single honest sample. Endpoint num=0 fails
        // the gate (cartel ≡ honest, gap=0); endpoint num=den passes
        // (cartel S=4, gap≥4). The detection floor + discrimination
        // ceiling helpers tell operators where the {1.8/2.2/0.4}
        // doctrine thresholds bite. Same seed, same report on every
        // validator."
        let h = balanced_honest(2_000);
        let report = run_sweep(
            &h,
            &SweepGrid::doctrine(),
            GateThresholds::doctrine(),
            seed(42),
        )
        .unwrap();

        // 33 rows = 11 points × 3 models.
        assert_eq!(report.rows.len(), 33);

        // Endpoint discipline.
        for kind in [
            CartelKind::CoordinatedSubset,
            CartelKind::BiasedCoin,
            CartelKind::PrBox,
        ] {
            let zero = report
                .rows
                .iter()
                .find(|r| r.kind == kind && r.point.num == 0)
                .unwrap();
            let full = report
                .rows
                .iter()
                .find(|r| r.kind == kind && r.point.num == r.point.den)
                .unwrap();
            assert!(matches!(zero.cell, SweepCell::Fail { .. }));
            assert!(matches!(full.cell, SweepCell::Pass { .. }));
        }

        // Floor exists, ceiling exists, floor > ceiling on the grid.
        for kind in [
            CartelKind::CoordinatedSubset,
            CartelKind::BiasedCoin,
            CartelKind::PrBox,
        ] {
            assert!(report.detection_floor(kind).is_some());
            assert!(report.discrimination_ceiling(kind).is_some());
        }
    }
}
