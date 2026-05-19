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
//! See `README.md` next to this `lib.rs` for the module-by-module
//! navigation guide + operator binary cheatsheet.
//!
//! # Status (as of `SCAFFOLD_VERSION = "phase-2.5-operational"`)
//!
//! The full proof-emission scaffold is operationally complete on
//! `main`. A real Nova `RecursiveSNARK` fixture flows end-to-end
//! through every wrapper to L1-paste-ready 256-byte EIP-197 bytes:
//!
//! ```text
//! generate_fixture
//!   → l_u_secondary_extract (serde reflection)
//!   → scalar_adapter
//!   → circuit_builder::build_circuit_from_fixture
//!   → groth16_wrapper::setup / prove
//!   → eip197::proof_to_eip197_bytes
//! ```
//!
//! Per-phase:
//!
//! - **Phase 2.1 — scaffold (DONE, PR #52).**
//! - **Phase 2.2 starter — fixture generator (DONE, PR #55):** see
//!   [`recursive_snark_fixture`].
//! - **Phase 2.2 skeleton — verifier circuit shape (DONE):** see
//!   [`verifier_circuit::NovaVerifierCircuit`] +
//!   `ConstraintSynthesizer` impl + public-input wiring contract.
//! - **Phase 2.2 Section 1 — structural checks (DONE, PR #125):**
//!   off-circuit precondition gate
//!   [`verifier_circuit::NovaVerifierCircuit::validate_structurally`]
//!   mapped to `SynthesisError::Unsatisfiable`.
//! - **Phase 2.2 Section 2 — Poseidon transcript constants (DONE):**
//!   byte-correct against neptune end-to-end via the substrate
//!   stack ([`grain_lfsr`], [`compress_ark`], [`mds_linalg`],
//!   [`neptune_dump_parser`], [`neptune_reference`],
//!   [`section2_gadget`], [`vendored_neptune_grain`]) +
//!   `dump-neptune-constants` / `dump-our-compressed-ark` /
//!   `check-neptune-parity` operator binaries.
//! - **Phase 2.2 Section 2 — sponge framing (DONE, 2026-05-13):**
//!   [`section2_gadget::enforce_neptune_sponge_primary`] byte-correct vs neptune.
//!   Root cause: `pre_sparse_mds` transpose; neptune `product_mds_with_matrix`
//!   computes `matrix^T * elements`. Both parity tests pass:
//!   `neptune_sponge_gadget_pinned_42_7_99` + `fully_aligned_gadget_byte_parity_with_neptune`.
//! - **Phase 2.2 Section 3 — RelaxedR1CS satisfiability (OPEN,
//!   BESPOKE):** 3-5 day research deliverable.
//! - **Phase 2.3 — scalar adapter + circuit_builder (DONE):**
//!   [`scalar_adapter`], [`l_u_secondary_extract`],
//!   [`circuit_builder::build_circuit_from_fixture`] producing a
//!   `NovaVerifierCircuit` with real `committed_hash_*` values.
//! - **Phase 2.4 — Groth16 setup/prove/verify (DONE):** see
//!   [`groth16_wrapper`]. Round-trip green on dummy and real
//!   fixtures.
//! - **Phase 2.5 — EIP-197 codec (DONE):** see [`eip197`]. 256-byte
//!   wire format with Fq2 (c1, c0) swap.
//!
//! Operator binaries:
//!
//! - `dump-neptune-constants`, `dump-our-compressed-ark`,
//!   `check-neptune-parity` — Section 2 constants verification.
//! - `dummy-proof-emit`, `fixture-proof-emit` — full proof
//!   pipeline emitting L1-paste-ready hex.
//!
//! What's still required for **cryptographic soundness** (vs
//! operational completeness): Section 3 RelaxedR1CS only (Section 2
//! sponge framing CLOSED 2026-05-13).

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::doc_overindented_list_items)]

pub mod bn256_g1_validation;
pub mod circuit_builder;
pub mod compress_ark;
pub mod eip197;
pub mod grain_lfsr;
pub mod grumpkin_config;
pub mod groth16_wrapper;
pub mod l_u_secondary_extract;
pub mod mds_linalg;
pub mod neptune_dump_parser;
pub mod neptune_permutation_gadget;
pub mod neptune_reference;
pub mod neptune_sponge;
pub mod recursive_snark_fixture;
pub mod scalar_adapter;
pub mod s4_msm_gadget;
pub mod s4_primary_msm_gadget;
pub mod s4_secondary_extract;
pub mod s4b_secondary_r1cs_gadget;
pub mod section2_gadget;
pub mod section2_witness;
pub mod section3_gadget;
pub mod section3_witness;
pub mod vendored_neptune_grain;
pub mod verifier_circuit;

pub use recursive_snark_fixture::{
    fixture_stats, generate_fixture, public_inputs_for_bridge, FixtureStats, Scalar1,
    TrivialIncrementCircuit, E1, E2,
};
pub use scalar_adapter::{
    ark_fr_to_primary, ark_fr_to_secondary_lossy, primary_to_ark_fr, secondary_to_ark_fr_lossy,
    PrimaryScalar, SecondaryScalar,
};
pub use neptune_dump_parser::{parse_dump, expected_crc_len, extract_compressed_round_constants};
pub use verifier_circuit::NovaVerifierCircuit;

/// Marker constant. Phase progression for the bridge crate:
///   - `phase-2.2-starter`    — fixture generator (PR #55)
///   - `phase-2.2-skeleton`   — verifier circuit skeleton
///   - `phase-2.2-section-1`  — Section 1 off-circuit gate (#125)
///   - `phase-2.2-section-2-constants` — neptune-byte-correct constants substrate (#130–#142)
///   - `phase-2.3-operational` — scalar adapter + circuit_builder
///                               + real l_u_secondary extraction (#143, #151, #152)
///   - `phase-2.4-operational` — setup/prove/verify wrappers (#145, #146)
///   - `phase-2.5-operational` — EIP-197 codec + full real-fixture pipeline
///                               (#147–#149, #153, #154). This is the
///                               current marker — proof emission is
///                               operationally complete; the remaining
///                               gap to cryptographic soundness is
///                               Section 2 sponge framing + Section 3
///                               RelaxedR1CS, both BESPOKE.
///   - `phase-2-complete`     — sponge framing + RelaxedR1CS closed.
pub const SCAFFOLD_VERSION: &str = "phase-2.6-operational";

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin that the crate compiles and the milestone marker survives
    /// any future refactor. Replaced with the in-circuit
    /// `RecursiveSNARK::verify` PoC test when Phase 2.2 finish ships.
    #[test]
    fn scaffold_compiles_and_marker_present() {
        assert_eq!(SCAFFOLD_VERSION, "phase-2.6-operational");
    }
}
