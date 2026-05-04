//! Topological Light Clients (PLC).
//!
//! ## What this crate is
//!
//! A light-client state structure built around a **persistent-
//! homology barcode summary** of the chain. Each block carries a
//! `Barcode` — a multiset of `(birth_energy, death_energy)`
//! intervals representing topological features (connected
//! components, loops, voids) of the energy-filtered state graph.
//!
//! By the Cohen-Steiner-Edelsbrunner-Harer 2007 stability
//! theorem: the **bottleneck distance** between two barcodes is
//! bounded above by the interleaving distance between their
//! generating filtrations. In our setting:
//!
//! - Two consecutive blocks differ by `Δstate` in interleaving
//!   distance (the chain's per-block state delta).
//! - Therefore their barcodes differ by at most `Δstate` in
//!   bottleneck distance.
//! - A "stability bound" carried in each block header (`bd_max`)
//!   gives the chain's *committed* upper bound on this
//!   bottleneck distance per block.
//!
//! ## What the light client checks
//!
//! For each new block, the light client verifies:
//!
//! 1. The new barcode's bottleneck distance to the previous
//!    block's barcode ≤ `bd_max`.
//! 2. The barcode's domain-tagged hash matches the
//!    `barcode_hash` field in the block header.
//!
//! That's it. No full state, no header chain, no Verkle paths.
//! Sublinear-in-N because barcode size depends on the *number
//! of distinct topological features*, not the total state.
//!
//! ## Three structural decisions enforced as tests
//!
//! 1. **Bottleneck distance is metric.** Symmetric, non-negative,
//!    triangle inequality. (Tested explicitly for the bottleneck
//!    matching algorithm we ship.)
//!
//! 2. **Barcode is canonically ordered.** Bars sorted by
//!    `(birth, death)` lex. Two validators producing the same
//!    set of intervals agree byte-for-byte on the barcode hash.
//!
//! 3. **Stability bound is honored at update.** Light client
//!    rejects a transition whose bottleneck distance exceeds
//!    the committed `bd_max` — even if the hash chain validates.
//!    This is the EFH-derived guarantee: tampering shows up as
//!    out-of-bound topology change.
//!
//! ## What this crate does NOT do
//!
//! - It does NOT *generate* barcodes. The full-node persistent-
//!   homology computation lives in `evaporchain-efh`. This crate
//!   consumes barcodes and performs distance / verification.
//! - It does NOT model the inverse problem (recover state from
//!   barcode). That's not light-client work.
//! - It does NOT cover infinite (∞-death) bars formally. V1
//!   represents them as `u64::MAX` death values; the bottleneck
//!   matching treats `(b, ∞)` as matchable only with another
//!   `(b', ∞)`.
//!
//! ## Module map
//!
//! - [`barcode`] — [`Barcode`] + canonical hash.
//! - [`bottleneck`] — bottleneck-distance matcher.
//! - [`client`] — [`LightClient`] state machine: ingest blocks,
//!   verify stability, advance.

pub mod barcode;
pub mod bottleneck;
pub mod client;

pub use barcode::{Bar, Barcode, BarcodeError, BARCODE_TAG};
pub use bottleneck::bottleneck_distance;
pub use client::{BlockHeader, LightClient, LightClientError};
