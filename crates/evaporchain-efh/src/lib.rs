//! Evaporative Filtration Homology (EFH).
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 9:
//!
//! > **Evaporative Filtration Homology (EFH)** — Persistent homology
//! > with energy as the filtration parameter. Stability theorem
//! > (Cohen-Steiner-Edelsbrunner-Harer 2007) gives free tamper-
//! > evidence.
//!
//! ## Why this gives free tamper-evidence
//!
//! Persistent homology computes "topological features" (connected
//! components for `H_0`, holes for `H_1`, etc.) at every level of a
//! filtration parameter. As the parameter sweeps from `+∞` to `−∞`
//! (or low to high in the energy form), features appear and
//! disappear; their `(birth, death)` pairs form the *persistence
//! diagram*.
//!
//! The Cohen-Steiner-Edelsbrunner-Harer stability theorem states:
//!
//! ```text
//!   bottleneck_distance( PD(f), PD(g) )  ≤  ||f − g||_∞
//! ```
//!
//! i.e. small input perturbations yield bounded perturbations of the
//! persistence diagram, in the bottleneck metric. On EvaporChain that
//! means: tampering with the chain's energy distribution shifts the
//! persistence diagram by at most the magnitude of the tamper —
//! providing a verifiable, principled tamper-evidence signal.
//!
//! ## Substrate scope
//!
//! - [`diagram`] — `PersistenceDiagram = Vec<(birth, death)>`.
//! - [`h0`] — `compute_h0(energies)` for 0-dimensional persistence
//!   (= sorted-energy → component-merging order).
//! - [`distance`] — `bottleneck_distance(d1, d2)` (the supremum over
//!   matched pairs of |d1.b - d2.b| max |d1.d - d2.d|, with caller-
//!   tractable matching since at substrate scope we use the simple
//!   sorted-pair matching that's exact for `H_0`).

pub mod diagram;
pub mod distance;
pub mod h0;

pub use diagram::{Filtration, PersistenceDiagram};
pub use distance::bottleneck_distance;
pub use h0::compute_h0;
