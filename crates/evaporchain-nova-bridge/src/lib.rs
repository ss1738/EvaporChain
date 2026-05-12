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
//! # Status — `phase-2.2-section-2-constants` (per [`SCAFFOLD_VERSION`])
//!
//! What ships today:
//!   - **`NovaVerifierCircuit`** with public-input wiring (PR #56) +
//!     off-circuit structural-validation gate (PR #64).
//!   - **Full Groth16-on-BN254 pipeline** — setup / prove / verify
//!     (PR #67) + canonical I/O (PR #72) + EIP-197 layout (PR #71)
//!     + operator CLIs `setup-keys` / `prove-and-verify`. Compressed
//!     proof = 128 B, EIP-197 uncompressed = 256 B.
//!   - **Section 2 constants byte-complete vs neptune** —
//!     `grain_lfsr::generate_round_constants_bn254_arity_24_standard`
//!     + `compress_ark::compress_full` reproduce neptune's
//!     `compressed_round_constants` byte-for-byte across all 259
//!     entries (PR #103). Verified via 4 independent paths.
//!
//! Remaining BESPOKE work (multi-day each):
//!   - **Section 2 sponge framing** — port neptune's SBOX-trick-fused
//!     `Poseidon::hash_optimized_static` into the arkworks
//!     `PoseidonSpongeVar` per-round op. Tracked by PR #98's
//!     `assert_ne!` parity canary.
//!   - **Section 3** — in-circuit RelaxedR1CS satisfiability.
//!   - **Phase 2.3 adapter** — `CompressedProof` bytes →
//!     `RecursiveSNARK<E1, E2, RealBlockCircuit>`. Blocked on
//!     `RecursiveSNARK` private-field access.
//!   - **Phase 2.5 Solidity Foundry test** — consume the wrapper's
//!     EIP-197 bytes on-chain via `BN254_PAIRING` precompile.
//!
//! See `DESIGN.md` for the milestone table + `README.md` for the
//! module/binary navigation guide.

#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod grain_lfsr;
pub mod compress_ark;
pub mod mds_linalg;
pub mod vendored_neptune_grain;
pub mod neptune_dump_parser;
pub mod neptune_reference;
pub mod recursive_snark_fixture;
pub mod section2_gadget;
pub mod verifier_circuit;

pub use grain_lfsr::{
    generate_round_constants_bn254_arity_24_standard, grain_seed_state, GrainLfsr,
    GrainSeedParams, BN254_FR_BITS,
};
pub use neptune_dump_parser::{
    decode_hex_scalar, expected_crc_len, extract_compressed_round_constants, extract_mds_matrix,
    parse_dump, NeptuneDumpShape,
};
pub use neptune_reference::{neptune_hash_primary, PrimaryScalar};
pub use section2_gadget::{
    enforce_poseidon_primary, enforce_section_2_primary, fully_aligned_poseidon_config,
    neptune_aligned_poseidon_config, placeholder_poseidon_config,
};
pub use recursive_snark_fixture::{
    fixture_stats, generate_fixture, FixtureStats, Scalar1, TrivialIncrementCircuit, E1, E2,
};
pub use verifier_circuit::NovaVerifierCircuit;

/// Marker constant. Phase 2.2-finish is multi-step:
///   - `phase-2.2-starter`              — fixture generator (PR #55)
///   - `phase-2.2-skeleton`             — verifier circuit skeleton + ConstraintSynthesizer + public-input wiring (PR #56)
///   - `phase-2.2-section-1`            — Section 1 structural checks filled in (PR #64)
///   - `phase-2.2-section-2-constants`  — Section 2 Poseidon CONSTANTS byte-complete (PR #103)
///   - `phase-2.2-section-2`            — Section 2 Poseidon hash byte-complete (sponge port — TBD)
///   - `phase-2.2-section-3`            — Section 3 RelaxedR1CS satisfiability filled in (BESPOKE)
///   - `phase-2.2-complete`             — all three sections + empirical constraint count
pub const SCAFFOLD_VERSION: &str = "phase-2.2-section-2-constants";

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin that the crate compiles and the milestone marker survives
    /// any future refactor. Replaced with the in-circuit
    /// `RecursiveSNARK::verify` PoC test when Phase 2.2 finish ships.
    #[test]
    fn scaffold_compiles_and_marker_present() {
        assert_eq!(SCAFFOLD_VERSION, "phase-2.2-section-2-constants");
    }
}
