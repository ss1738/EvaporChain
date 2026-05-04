//! Finality Attestation — one root, three V2 commitments folded in.
//!
//! V2 hardening of the locked invention stack produced three
//! independent commitments per finalised block:
//!
//! - **Light-Cone V2** — `causal_root`, a Merkle root over the
//!   block's ancestor set.
//! - **Bell-Beacon V2** — `BellCertificate.seed`, a 32-byte
//!   anti-grinding randomness anchor.
//! - **Evap-Fork-Cert V2** — a list of `EvaporatedForkCertV2`
//!   (witness bytes), each proving an alternative fork has decayed
//!   below threshold.
//!
//! This crate folds those three into a single canonical 32-byte
//! attestation root. A light client that sees `(block_hash,
//! FinalityAttestation, attestation_root)` can verify finality
//! without holding the DAG, the beacon archive, or any fork blocks:
//!
//! ```text
//!   attestation_root = BLAKE3(domain
//!                            || block_hash
//!                            || finalised_at_epoch
//!                            || causal_root
//!                            || bell_seed
//!                            || merkle_root_of_evaporated_forks)
//! ```
//!
//! Tamper any field → root diverges → light client rejects.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT re-prove the three V2 sub-commitments. Those are
//!   produced upstream by the V2 crates; this crate just folds them.
//! - It does NOT model committee signatures over the attestation
//!   (consensus-layer concern).

pub mod attest;

pub use attest::{
    build_attestation, verify_attestation, AttestationError, EvaporatedForkWitnessRef,
    FinalityAttestation,
};
