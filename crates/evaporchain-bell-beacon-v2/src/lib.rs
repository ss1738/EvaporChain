//! Bell-Certified Beacon V2 — chain-level certificate.
//!
//! V1 (`evaporchain-bell-beacon`) ships the abstract CHSH gate at
//! integer milli-units. V2 hardens that primitive onto real chain
//! data:
//!
//! 1. Operator collects a window of concurrent-block pairs from the
//!    LightCone DAG (height range `[window_start, window_end)`).
//! 2. Doctrine observables (energy-above-median, tx-count-above-median)
//!    bin each pair into one of four CHSH setting-pairs deterministically
//!    via the pair's tag.
//! 3. The gate runs against the honest sample plus a synthetic
//!    coordinated-subset cartel injection over the *same* pair set
//!    (so the gap measures discrimination on the actual traffic).
//! 4. On Pass, a `BellCertificate` is issued: window bounds, sample
//!    stats, and a beacon seed = BLAKE3(domain || window || prev_block ||
//!    pair_tags). The certificate attaches to the proposer's next
//!    block header.
//! 5. Verifiers re-run the gate against the same window and re-derive
//!    the seed. Any divergence rejects the block.
//!
//! Anti-grinding follows from the seed derivation: the seed depends
//! on the chain-supplied `prev_block_hash` plus the canonical pair
//! tags, so a proposer cannot pick the seed by reordering pairs.

pub mod certificate;
pub mod issuance;
pub mod observables;
pub mod verification;

pub use certificate::{BellCertificate, CertificateBytes, GateThresholdsMilli};
pub use issuance::{issue_certificate, BeaconError};
pub use observables::{ConcurrentPair, PairStats};
pub use verification::{verify_certificate, VerifyError};

#[cfg(test)]
mod press_claim_tests {
    use super::*;
    use evaporchain_causal_chsh::gate::GateThresholds;
    use observables::PairStats;

    fn pair(e1: u64, t1: u64, e2: u64, t2: u64, tag_byte: u8) -> ConcurrentPair {
        let mut tag = [0u8; 32];
        tag[31] = tag_byte;
        tag[0] = tag_byte;
        ConcurrentPair {
            first: PairStats {
                energy: e1,
                tx_count: t1,
            },
            second: PairStats {
                energy: e2,
                tx_count: t2,
            },
            tag,
        }
    }

    fn balanced_window() -> Vec<ConcurrentPair> {
        let mut out = Vec::new();
        for i in 0..16u8 {
            out.push(pair(
                if i & 1 == 1 { 100 } else { 10 },
                if (i >> 1) & 1 == 1 { 100 } else { 10 },
                if (i >> 2) & 1 == 1 { 100 } else { 10 },
                if (i >> 3) & 1 == 1 { 100 } else { 10 },
                i,
            ));
        }
        out
    }

    /// **Audit fix (test-coverage gap)**: doctrine claim asserted as
    /// a structural test.
    ///
    /// Press claim: "Bell-Certified Beacon V2 issues a certificate
    /// that any validator can reproduce byte-for-byte from the same
    /// (chain_id, window, prev_block_hash, pairs) inputs. Pair
    /// reordering does NOT change the seed (anti-grinding). The
    /// certificate verifies under the same inputs and the prev_block
    /// hash is bound into the seed."
    #[test]
    fn the_press_claim_lives_as_a_test() {
        let pairs = balanced_window();
        let prev = [9u8; 32];

        // Issuance + verification round-trip.
        let cert = issue_certificate(
            "test-chain-v1",
            100,
            200,
            &pairs,
            GateThresholds::doctrine(),
            prev,
        )
        .unwrap();
        verify_certificate(
            "test-chain-v1",
            &pairs,
            prev,
            GateThresholds::doctrine(),
            &cert,
        )
        .unwrap();

        // Determinism: same inputs → byte-identical certificate.
        let cert2 = issue_certificate(
            "test-chain-v1",
            100,
            200,
            &pairs,
            GateThresholds::doctrine(),
            prev,
        )
        .unwrap();
        assert_eq!(cert, cert2);

        // Anti-grinding: reordering pairs does NOT change the seed.
        let mut shuffled = pairs.clone();
        shuffled.reverse();
        let cert_shuffled = issue_certificate(
            "test-chain-v1",
            100,
            200,
            &shuffled,
            GateThresholds::doctrine(),
            prev,
        )
        .unwrap();
        assert_eq!(cert.seed, cert_shuffled.seed);

        // Prev-block binding: changing prev_block_hash changes the seed.
        let cert_other = issue_certificate(
            "test-chain-v1",
            100,
            200,
            &pairs,
            GateThresholds::doctrine(),
            [7u8; 32],
        )
        .unwrap();
        assert_ne!(cert.seed, cert_other.seed);

        // Empty window fails closed.
        assert!(matches!(
            issue_certificate(
                "test-chain-v1",
                100,
                200,
                &[],
                GateThresholds::doctrine(),
                prev
            ),
            Err(BeaconError::EmptyWindow)
        ));
    }
}
