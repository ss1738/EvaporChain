//! dFRI V2 — Fiat-Shamir-amplified multi-round energy-weighted FRI.
//!
//! ## What V2 adds over V1
//!
//! V1 (`evaporchain-dfri`) ships a single-round verifier query
//! gate. Soundness error per round is at most ½ + δ where δ is
//! the proximity gap (Ben-Sasson-Bentov-Horesh-Riabzev 2018). For
//! cryptographic soundness, you need O(λ) independent rounds.
//!
//! V2 (this crate) wraps V1's `verify_query_round` with:
//!
//! 1. **Fiat-Shamir transcript** — query positions are derived
//!    deterministically from a BLAKE3 transcript over the
//!    prover's commitments. The prover cannot choose query
//!    positions; the verifier doesn't need to be interactive.
//!
//! 2. **Batched query rounds** — sample `num_queries` distinct
//!    `x` values from the input domain, run the V1 query gate at
//!    each, fail if any single round fails.
//!
//! 3. **Soundness-error structural decrease** — the per-round
//!    error compounds: `total_error ≤ (½ + δ)^num_queries`.
//!    Documented in tests via the structural property that
//!    increasing `num_queries` strictly decreases the
//!    accept-rate of a tampered prover.
//!
//! ## What this crate does NOT do
//!
//! - Does NOT recompute V1's fold step. Caller passes the
//!   pre-folded codeword + the input codeword.
//! - Does NOT cover multi-layer fold (fold input, fold the
//!   folded, fold the doubly-folded, …). V2.1 ships the
//!   single-layer multi-round amplifier; multi-layer is V2.2.
//! - Does NOT hash any non-canonical bytes — the transcript
//!   binds (input_root_hash || folded_root_hash || domain_size
//!   || num_queries) so two verifiers with the same prover
//!   commitments derive identical query positions.
//!
//! ## Module map
//!
//! - [`transcript`] — Fiat-Shamir BLAKE3 transcript + query-
//!   position derivation.
//! - [`verify`] — [`verify_amplified`] driver that runs N V1
//!   query rounds against the same (input, folded) pair.

pub mod transcript;
pub mod verify;

pub use transcript::{derive_query_positions, FsTranscript, TRANSCRIPT_TAG};
pub use verify::{verify_amplified, AmplifiedError};
