//! LAD-mode resource annotations for EvaporScript contracts.
//!
//! This crate provides the **script-lad compiler frontend** that closes the
//! last Tier 1 launch primitive gap identified in INVENTION_STACK.md §4.1
//! row 12.
//!
//! ## What this is
//!
//! EvaporScript contracts can annotate state fields with `@lad(...)` directives
//! that assign a Linear-Affine-Decay resource mode to the field. This crate:
//!
//! 1. **Parses** `@lad(mode=..., value=..., window=...)` annotations from
//!    script source text.
//! 2. **Tracks** the LAD resource lifecycle across a contract call via
//!    `LadResourceTracker`.
//! 3. **Checks** post-call resource state: unconsumed Linear resources are a
//!    compile-time (or pre-submit) invariant violation.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use evaporchain_script_lad::{check_lad_resources, LadCheckResult};
//!
//! let source = r#"
//! @lad(mode=linear, value=1000)
//! let payment: u64 = 0;
//! @lad(mode=decaying, window=50, value=500)
//! let voucher: u64 = 0;
//! "#;
//!
//! let result = check_lad_resources(source, current_epoch)?;
//! // result.verdicts: {"payment": Live(1000), "voucher": Live(500)}
//! // At epoch 100 the voucher would evaporate.
//! ```
//!
//! ## Doctrine position
//!
//! Real Move-style linearity needs compiler-enforced substructural types that
//! would require full integration into the EvaporScript parser and bytecode.
//! This crate ships the **pre-execution analysis layer** that operates on source
//! text and interacts with the LAD-VM runtime. A future `evaporchain-script`
//! integration would lower these annotations into typed bytecode; until then
//! this crate serves as the authoritative check tool for off-chain validators
//! and the `/api/script_lad/check` endpoint.

pub mod annotation;
pub mod error;
pub mod tracker;

pub use annotation::{parse_annotations, LadAnnotation};
pub use error::LadScriptError;
pub use tracker::{LadResourceTracker, ResourceVerdict};

use std::collections::BTreeMap;

/// Result of a LAD resource check pass over a script source.
#[derive(Debug, Clone)]
pub struct LadCheckResult {
    /// Parsed annotations found in the source.
    pub annotations: Vec<LadAnnotation>,
    /// Resource verdicts at `check_epoch`.
    pub verdicts: BTreeMap<String, ResourceVerdict>,
    /// Names of Linear resources that are still live (not consumed) — these
    /// are invariant violations if the contract returns without using them.
    pub unconsumed_linear: Vec<String>,
    /// Names of Decaying resources that evaporated before `check_epoch`.
    pub evaporated: Vec<String>,
}

impl LadCheckResult {
    /// True iff all Linear resources are consumed and no evaporations occurred.
    pub fn is_clean(&self) -> bool {
        self.unconsumed_linear.is_empty() && self.evaporated.is_empty()
    }
}

/// Parse `@lad(...)` annotations from `source` and compute resource verdicts
/// at `check_epoch`. Returns the full check result.
///
/// Resources are initialised at epoch 0 (`created_epoch = 0`). Pass
/// `check_epoch = 0` to get the initial state; pass the actual block epoch
/// to see which resources have evaporated.
pub fn check_lad_resources(source: &str, check_epoch: u64) -> Result<LadCheckResult, LadScriptError> {
    let annotations = parse_annotations(source)?;
    let tracker = LadResourceTracker::from_annotations(&annotations, 0);
    let verdicts = tracker.snapshot(check_epoch);

    let unconsumed_linear: Vec<String> = annotations
        .iter()
        .filter(|ann| {
            ann.mode == evaporchain_lad_vm::Mode::Linear
                && verdicts
                    .get(&ann.field_name)
                    .map(|v| v.is_live())
                    .unwrap_or(false)
        })
        .map(|ann| ann.field_name.clone())
        .collect();

    let evaporated: Vec<String> = verdicts
        .iter()
        .filter(|(_, v)| v.is_evaporated())
        .map(|(name, _)| name.clone())
        .collect();

    Ok(LadCheckResult {
        annotations,
        verdicts,
        unconsumed_linear,
        evaporated,
    })
}

/// Simulate a complete resource lifecycle — pledge, optional use/drop, then
/// tick to `final_epoch` — and return the final verdict map.
///
/// `ops` is a list of `("use" | "drop", field_name, epoch)` tuples executed
/// in order before ticking to `final_epoch`. Unknown ops are silently skipped.
pub fn simulate_lifecycle(
    source: &str,
    created_epoch: u64,
    ops: &[(&str, &str, u64)],
    final_epoch: u64,
) -> Result<BTreeMap<String, ResourceVerdict>, LadScriptError> {
    let annotations = parse_annotations(source)?;
    let mut tracker = LadResourceTracker::from_annotations(&annotations, created_epoch);

    for (op, name, epoch) in ops {
        match *op {
            "use" => {
                let _ = tracker.use_resource(name, *epoch);
            }
            "drop" => {
                let _ = tracker.drop_resource(name);
            }
            _ => {}
        }
    }

    Ok(tracker.tick_all(final_epoch))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINEAR_SRC: &str = "\
@lad(mode=linear, value=1000)\n\
let payment: u64 = 0;";

    const DECAYING_SRC: &str = "\
@lad(mode=decaying, window=50, value=500)\n\
let voucher: u64 = 0;";

    const MULTI_SRC: &str = "\
@lad(mode=linear, value=1000)\n\
let payment: u64 = 0;\n\
@lad(mode=affine, value=200)\n\
let tip: u64 = 0;\n\
@lad(mode=decaying, window=10, value=50)\n\
let voucher: u64 = 0;";

    #[test]
    fn check_linear_shows_unconsumed() {
        let result = check_lad_resources(LINEAR_SRC, 1).unwrap();
        assert_eq!(result.annotations.len(), 1);
        assert_eq!(result.unconsumed_linear, vec!["payment"]);
        assert!(!result.is_clean());
    }

    #[test]
    fn check_decaying_live_before_window() {
        let result = check_lad_resources(DECAYING_SRC, 10).unwrap();
        assert!(result.evaporated.is_empty());
        assert!(result.is_clean());
    }

    #[test]
    fn check_decaying_evaporated_after_window() {
        let result = check_lad_resources(DECAYING_SRC, 60).unwrap();
        assert_eq!(result.evaporated, vec!["voucher"]);
        assert!(!result.is_clean());
    }

    #[test]
    fn simulate_use_clears_linear() {
        let verdicts = simulate_lifecycle(LINEAR_SRC, 0, &[("use", "payment", 1)], 5).unwrap();
        assert_eq!(verdicts["payment"], ResourceVerdict::Consumed);
    }

    #[test]
    fn simulate_drop_affine() {
        let src = "@lad(mode=affine, value=100)\nlet tip: u64 = 0;";
        let verdicts = simulate_lifecycle(src, 0, &[("drop", "tip", 1)], 5).unwrap();
        assert_eq!(verdicts["tip"], ResourceVerdict::Consumed);
    }

    #[test]
    fn simulate_multi_resource() {
        let verdicts = simulate_lifecycle(
            MULTI_SRC,
            0,
            &[("use", "payment", 1), ("drop", "tip", 2)],
            20, // past voucher window=10
        )
        .unwrap();
        assert_eq!(verdicts["payment"], ResourceVerdict::Consumed);
        assert_eq!(verdicts["tip"], ResourceVerdict::Consumed);
        assert_eq!(verdicts["voucher"], ResourceVerdict::Evaporated);
    }

    #[test]
    fn simulate_unknown_op_silently_skipped() {
        let verdicts = simulate_lifecycle(LINEAR_SRC, 0, &[("destroy", "payment", 1)], 1).unwrap();
        // payment not consumed
        assert!(matches!(verdicts["payment"], ResourceVerdict::Live { .. }));
    }

    #[test]
    fn empty_source_returns_empty_result() {
        let result = check_lad_resources("// no lad annotations here\nlet x = 5;", 0).unwrap();
        assert!(result.annotations.is_empty());
        assert!(result.verdicts.is_empty());
        assert!(result.is_clean());
    }
}
