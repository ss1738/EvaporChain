//! EPA-MMR V2 — sumcheck-folded inclusion proofs.
//!
//! ## What V2 adds over V1
//!
//! V1 (`evaporchain-epa-mmr`) ships standard MMR inclusion: a
//! `(leaf_index, leaf, sibling_path, peak_set)` tuple verified
//! by walking O(log N) BLAKE3 hashes. The energy floor check is
//! structural — decayed leaves are unprovable.
//!
//! V2 (this crate) adds an **alternative path**: instead of
//! walking the hash siblings, the verifier executes the
//! Lund-Fortnow-Karloff-Nisan 1992 **sumcheck protocol** on the
//! multilinear extension of the leaf-energy vector. The verifier
//! does O(log N) field operations per round + O(log N) rounds =
//! O(log² N) total, with constant-size per-round messages.
//!
//! Crucially, the **energy floor still gates** in V2: the
//! sumcheck claim is `Σ_x leaf_energy(x) · selector(x, target) ≥
//! floor`, where `selector` is the multilinear extension of the
//! one-hot indicator at `target`. A decayed leaf yields a
//! sumcheck claim that fails to clear the floor.
//!
//! ## What this crate ships in V1 of the V2-protocol
//!
//! 1. **Multilinear extension** of a leaf-energy vector over
//!    Mersenne-31. Pure integer.
//! 2. **Sumcheck rounds** — each round, the prover sends a
//!    univariate polynomial; the verifier checks consistency
//!    and folds with a Fiat-Shamir-derived random point.
//! 3. **Energy-floor verifier gate** — at the end of the
//!    sumcheck, the claimed value must clear the verifier's
//!    floor.
//!
//! ## What this crate does NOT do
//!
//! - Does NOT replace V1's hash-path inclusion. The two paths
//!   coexist; the chain picks based on cost (sumcheck wins for
//!   large N + many proofs amortised; hash-path wins for small
//!   single proofs).
//! - Does NOT integrate Merkle commitment of the sumcheck
//!   transcript. Production would commit each round's
//!   polynomial via a vector-commitment scheme.
//! - Does NOT cover multi-leaf inclusion (sumcheck batched
//!   across N queries). V2.1 ships single-leaf; V2.2 batched.
//!
//! ## Module map
//!
//! - [`field`] — Mersenne-31 field + multilinear extension.
//! - [`sumcheck`] — sumcheck round protocol + verifier gate.

pub mod field;
pub mod sumcheck;

pub use field::{multilinear_extend, MOD_P};
pub use sumcheck::{verify_sumcheck_inclusion, SumcheckError, SumcheckProof};
