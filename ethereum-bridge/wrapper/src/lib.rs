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
//! What this commit does **NOT YET** land but is structurally complete:
//!
//! - **EIP-197 calldata conversion shipped.** [`prove`] still returns
//!   128 bytes (arkworks compressed); [`proof_bytes_to_eip197`]
//!   converts to the 256-byte L1 calldata format (big-endian Fq,
//!   G2-coefficient order c1-then-c0 per EIP-197 §G_2 encoding). The
//!   CLI emits both. Sub-B-finish adds the in-circuit Halo2 IPA
//!   verifier; the EIP-197 layer is already ready for it.
//!
//! # Fixture contract
//!
//! The wrapper reads its witness from `verkle_proof_v2_sample.json`
//! emitted by [`ethereum-bridge/circuits/src/bin/fixture_emit.rs`]. The
//! schema is pinned by Solidity-side tests in
//! [`ethereum-bridge/contracts/test/VerkleProofVerifier.t.sol`].

#![deny(unsafe_code)]

pub mod circuit;
pub mod eip197;
pub mod fixture;
pub mod inputs;
pub mod nonnative_fq;
pub mod pallas_g1;
pub mod pallas_g1_double;
pub mod pallas_scalar_mul;
pub mod prover;

pub use circuit::WrapperCircuit;
pub use eip197::{
    eip197_split, proof_bytes_to_eip197, proof_to_eip197, ConversionError, Eip197Parts,
    EIP197_PROOF_LEN,
};
pub use fixture::{FixtureLoadError, VerkleFixture};
pub use inputs::{decode_anchor, AnchorDecodeError, WrapperPublicInputs};
pub use nonnative_fq::{
    alloc_nonnative_fq_input, alloc_nonnative_fq_witness, enforce_nonnative_fq_add,
    NonNativeFqVar,
};
pub use pallas_g1::{enforce_g1_add, NonNativePallasPoint};
pub use pallas_g1_double::enforce_g1_doubling;
pub use pallas_scalar_mul::enforce_scalar_mul;
pub use prover::{prove, setup, verify, ProveError, SetupError, VerifyError};
