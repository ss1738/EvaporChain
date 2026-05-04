//! Markdown renderer for `FinalityAttestation`.
//!
//! Same shape as the Causal-CHSH `evaporchain-causal-chsh-renderer`:
//! pure string output, no I/O. Caller hands in the attestation +
//! pre-computed root (or `None` to recompute) plus rendering options.
//!
//! Output layout:
//! ```text
//! # Finality Attestation
//!
//! ## Summary
//! - block_hash         = 0x…
//! - finalised_at_epoch = 1234
//! - attestation_root   = 0x…
//! - evaporated_forks   = N
//!
//! ## Light-Cone V2 (causal-root)
//! - causal_root = 0x…
//!
//! ## Bell-Beacon V2 (seed)
//! - bell_seed = 0x…
//!
//! ## Evaporated forks (Evap-Fork-Cert V2)
//! | # | fork_root | witness |
//! |--:|:---|:---|
//! | 1 | 0x… | 0x… |
//! …
//! ```

pub mod render;

pub use render::{render_attestation, RenderOptions};
