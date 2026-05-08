//! Verkle membership IVC circuit for the EvaporChain → Ethereum bridge.
//!
//! ## What this proves
//!
//! Given public inputs `(state_root: F, key: [u8;32], value: [u8;32])`, this
//! circuit produces a Nova IVC accumulator that folds D steps — one per level
//! of the Verkle proof path — and proves:
//!
//!   "I know a sequence of D (path_index, sibling_commitment_hash) pairs that
//!    chain via Poseidon from Poseidon(key, value) at the leaf up to state_root
//!    at the root."
//!
//! ## Security model
//!
//! The EC Pedersen commitment check (`C = Σ s_i·G_i`) is performed OFF-CIRCUIT
//! by the prover (via `evaporchain-crypto::VerkleProof::verify()`).  The circuit
//! binds the path to `(key, value, root)` via a Poseidon hash chain, which is
//! collision-resistant.  Combined with the BLS aggregate commit-cert that anchors
//! `state_root` into `EvaporHeaderInbox.sol`, this gives a two-layer guarantee:
//!   1. The state root is signed by ≥2/3 EvaporChain stake (Phase 3/4-MVP).
//!   2. The claimed (key, value) path is cryptographically bound to that root.
//!
//! Phase 4 full (native Pallas EC constraint verification) can be dropped in
//! later by replacing the Poseidon binding with an `EccChip`-based MSM verifier.

pub mod circuit;
pub mod prover;
pub mod types;

pub use circuit::VerkleStepCircuit;
pub use prover::{VerkleProof, VerkleProver};
pub use types::{EngineE1, EngineE2, VerklePublicParams};
