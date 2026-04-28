//! Evaporated-Fork Certificates.
//!
//! Per `research/INVENTION_STACK.md` §4.1 row 10:
//!
//! > **Evaporated-Fork Certificates** — Negative-finality ZK proof:
//! > a fork *cannot* finalize because its energy decayed below
//! > threshold. Light clients verify in O(1).
//!
//! ## Why this is meaningful
//!
//! Existing chains' fork-rejection logic ("longest chain", GHOST,
//! Casper FFG) gives *positive* finality: "this chain wins because
//! it has the most weight". EvaporChain's energy-decay model
//! supports the dual: *negative* finality. A certificate proving
//! that a competing fork's *aggregate energy is below the chain-set
//! finalization threshold* immediately rules it out — no scan of the
//! competing chain's history needed at light-client time.
//!
//! Pairs structurally with [`evaporchain_prp`] (Provable Retention
//! Proofs), which is the positive dual. Together they cover both
//! finality directions in O(1) light-client work.
//!
//! ## Substrate
//!
//! - [`cert`] — `EvaporatedForkCert { fork_root, evaluated_at_epoch,
//!   total_seed_energy, decayed_energy, threshold, witness }`.
//! - [`prove`] — `prove_fork_evaporated(fork_root, blocks, λ, t,
//!   threshold)` aggregates the fork's λ-decayed energy at `t` and
//!   returns the certificate.
//! - [`verify`] — `verify_evaporated_cert(cert)` checks decayed_energy
//!   is strictly below threshold AND the witness re-derives.

pub mod cert;
pub mod prove;
pub mod verify;

pub use cert::{CertError, EvaporatedForkCert, ForkBlock};
pub use prove::prove_fork_evaporated;
pub use verify::verify_evaporated_cert;
