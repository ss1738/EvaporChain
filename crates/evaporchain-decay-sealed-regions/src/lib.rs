//! Decay-Sealed Regions — sub-block finality primitive.
//!
//! ## What this is
//!
//! A "region" is a contiguous half-open span `[lo, hi)` of state
//! keys (or equivalent index space) that the chain commits to as
//! **sealed** mid-block. The seal carries:
//! 1. The state-root hash over the region.
//! 2. An energy budget that decays from creation forward.
//! 3. A finality floor: when the seal's energy crosses below,
//!    the region transitions from `Tentative` to `Frozen` —
//!    cryptographically locked.
//!
//! Once `Frozen`, no validator can produce a competing seal over
//! the same span (or any span overlapping it). This gives the
//! chain a structural sub-block finality knob: hot regions of
//! state can be finalized faster than the block-level finality
//! by attaching extra-energy seals at production time.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Span-disjoint at any height.** Two seals at the same
//!    height with overlapping spans: the *higher-energy* seal
//!    wins; the registry refuses to register the lower-energy
//!    overlap. Pure thermal priority — same rule as Thermal-STM.
//!
//! 2. **Frozen seals are immutable.** Once energy decays below
//!    the finality floor and the chain calls `freeze_below_floor`,
//!    the seal cannot be replaced even by a higher-energy seal.
//!    Frozen is a one-way transition — sub-block finality is
//!    real finality.
//!
//! 3. **Domain-separated commitment.** Tag is
//!    `b"evaporchain:sealed-region:v1\0"`; binds (span_lo,
//!    span_hi, height, state_root, energy, sealed_at_epoch).
//!    Anti-replay: changing any field changes the commitment.
//!
//! ## Module map
//!
//! - [`region`] — [`Region`] descriptor + commitment hash.
//! - [`registry`] — [`SealRegistry`] with overlap-check + freeze.

pub mod region;
pub mod registry;

pub use region::{region_commitment, Region, RegionState, REGION_TAG};
pub use registry::{RegistryError, SealRegistry};
