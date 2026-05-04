//! Decay-FRI (dFRI) — energy-weighted Reed-Solomon proximity testing.
//!
//! ## What FRI is
//!
//! Standard FRI (Ben-Sasson-Bentov-Horesh-Riabzev 2018) is the
//! interactive proximity test that backs STARKs: a prover commits
//! to a function f over a coset of size N, and over log₂N rounds
//! folds f → f' → f'' → … into a constant. At each round the
//! verifier samples random query positions and checks the
//! consistency relation `f(x) + f(-x) = 2·f'(x²)` for the current
//! fold polynomial. Soundness: if f is far from low-degree, the
//! verifier rejects with high probability.
//!
//! ## What "decay" adds
//!
//! In dFRI, every codeword position `(x, f(x))` carries an
//! **energy tag** `e(x)`. The verifier's query gate enforces
//! `e(x) ≥ floor` per queried position. Positions whose energy
//! has decayed below the floor are **structurally invalid query
//! targets** — the prover cannot satisfy the consistency relation
//! using decayed coordinates because the verifier refuses to
//! evaluate them.
//!
//! Why this matters: in EvaporChain's setting, the codeword
//! represents the energy-filtered execution trace. Old trace
//! positions have decayed energy. dFRI ensures the proximity
//! test is exclusively against the *currently-live* trace,
//! which is what the chain commits to via Verkle / MERA. A
//! prover trying to pass FRI on a stale-coordinate forgery
//! gets rejected at query time.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Per-position energy floor.** Queried positions below
//!    floor → `BelowFloor` error. The chain configures the
//!    floor; positions above pass to the consistency relation.
//!
//! 2. **Pure-integer arithmetic.** All values, energies, and
//!    consistency checks are u64 / u128 fixed-point. No floats.
//!    Validator-deterministic byte-equality.
//!
//! 3. **Fold-step domain halves.** Each round halves the
//!    domain: `domain_size_round_k = domain_size_round_0 / 2^k`.
//!    A fold tree with mismatched sizes is rejected.
//!
//! ## What this crate does NOT do
//!
//! - Does NOT implement field arithmetic (a real FRI runs over
//!   F_p). V1 ships the *protocol skeleton* over `u64` arithmetic
//!   modulo a small prime, with the integer / Merkle / energy
//!   plumbing wired correctly. Production dFRI plugs in a
//!   prime-field crate.
//! - Does NOT do the full Reed-Solomon distance proof. V1 ships
//!   the verifier-side **query gate + consistency check** for a
//!   single round; multi-round amplification is V2.
//! - Does NOT model the interactive transcript / Fiat-Shamir
//!   challenge derivation (that lives in `evaporchain-eb-fs`).
//!
//! ## Module map
//!
//! - [`codeword`] — [`CodewordPosition`] + energy-tagged
//!   evaluation table.
//! - [`fold`] — `fold_codeword` round step + size-halving check.
//! - [`verify`] — [`verify_query_round`] gate.

pub mod codeword;
pub mod fold;
pub mod verify;

pub use codeword::{CodewordPosition, EnergyCodeword, FieldElem, MOD_P};
pub use fold::{fold_codeword, FoldError};
pub use verify::{verify_query_round, VerifyError};
