//! Total-Programming EvaporScript V2 — fuel-based termination.
//!
//! ## V1 vs V2
//!
//! V1 (`evaporchain-total-evaporscript`) ships **syntactic
//! strict-decrement**: a `BoundedWhile` loop is total iff its
//! body contains an `Assign { ranking_var, ranking_var − k }`
//! for positive literal `k` on every CFG path.
//!
//! Sound but conservative. Programs that ARE total but use a
//! non-syntactic decrement (e.g., `r = r * old_r / (old_r + 1)`,
//! integer division by 2, variable-step decrement) get rejected
//! by V1 even though they terminate.
//!
//! V2 (this crate) adds an **alternative termination certificate**:
//! the caller supplies an initial fuel value. The checker
//! symbolically executes the loop body once with the input fuel,
//! observes the post-body fuel, and confirms `fuel_after <
//! fuel_before` on every CFG path — exiting if the certificate
//! holds. The fuel itself acts as a ranking function the V1
//! syntactic check couldn't see.
//!
//! ## What this V2 actually checks
//!
//! 1. The caller supplies `(initial_fuel, fuel_var, body)`.
//! 2. The checker walks the body, simulating each `Assign`'s
//!    effect on `fuel_var` symbolically over a fixed initial
//!    state.
//! 3. After execution, the new fuel value must be `< initial_fuel`.
//!
//! This is a **bounded** symbolic execution — single-step, single-
//! initial-state. It catches whether ONE iteration decreases
//! fuel, which is the per-iteration termination measure.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Fuel must strictly decrease per iteration.** `fuel_after
//!    < fuel_before` required; equality or growth → reject.
//!
//! 2. **Symbolic execution is single-state.** Caller passes a
//!    starting `var → value` map; checker simulates assigns over
//!    that state. Loops nested inside the body are not
//!    re-executed (V1 takes care of those).
//!
//! 3. **Termination certificate composes with V1's gate.** A
//!    program passes V2 iff it would have passed V1 OR carries a
//!    valid fuel certificate. Both routes structurally bound the
//!    iteration count.
//!
//! ## What this crate does NOT do
//!
//! - Does NOT implement full symbolic execution (multi-path,
//!   constraint-solver-backed). V1 of V2 ships single-path
//!   single-state evaluation. Multi-path is V2.2.
//! - Does NOT model arithmetic overflow as termination. The
//!   chain ensures inputs are bounded externally.
//! - Does NOT replace V1. V2 is additive: a fuel certificate is
//!   one possible termination witness; V1 syntactic check is
//!   another.
//!
//! ## Module map
//!
//! - [`evaluator`] — single-step symbolic [`evaluate`] over a
//!   `Term` body with a `state: HashMap<String, i64>`.
//! - [`certify`] — [`certify_fuel`] entry point: check
//!   `fuel_after < fuel_before` after one body execution.

pub mod certify;
pub mod evaluator;

pub use certify::{certify_fuel, FuelCertError};
pub use evaluator::{evaluate, EvalError};
