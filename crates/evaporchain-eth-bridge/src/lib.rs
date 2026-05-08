//! EvaporChain → Ethereum bridge proof-builder.
//!
//! Pairs with `ethereum-bridge/contracts/` (Foundry).
//! The Solidity side verifies; this crate constructs the proofs that
//! get verified, plus byte-exact ABI-encoded calldata.
//!
//! See [`ETHEREUM_BRIDGE_PLAN.md`](../../../ETHEREUM_BRIDGE_PLAN.md).
//!
//! ## Phases (mirrors the plan file)
//!
//! - [x] Phase 0 — scaffold (this commit)
//! - [ ] Phase 1 — `valset` (validator-set commitment, Solidity-hash agreement)
//! - [ ] Phase 2 — `commit_cert` (BLS commit-cert ABI encoder)
//! - [ ] Phase 3 — relayer
//! - [ ] Phase 4 — Verkle Pallas-IPA → Groth16 wrap
//! - [ ] Phase 5 — evaporation dispatcher proofs
//! - [ ] Phase 6 — Sepolia E2E

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod constants;
pub mod mmr;
pub mod valset;

pub use constants::*;
pub use mmr::{build_and_prove as mmr_build_and_prove, ghost_leaf_hash, InclusionProof};
pub use valset::{compute_root as compute_valset_root, Validator, ValsetError};
