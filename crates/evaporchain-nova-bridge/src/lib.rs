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
//! See `DESIGN.md` next to this `lib.rs` for:
//!   - The three sub-paths considered (A1 = raw RecursiveSNARK, A2 = CompressedSNARK,
//!     A3 = re-prove via relayer) and why A3 is the recommended path.
//!   - The open research questions for Phase 2.2 onward.
//!   - The Phase 2 milestone breakdown.
//!
//! # Status — Phase 2.1 scaffold
//!
//! This crate ships:
//!   - Cargo.toml with nova-snark + arkworks deps pinned to the chain's
//!     existing versions (no new dependency drift).
//!   - This module-level doc + the design rationale doc next to it.
//!   - No working verifier yet. The verifier circuit is the multi-day
//!     research deliverable for Phase 2.2-2.3.
//!
//! Phase 2.1's goal is to give the *next* session a clean starting
//! point with the surrounding plumbing in place, so the research
//! attention can stay on the verifier algorithm itself.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod recursive_snark_fixture;

pub use recursive_snark_fixture::{
    fixture_stats, generate_fixture, FixtureStats, Scalar1, TrivialIncrementCircuit, E1, E2,
};

/// Marker constant that downstream tooling (CI, doc-drift audit) can
/// grep for to confirm the current Phase milestone. Replaced once
/// the full in-circuit verifier (Phase 2.2 finish) ships.
pub const SCAFFOLD_VERSION: &str = "phase-2.2-starter";

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin that the crate compiles and the milestone marker survives
    /// any future refactor. Replaced with the in-circuit
    /// `RecursiveSNARK::verify` PoC test when Phase 2.2 finish ships.
    #[test]
    fn scaffold_compiles_and_marker_present() {
        assert_eq!(SCAFFOLD_VERSION, "phase-2.2-starter");
    }
}
