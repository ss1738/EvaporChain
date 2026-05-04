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

#[cfg(test)]
mod press_claim_tests {
    use super::*;

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Topological Light Client verifies block-to-block
    /// barcode transitions against the chain-committed `bd_max` per
    /// the CSEH stability theorem. A transition exceeding `bd_max` is
    /// rejected even when the barcode hash is internally consistent —
    /// tampering shows up as out-of-bound topology change."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let genesis = Barcode::from_bars(vec![Bar::new(1, 10).unwrap()]);
        let mut lc = LightClient::new(0, genesis);

        // Valid step: bottleneck distance well within bd_max=5.
        let next = Barcode::from_bars(vec![Bar::new(1, 11).unwrap()]);
        let header = BlockHeader {
            height: 1,
            barcode_hash: next.hash(),
            bd_max: 5,
        };
        lc.ingest(&header, next).unwrap();
        assert_eq!(lc.current_height(), 1);

        // Tampered step: barcode wildly different, bd_max claims 1.
        // Stability bound MUST reject even though hash matches.
        let leap = Barcode::from_bars(vec![Bar::new(1_000, 2_000).unwrap()]);
        let leap_header = BlockHeader {
            height: 2,
            barcode_hash: leap.hash(),
            bd_max: 1,
        };
        let res = lc.ingest(&leap_header, leap);
        assert!(matches!(
            res,
            Err(LightClientError::StabilityBoundViolated { .. })
        ));

        // Hash-mismatch detection: header commits to one barcode,
        // supplied barcode hashes to another → rejected.
        let other = Barcode::from_bars(vec![Bar::new(2, 12).unwrap()]);
        let mismatched = BlockHeader {
            height: 2,
            barcode_hash: [0u8; 32], // wrong hash
            bd_max: 100,
        };
        let res2 = lc.ingest(&mismatched, other);
        assert!(matches!(
            res2,
            Err(LightClientError::BarcodeHashMismatch { .. })
        ));
    }
}
