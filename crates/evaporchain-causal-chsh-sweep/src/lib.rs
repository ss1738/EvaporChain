//! Lane O.4 — parameter-sweep harness for Causal-CHSH cartel models.
//!
//! Lane O.3 ships three cartel models with a single rational parameter
//! each (subset fraction k/n, coin bias p, PR-box visibility v). The
//! gate's doctrine thresholds {1.8 / 2.2 / 0.4} discriminate honest
//! from cartel — but only above some critical parameter strength.
//!
//! This lane drives each cartel model across a rational grid and
//! returns a `SweepReport` with per-cell {S_honest, S_cartel, gap,
//! verdict, fail_reasons}. Operators read the report to find:
//!
//! - **Detection floor**: smallest parameter value where the gate
//!   passes (cartel S clears 2.2 with gap > 0.4).
//! - **Discrimination ceiling**: largest parameter value where the
//!   gate still fails (e.g., bias-coin at p=0 → output is honest;
//!   gate fails because cartel ≡ honest).
//!
//! Single sweep call covers all three models; same honest sample is
//! reused so all rows are comparable.

pub mod sweep;

pub use sweep::{
    run_sweep, CartelKind, GridPoint, SweepCell, SweepError, SweepGrid, SweepReport, SweepRow,
};
