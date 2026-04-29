//! Cone-locked Capsule (ETLP) — Tier 2.
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Cone-locked Capsule (ETLP)** — Energy-gated time-lock
//! > encryption; replaces RSW sequentiality with energy-witness.
//!
//! ## How this differs from classical time-lock
//!
//! Rivest-Shamir-Wagner 1996 time-lock encryption gates decryption
//! on *sequential computation* (`x → x² mod N` repeated `T` times).
//! Wall-clock-equivalent unlock cost; defeats parallel acceleration.
//!
//! ETLP gates decryption on *aggregate energy expenditure* on the
//! chain — to unlock you must present a `Witness` proving `≥ τ`
//! energy was spent under the chain-global λ since the capsule's
//! `seal_epoch`. The chain produces the witness for any consumer
//! that has paid the energy.
//!
//! Decentralised, deterministic, and parallel-resistant in the *only*
//! sense the chain cares about: the cost is the chain's λ-bounded
//! energy budget, not wall-clock arithmetic.
//!
//! ## Substrate scope
//!
//! Real encryption (e.g. ElGamal-over-Curve25519) plugs in around
//! the witness check. This crate exposes:
//!
//! - [`capsule`] — `Capsule { seal_epoch, energy_threshold,
//!   ciphertext_blob }`. `ciphertext_blob` is opaque; consumers
//!   choose their cipher.
//! - [`witness`] — `EnergyWitness { committed_energy, observed_epoch,
//!   binding }` — the chain's proof that enough energy has been
//!   spent.
//! - [`unlock`] — `can_unlock(capsule, witness, λ, current_epoch)`
//!   returns true iff the witness's λ-decayed remaining energy at
//!   `current_epoch` is at or above `capsule.energy_threshold`.

pub mod capsule;
pub mod unlock;
pub mod witness;

pub use capsule::{Capsule, CapsuleError};
pub use unlock::can_unlock;
pub use witness::{EnergyWitness, WitnessError};
