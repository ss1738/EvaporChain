//! Singh-Shamir Cells (HLTS) — Half-Life Threshold Shares.
//!
//! Per `research/INVENTION_STACK.md` §4.2:
//!
//! > **Singh-Shamir Cells (HLTS)** — Half-life threshold shares;
//! > secret recoverable only by surviving high-energy quorum.
//!
//! ## Mechanics
//!
//! Take a `(k, n)`-Shamir-style threshold scheme and attach an energy
//! to each share. Energies decay under λ. Reconstruction is possible
//! iff at least `k` of the `n` shares are still **above** the chain-
//! set survival threshold at the query epoch.
//!
//! As shares decay, the set of alive shares shrinks. When it drops
//! below `k`, the secret is permanently lost — a *time-bounded*
//! capability that the chain enforces by counting alive shares.
//!
//! ## Layer 1 (V1 — substrate)
//!
//! Share-survival accounting that gates whether reconstruction is
//! even attempted:
//!
//! - [`share`] — `Share { idx, energy, observed_epoch }`.
//! - [`survival`] — `is_alive(share, λ, query_epoch, threshold)` and
//!   `count_alive(shares, λ, query_epoch, threshold)`.
//! - [`quorum`] — `quorum_alive(shares, k, λ, query_epoch, threshold)`
//!   returns true iff ≥ k of n are alive.
//!
//! ## Layer 2 (V1 — production crypto, NEW)
//!
//! Real Shamir secret-sharing over `GF(2^61 - 1)`:
//!
//! - [`field`] — `Scalar(u64)` with field arithmetic over the
//!   Mersenne-61 prime. Add / sub / mul / inv / pow.
//! - [`secret`] — `Secret`, `SecretShare { idx, value }`,
//!   `deal(secret, n, k, rng)` and `reconstruct(shares, k)`.
//!   Lagrange interpolation at `x = 0` recovers the secret from any
//!   `k` shares.
//! - [`gated`] — composes Layer 1 and Layer 2: only attempts
//!   reconstruction if the chain-side energy quorum is met.
//!
//! ## Future slices (V2)
//!
//! - **BLS12-381 field upgrade** — swap `Scalar(u64)` for
//!   `bls12_381::Scalar` (256-bit field). Production-grade security.
//! - **Verifiable secret sharing** — Pedersen commitments to the
//!   polynomial coefficients so dealer cheating is detectable by
//!   share-holders.
//! - **ZK refresh attestations** — share-holder proves they refreshed
//!   without revealing the share. Sigma protocols over the BLS field.
//!
//! ## Security caveats (V1)
//!
//! - 61-bit field is **NOT** secure for production secret-sharing —
//!   k-1 shares + brute-force breaks it in ~2^61 operations. V1 is
//!   for correctness demonstration + integration with the energy
//!   gate. V2 over BLS12-381 is the production-grade upgrade.
//! - Deal RNG is blake3-XOF over a seed (deterministic). Production
//!   deal uses `OsRng`/`getrandom`.
//! - No share-validity proofs: malicious dealer not detectable yet.

pub mod field;
pub mod gated;
pub mod quorum;
pub mod secret;
pub mod share;
pub mod survival;

pub use field::Scalar;
pub use gated::{reconstruct_if_alive, ReconstructionError};
pub use quorum::quorum_alive;
pub use secret::{deal, reconstruct, DealRng, HltsError, Secret, SecretShare};
pub use share::Share;
pub use survival::{count_alive, is_alive};
