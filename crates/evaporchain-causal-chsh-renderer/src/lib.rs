//! Lane O.5 — markdown renderer for the Causal-CHSH sweep report.
//!
//! Closes the operator-facing side of the stack: takes a
//! `SweepReport` from Lane O.4 and produces a `GATE_RESULT.md`-shaped
//! string. Pure string output; no I/O.
//!
//! Layout:
//! ```text
//! # Causal-CHSH Gate Result
//!
//! ## Summary
//!
//! - S_honest = 0.0000
//! - thresholds = {honest_ceiling=1.80, cartel_floor=2.20, min_gap=0.40}
//! - rows = 33 (3 models × 11 grid points)
//!
//! ## Coordinated-subset
//!
//! | num/den | S_honest | S_cartel | gap | verdict |
//! |---:|---:|---:|---:|:---|
//! | 0/10 | 0.0000 | 0.0000 | 0.0000 | FAIL: gap (0.0000) ≤ min_gap (0.40)…|
//! | 1/10 | … |
//! …
//! - detection floor: 4/10
//! - discrimination ceiling: 3/10
//!
//! ## Biased-coin
//! …
//!
//! ## PR-box approximation
//! …
//! ```

pub mod render;

pub use render::{render_sweep_report, RenderOptions};
