//! Evaporated-Fork Certificate V2 — Bell-anchored witness.
//!
//! V1 (`evaporchain-evap-fork-cert`) ships a certificate whose
//! witness hashes only chain-math inputs (fork root, eval epoch,
//! seed energy, decayed energy, threshold). A tactical forker who
//! knows the chain's half-life can pre-compute a future certificate
//! before the chain has confirmed any blocks at the eval epoch — the
//! V1 witness is purely a function of public chain state.
//!
//! V2 closes the pre-computation gap by binding the certificate to
//! a 32-byte chain-supplied seed anchor (typically a `BellCertificate.seed`
//! from `evaporchain-bell-beacon-v2`) plus the epoch that seed was
//! issued at. The anchor isn't known until the chain publishes it,
//! so a forker cannot compute the V2 witness in advance even if they
//! know the energy-decay math.
//!
//! ## Causality constraint
//!
//! The seed-anchor epoch must satisfy `seed_anchor_epoch ≤
//! evaluated_at_epoch`: an evaporated-fork certificate that claims
//! "this fork is dead as of epoch E" cannot consume a seed published
//! after epoch E.
//!
//! ## What V2 does NOT change
//!
//! - **Decay math** — V2 reuses V1's `prove_fork_evaporated` for the
//!   energy aggregation; only the witness derivation is upgraded.
//! - **Verification structure** — V2 verifier still re-derives the
//!   witness and checks decayed < threshold. The new anchor field
//!   is folded into the witness derivation.
//! - **Backward compatibility** — V1 certs cannot be parsed as V2
//!   (different shape); this is intentional. Operators choose one.

pub mod cert;
pub mod prove;
pub mod verify;

pub use cert::{CertV2Error, EvaporatedForkCertV2};
pub use prove::prove_fork_evaporated_v2;
pub use verify::verify_evaporated_cert_v2;
