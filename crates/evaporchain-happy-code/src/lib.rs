//! HaPPY Holographic Decay Code.
//!
//! ## What HaPPY codes are
//!
//! Pastawski-Yoshida-Harlow-Preskill 2015 introduced the HaPPY
//! code: a holographic erasure-correcting code where one *bulk*
//! qudit is encoded redundantly across N *boundary* qudits via
//! a perfect tensor. Any subset of `≥ k_threshold` boundary
//! qudits suffices to reconstruct the bulk; any subset of
//! `< k_threshold` reveals nothing.
//!
//! In the discrete-classical analogue (which is what we ship —
//! V1 is integer arithmetic, not quantum), the bulk byte is
//! distributed via a `(N, k_threshold)`-threshold secret-sharing
//! scheme over a small prime field.
//!
//! ## What "decay" adds
//!
//! Each boundary share carries an energy tag. The reconstruction
//! gate requires `≥ k_threshold` shares whose energy is `≥
//! reconstruction_floor`. Shares whose energy has decayed below
//! floor cannot contribute even if they're presented.
//!
//! Implication: an adversary with N-1 stale shares has zero
//! bulk-recovery power; a holder with k fresh shares recovers
//! the bulk; a holder with k shares mostly stale + 1 fresh has
//! recovery power equal to the count of fresh shares.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Decay-floor on the share count, not the share energy
//!    sum.** The chain counts fresh shares; the threshold is
//!    `≥ k_threshold` fresh shares, regardless of the total
//!    energy across them. (Avoids "rich-get-richer" — one very
//!    fresh share doesn't substitute for k shares.)
//!
//! 2. **k_threshold structurally enforced.** Below threshold
//!    → reconstruction fails closed. Adversary cannot bypass
//!    by submitting decayed shares to "fill out" the count.
//!
//! 3. **Pure-integer Shamir-like reconstruction over a prime
//!    field.** Same Mersenne-31 used by dFRI; validators agree
//!    byte-for-byte.
//!
//! ## What this crate does NOT do
//!
//! - Does NOT implement the full quantum-tensor-network
//!   machinery. V1 ships the *classical* analog (Shamir-style
//!   threshold sharing) with the energy-decay gate.
//! - Does NOT implement multi-bulk codes (multiple bulk values
//!   per boundary). V1 is one bulk byte per boundary set.
//! - Does NOT model the full HaPPY locality structure. V1 is
//!   flat (any-k-of-N).
//!
//! ## Module map
//!
//! - [`field`] — Mersenne-31 field arithmetic + polynomial eval.
//! - [`encode`] — bulk → N boundary shares via random degree-(k-1)
//!   polynomial, deterministic from a seed.
//! - [`reconstruct`] — k-of-N Lagrange interpolation, decay-floor
//!   gate.

pub mod encode;
pub mod field;
pub mod reconstruct;

pub use encode::{encode_bulk, EncodeError, Share};
pub use field::{add_p, inverse_p, mul_p, neg_p, sub_p, FieldElem, MOD_P};
pub use reconstruct::{reconstruct_bulk, ReconstructError};
