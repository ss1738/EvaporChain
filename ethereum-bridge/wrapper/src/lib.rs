//! Halo2-IPA-in-BN254 Groth16 wrapper for the EvaporChain → Ethereum
//! bridge — T0.10 sub-B (starter).
//!
//! # What this crate proves
//!
//! Given a Halo2 IPA proof π (produced by `VerkleProverV2` in the sister
//! `ethereum-bridge/circuits/` workspace) and public inputs
//! `(state_root, key, value_commitment, params_fingerprint)`, this
//! crate produces a Groth16 BN254 proof π' that the L1 Solidity verifier
//! [`VerkleProofVerifier.sol`] can check via EIP-197 in ~250k gas.
//!
//! # Layering
//!
//! ```text
//!   Chain side                          Wrapper side                    L1 side
//!   ──────────                          ────────────                    ───────
//!   VerkleTrie::prove()    ─Halo2 IPA→  WrapperCircuit  ─Groth16 BN254→ VerkleProofVerifier.sol
//!   (Pallas/Vesta curves)               (this crate)                    (EIP-197 pairing)
//! ```
//!
//! The wrapper is the cross-curve bridge: it verifies a Halo2 IPA proof
//! *inside* a Groth16 BN254 circuit. The L1 verifier never sees Pallas/
//! Vesta — only a 256-byte BN254 Groth16 proof.
//!
//! # Status — T0.10 sub-B (starter)
//!
//! What this commit lands:
//!
//! - `WrapperPublicInputs` — the 4 BN254 Fr anchors the L1 verifier
//!   consumes, with bytes32-from-fixture decoding ([`decode_anchor`]).
//! - [`WrapperCircuit`] — a `ConstraintSynthesizer<Fr>` skeleton with
//!   the public-input wiring + a *placeholder* binding constraint
//!   (`state_root != 0`). The placeholder lets Groth16 setup/prove/
//!   verify run end-to-end so the surrounding pipeline can be built
//!   and tested *before* the in-circuit Halo2 IPA verifier lands.
//! - [`setup`] / [`prove`] / [`verify`] — Groth16 trusted-setup +
//!   prover + verifier wrappers, returning the canonical 256-byte
//!   proof encoding that `VerkleProofVerifier.sol` expects.
//!
//! What this commit does **NOT** land (multi-week sub-B-finish):
//!
//! - In-circuit Halo2 IPA verifier. Requires non-native arithmetic for
//!   Pallas Fq inside BN254 Fr — ~thousands of constraints per IPA
//!   challenge round. See [`WrapperCircuit::synthesize`] for the TODO
//!   skeleton.
//! - Trusted-setup ceremony (sub-C, weeks of operator coordination).
//!   Until ceremony output is in, [`setup`] uses an unsafe in-process
//!   keygen with `rand::thread_rng()` — fine for testing the pipeline,
//!   NEVER for production.
//! - **EIP-197 calldata conversion.** [`prove`] returns 128 bytes
//!   (arkworks `serialize_compressed`); the L1 verifier expects 256
//!   bytes uncompressed with big-endian field elements. See `prove`
//!   docstring for the conversion steps. Sub-B-finish adds a
//!   `proof_bytes_to_eip197` helper.
//!
//! # Fixture contract
//!
//! The wrapper reads its witness from `verkle_proof_v2_sample.json`
//! emitted by [`ethereum-bridge/circuits/src/bin/fixture_emit.rs`]. The
//! schema is pinned by Solidity-side tests in
//! [`ethereum-bridge/contracts/test/VerkleProofVerifier.t.sol`].

#![deny(unsafe_code)]

pub mod circuit;
pub mod fixture;
pub mod inputs;
pub mod prover;

pub use circuit::WrapperCircuit;
pub use fixture::{FixtureLoadError, VerkleFixture};
pub use inputs::{decode_anchor, AnchorDecodeError, WrapperPublicInputs};
pub use prover::{prove, setup, verify, ProveError, SetupError, VerifyError};
