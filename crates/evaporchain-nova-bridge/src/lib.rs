//! T0.10 Path A — chain-side Nova accumulator → L1 Groth16-on-BN254 bridge.
//!
//! # Architecture
//!
//! ```text
//!   chain side                          bridge (this crate)              L1
//!   ──────────                          ───────────────────              ──
//!   evaporchain-proving::nova   ────►   verifier-circuit-as-R1CS  ────►  EIP-197
//!     RecursiveSNARK<E1, E2,                Groth16-on-BN254
//!                    RealBlockCircuit>      proof of accumulator-valid
//!     E1 = Bn256EngineKZG
//!     E2 = GrumpkinEngine
//! ```
//!
//! The chain's `evaporchain-proving::nova` module already produces a
//! `RecursiveSNARK<Bn256EngineKZG, GrumpkinEngine, RealBlockCircuit<G1>>`
//! after every fold step (see `crates/evaporchain-proving/src/nova.rs:1860`).
//! This crate's job is to wrap a verification of that accumulator inside
//! an arkworks R1CS circuit, then prove that circuit via Groth16-on-BN254.
//! The resulting 256-byte Groth16 proof is what L1's `VerkleProofVerifier.sol`
//! verifies via EIP-197.
//!
//! Why this architecture instead of the [`evaporchain-verkle-wrapper`]
//! crate's Halo2-IPA-in-Groth16 approach: the IPA-in-Groth16 wrapper
//! requires non-native scalar-multiplication over Pallas inside BN254 R1CS,
//! which is ~80× over the 2^18 Powers-of-Tau ceremony budget for a
//! realistic ~10-round IPA verifier. Nova folding does the IPA-level
//! accumulation *outside* the Groth16 circuit; the wrapper only needs to
//! verify the final accumulator state, which is constraint-cheap.
//!
//! A `DESIGN.md` companion (on a parallel docstring-refresh stack,
//! not yet merged into `main`) covers:
//!   - The three sub-paths considered (A1 = raw RecursiveSNARK, A2 = CompressedSNARK,
//!     A3 = re-prove via relayer) and why A3 is the recommended path.
//!   - The open research questions for Phase 2.2 onward.
//!   - The Phase 2 milestone breakdown.
//!
//! # Status (as of `SCAFFOLD_VERSION = "phase-2.2-section-1"`)
//!
//! - **Phase 2.1 — scaffold (DONE, PR #52):** Cargo.toml with
//!   nova-snark + arkworks deps pinned to the chain's existing
//!   versions; no new dependency drift.
//! - **Phase 2.2 starter — fixture generator (DONE, PR #55):** see
//!   [`recursive_snark_fixture`].
//! - **Phase 2.2 skeleton — verifier circuit shape (DONE, earlier
//!   commit):** see [`verifier_circuit::NovaVerifierCircuit`] +
//!   `ConstraintSynthesizer` impl + public-input wiring contract.
//! - **Phase 2.2 Section 1 — structural checks (DONE, PR #125):**
//!   off-circuit precondition gate
//!   [`verifier_circuit::NovaVerifierCircuit::validate_structurally`],
//!   typed
//!   [`verifier_circuit::StructuralValidationError`] variants, wired
//!   into `generate_constraints` as
//!   `SynthesisError::Unsatisfiable`.
//! - **Phase 2.2 Section 2 — Poseidon transcript (PARTIAL):**
//!   constants layer byte-correct against neptune on the parallel
//!   docstring-refresh stack (not yet on `main`). Sponge-framing
//!   gap remains BESPOKE multi-day work.
//! - **Phase 2.2 Section 3 — RelaxedR1CS satisfiability (OPEN):**
//!   BESPOKE, 3-5 day research deliverable.
//! - **Phase 2.3+ — scalar adapter + Groth16 wrapper (OPEN).**

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod grain_lfsr;
pub mod mds_linalg;
pub mod neptune_dump_parser;
pub mod recursive_snark_fixture;
pub mod vendored_neptune_grain;
pub mod verifier_circuit;

pub use recursive_snark_fixture::{
    fixture_stats, generate_fixture, FixtureStats, Scalar1, TrivialIncrementCircuit, E1, E2,
};
pub use verifier_circuit::NovaVerifierCircuit;

/// Marker constant. Phase 2.2-finish is multi-step:
///   - `phase-2.2-starter`    — fixture generator (PR #55)
///   - `phase-2.2-skeleton`   — verifier circuit skeleton + ConstraintSynthesizer + public-input wiring
///   - `phase-2.2-section-1`  — Section 1 structural checks filled in via off-circuit
///                              `validate_structurally` gate (PR #125, this commit bumps the
///                              constant). `StructuralValidationError` carries typed rejection
///                              variants; `generate_constraints` maps failures to
///                              `SynthesisError::Unsatisfiable`.
///   - `phase-2.2-section-2`  — Section 2 Poseidon transcript filled in
///   - `phase-2.2-section-3`  — Section 3 RelaxedR1CS satisfiability filled in (BESPOKE)
///   - `phase-2.2-complete`   — all three sections + empirical constraint count
pub const SCAFFOLD_VERSION: &str = "phase-2.2-section-1";

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin that the crate compiles and the milestone marker survives
    /// any future refactor. Replaced with the in-circuit
    /// `RecursiveSNARK::verify` PoC test when Phase 2.2 finish ships.
    #[test]
    fn scaffold_compiles_and_marker_present() {
        assert_eq!(SCAFFOLD_VERSION, "phase-2.2-section-1");
    }
}
