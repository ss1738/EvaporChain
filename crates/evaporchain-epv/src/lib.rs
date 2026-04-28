//! Evaporative Protocol Versioning (EPV).
//!
//! Per `research/INVENTION_STACK.md` §A1.2 T5 (Tier-0 theorem-grade):
//!
//! > **Evaporative Protocol Versioning (EPV)** — old protocol versions
//! > decay below `E_min` and become *cryptographically un-runnable*;
//! > verifier modules pruned by the same λ.
//! >
//! > "Rollback is not socially discouraged — it is *physically
//! > impossible*. The verifier modules for old versions have evaporated."
//!
//! ## How "physically impossible" actually works
//!
//! Each protocol version `v` ships with a *verifier energy* — a seed
//! amount the chain credits at the moment that version is activated.
//! That energy decays via the chain-global λ. When a version's
//! remaining energy drops below `E_min`, the verifier-module pruner
//! removes its code path from the active node binary.
//!
//! After pruning, ANY block claiming `version = v` simply has no
//! verifier to validate against — and the consensus path will reject
//! by `UnknownProtocolVersion`. The version isn't politically
//! deprecated; it's literally absent from the binary.
//!
//! ## Module map
//!
//! - [`registry`] — `ProtocolVersion` + `EpvRegistry`. The active set
//!   of protocol versions, each with a seed energy and activation
//!   epoch. `registry.is_runnable(v, λ, t)` is the gate the consensus
//!   layer reads at block-validation time.
//! - [`prune`] — `prune_evaporated(registry, λ, t, e_min)` walks the
//!   registry and removes versions whose remaining energy is below
//!   `e_min`. Run per-block (or per-epoch) by the node binary.

pub mod prune;
pub mod registry;

pub use prune::{prune_evaporated, PruneOutcome};
pub use registry::{EpvRegistry, ProtocolVersion, RegistryError};
