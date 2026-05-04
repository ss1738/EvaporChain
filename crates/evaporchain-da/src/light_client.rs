//! Light-client DA sampler.
//!
//! Bridges the existing 2D-encoder + cell-proof verifier (which run against a
//! local `BlockDA2DPackage`) to a true light-client model where the only
//! local state is the block header. Cells are fetched from peers and verified
//! against `BlockDA2DHeader` row/column commitments. Bad proofs trigger a
//! peer-fault hook so the host (light-client binary, dapp, etc.) can integrate
//! with whatever peer-reputation infrastructure it has.
//!
//! Closes punch-list item #2 (`docs/MAINNET_PUNCHLIST.md`). The encoder side
//! was closed by commit 1fc67c0 (K-04); this module is the missing
//! light-client half of `da_flow.mmd` (nodes L → R).

use crate::block_da_2d::{AvailabilityMetrics, BlockDA2D, BlockDA2DHeader, CellSampleResult};
use crate::commitments::CellProof;

/// Why a peer was marked faulty during sampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerFaultReason {
    /// Peer returned a cell whose Merkle proof did not verify against the
    /// block's row/column commitments.
    InvalidProof,
    /// Peer returned a cell whose `cell_data` does not hash to `cell_hash`.
    /// Subsumes "outright fabrication" — strictly stronger evidence than
    /// `InvalidProof` because it requires no header context.
    HashMismatch,
    /// Peer claimed a cell at coordinates outside the extended-dimension grid.
    OutOfRange,
    /// Peer was unreachable, timed out, or returned a transport-layer error.
    Unreachable,
}

/// Errors a `CellSource` can return per fetch.
#[derive(Debug, Clone)]
pub enum CellSourceError {
    /// Underlying transport failure (HTTP error, libp2p disconnect, etc.).
    Transport(String),
    /// Peer responded but the response was malformed (bad JSON, missing fields).
    Malformed(String),
    /// Peer explicitly reported the cell is unavailable on its side.
    NotFound,
}

/// Source of cells for light-client sampling. Production wires this to an
/// HTTP client hitting `/api/da/cell/:block/:row/:col`; tests use a mock.
///
/// `report_faulty` lets the sampler push reputation signals back to whatever
/// peer-tracking the host owns. The DA crate deliberately stays free of any
/// `evaporchain-network` or `evaporchain-node` dependency, so the hook is
/// abstract.
pub trait CellSource {
    /// Fetch a single cell + its proof at `(row, col)` for the given block.
    /// Returns the cell proof along with an opaque peer identifier the
    /// sampler can pass back to `report_faulty` if verification fails.
    fn fetch_cell(
        &self,
        height: u64,
        row: usize,
        col: usize,
    ) -> Result<(String, CellProof), CellSourceError>;

    /// Mark a peer faulty. Called by the sampler when a returned cell fails
    /// verification. Default impl does nothing — hosts that want
    /// peer-reputation tracking override it.
    fn report_faulty(&self, _peer_id: &str, _reason: PeerFaultReason) {}
}

/// Result of one sampling round.
#[derive(Debug, Clone)]
pub struct SamplingReport {
    /// Per-cell verification outcomes.
    pub results: Vec<CellSampleResult>,
    /// DAS confidence + recovery metrics.
    pub metrics: AvailabilityMetrics,
    /// Peers that returned at least one bad proof during this round, with
    /// the reason for the first fault recorded. Same peer may appear once
    /// even if it served multiple bad cells — light clients should treat
    /// any presence in this list as grounds to drop or score-down the peer.
    pub faulty_peers: Vec<(String, PeerFaultReason)>,
    /// True iff every requested cell was returned + verified. Light clients
    /// can short-circuit on `false` regardless of confidence score: a single
    /// bad cell is itself a fraud signal even if the metrics still look good.
    pub all_valid: bool,
}

impl SamplingReport {
    /// True iff confidence ≥ `threshold` AND no faulty peers were detected.
    /// Default threshold per Celestia DAS analysis: 1 - 2^-15 ≈ 0.99997 with
    /// 15 honest samples. Caller picks the bound; this is just a convenience.
    pub fn passes(&self, threshold: f64) -> bool {
        self.all_valid && self.faulty_peers.is_empty() && self.metrics.confidence >= threshold
    }
}

/// Light-client DA sampler. Generic over any `CellSource`.
pub struct LightClientSampler<S: CellSource> {
    source: S,
    da: BlockDA2D,
}

impl<S: CellSource> LightClientSampler<S> {
    pub fn new(source: S) -> Self {
        Self {
            source,
            da: BlockDA2D::new(),
        }
    }

    /// Underlying cell source. Useful for tests that want to inspect mock
    /// state after a sampling round.
    pub fn source(&self) -> &S {
        &self.source
    }

    /// Run a full sampling round against `header`. Generates `num_samples`
    /// random cell queries (seeded by `seed`), fetches each from the source,
    /// verifies the returned proof, and reports per-peer faults.
    ///
    /// `seed` should be derived deterministically from public block context
    /// (e.g. `blake3(height ‖ data_root ‖ verifier_id)`) so the sampling
    /// pattern is unpredictable to an adversary at block-production time.
    /// See `tendermint.rs:2498-2504` for the producer-side mirror.
    pub fn sample_block(
        &self,
        header: &BlockDA2DHeader,
        height: u64,
        num_samples: usize,
        seed: &[u8],
    ) -> SamplingReport {
        let queries = BlockDA2D::generate_cell_queries(height, header, num_samples, seed);
        let mut results = Vec::with_capacity(queries.len());
        let mut faulty_peers: Vec<(String, PeerFaultReason)> = Vec::new();
        let mut all_valid = true;

        for q in &queries {
            // Defensive: never let a peer convince us to verify a cell at
            // out-of-range coords. The producer-side cell-proof generator
            // would refuse this; the light client must too.
            if q.row >= header.extended_dim || q.col >= header.extended_dim {
                all_valid = false;
                results.push(failed_result(q.row, q.col));
                continue;
            }

            match self.source.fetch_cell(height, q.row, q.col) {
                Ok((peer_id, proof)) => {
                    // Reject any peer that lies about coords — the proof we
                    // verify against is for the queried (row, col), not what
                    // the peer claims it is.
                    if proof.row != q.row || proof.col != q.col {
                        record_fault(&mut faulty_peers, &peer_id, PeerFaultReason::OutOfRange);
                        self.source
                            .report_faulty(&peer_id, PeerFaultReason::OutOfRange);
                        all_valid = false;
                        results.push(failed_result(q.row, q.col));
                        continue;
                    }

                    // First check: does cell_data hash to cell_hash? This
                    // catches outright fabrications without needing the
                    // header at all, so it's strictly stronger evidence.
                    // Uses the domain-tagged leaf hash (audit fix C3).
                    let computed = crate::commitments::leaf_hash(&proof.cell_data);
                    if computed != proof.cell_hash {
                        record_fault(&mut faulty_peers, &peer_id, PeerFaultReason::HashMismatch);
                        self.source
                            .report_faulty(&peer_id, PeerFaultReason::HashMismatch);
                        all_valid = false;
                        results.push(failed_result(q.row, q.col));
                        continue;
                    }

                    // Second check: do the row + column Merkle paths
                    // verify against the header's commitments?
                    let valid = BlockDA2D::verify_cell_proof(header, &proof);
                    if !valid {
                        record_fault(&mut faulty_peers, &peer_id, PeerFaultReason::InvalidProof);
                        self.source
                            .report_faulty(&peer_id, PeerFaultReason::InvalidProof);
                        all_valid = false;
                        results.push(failed_result(q.row, q.col));
                        continue;
                    }

                    results.push(CellSampleResult {
                        row: q.row,
                        col: q.col,
                        cell_hash: hex::encode(proof.cell_hash),
                        row_root: hex::encode(proof.row_root),
                        col_root: hex::encode(proof.col_root),
                        valid: true,
                    });
                }
                Err(_) => {
                    // Transport / not-found is *not* automatically a fault:
                    // a peer can be honest but stale. Caller may want to
                    // retry against another peer. We record it as
                    // unreachable but DO NOT push to `faulty_peers`, since
                    // we have no peer_id to report against.
                    all_valid = false;
                    results.push(failed_result(q.row, q.col));
                }
            }
        }

        let metrics = AvailabilityMetrics::from_samples(&results, header.extended_dim);
        SamplingReport {
            results,
            metrics,
            faulty_peers,
            all_valid,
        }
    }

    /// Convenience accessor for the inner encoder, mainly for tests that
    /// want to construct a synthetic `BlockDA2DPackage` and use its proofs.
    pub fn da(&self) -> &BlockDA2D {
        &self.da
    }
}

fn failed_result(row: usize, col: usize) -> CellSampleResult {
    CellSampleResult {
        row,
        col,
        cell_hash: String::new(),
        row_root: String::new(),
        col_root: String::new(),
        valid: false,
    }
}

fn record_fault(
    faulty: &mut Vec<(String, PeerFaultReason)>,
    peer_id: &str,
    reason: PeerFaultReason,
) {
    if !faulty.iter().any(|(p, _)| p == peer_id) {
        faulty.push((peer_id.to_string(), reason));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_da_2d::{BlockDA2D, BlockDA2DPackage};
    use std::cell::RefCell;

    /// Mock peer set: `(peer_id, behavior)` pairs. Behaviors:
    ///   - Honest: forward the package's real proof
    ///   - Corrupt: return a proof with wrong sibling bytes
    ///   - Fabricate: return a proof with wrong cell_data (hash mismatch)
    enum PeerBehavior {
        Honest,
        Corrupt,
        Fabricate,
    }

    /// Mock CellSource that round-robins across a set of peers and lets
    /// tests assert which peers got marked faulty.
    struct MockCellSource {
        package: BlockDA2DPackage,
        peers: Vec<(String, PeerBehavior)>,
        round_robin: RefCell<usize>,
        faulty_log: RefCell<Vec<(String, PeerFaultReason)>>,
    }

    impl CellSource for MockCellSource {
        fn fetch_cell(
            &self,
            _height: u64,
            row: usize,
            col: usize,
        ) -> Result<(String, CellProof), CellSourceError> {
            let mut idx = self.round_robin.borrow_mut();
            let (peer_id, behavior) = &self.peers[*idx % self.peers.len()];
            *idx += 1;

            let da = BlockDA2D::new();
            let mut proof = da
                .prove_cell(&self.package, row, col)
                .map_err(|e| CellSourceError::Malformed(format!("{e:?}")))?;

            match behavior {
                PeerBehavior::Honest => {}
                PeerBehavior::Corrupt => {
                    if let Some(s) = proof.row_siblings.first_mut() {
                        s[0] ^= 0xff;
                    }
                }
                PeerBehavior::Fabricate => {
                    proof.cell_data = vec![0xAB; proof.cell_data.len()];
                }
            }

            Ok((peer_id.clone(), proof))
        }

        fn report_faulty(&self, peer_id: &str, reason: PeerFaultReason) {
            self.faulty_log
                .borrow_mut()
                .push((peer_id.to_string(), reason));
        }
    }

    fn build_test_package() -> BlockDA2DPackage {
        let da = BlockDA2D::new();
        // Use enough data to fill more than one row so sampling actually
        // exercises distinct (row, col) coords.
        let block_data = vec![0xCDu8; 4096];
        da.encode_block(&block_data).unwrap()
    }

    #[test]
    fn test_all_honest_peers_produce_high_confidence() {
        let package = build_test_package();
        let header = package.header.clone();
        let source = MockCellSource {
            package,
            peers: vec![
                ("peer-a".into(), PeerBehavior::Honest),
                ("peer-b".into(), PeerBehavior::Honest),
                ("peer-c".into(), PeerBehavior::Honest),
            ],
            round_robin: RefCell::new(0),
            faulty_log: RefCell::new(Vec::new()),
        };
        let sampler = LightClientSampler::new(source);

        let report = sampler.sample_block(&header, 1, 16, b"test-seed-honest");

        assert!(
            report.all_valid,
            "honest peers should produce all-valid samples"
        );
        assert!(
            report.faulty_peers.is_empty(),
            "no peer should be marked faulty"
        );
        // 16 valid samples → 1 - 2^-16 ≈ 0.99998 confidence
        assert!(
            report.metrics.confidence > 0.999,
            "confidence too low: {}",
            report.metrics.confidence
        );
        assert_eq!(report.metrics.valid_samples, 16);
        assert!(report.passes(0.99), "sampling should pass 99% threshold");
    }

    #[test]
    fn test_corrupt_peer_marked_faulty() {
        let package = build_test_package();
        let header = package.header.clone();
        let source = MockCellSource {
            package,
            // Single corrupt peer — every fetch returns a tampered proof.
            peers: vec![("peer-evil".into(), PeerBehavior::Corrupt)],
            round_robin: RefCell::new(0),
            faulty_log: RefCell::new(Vec::new()),
        };
        let sampler = LightClientSampler::new(source);

        let report = sampler.sample_block(&header, 1, 8, b"test-seed-corrupt");

        assert!(!report.all_valid);
        assert_eq!(
            report.faulty_peers.len(),
            1,
            "corrupt peer should be in faulty list once"
        );
        assert_eq!(report.faulty_peers[0].0, "peer-evil");
        assert_eq!(report.faulty_peers[0].1, PeerFaultReason::InvalidProof);
        // report_faulty hook should have been called for every bad cell
        let logged = sampler.source().faulty_log.borrow().len();
        assert_eq!(logged, 8, "report_faulty should fire on every bad sample");
        assert!(
            !report.passes(0.99),
            "report with faulty peer must not pass"
        );
    }

    #[test]
    fn test_fabricated_cell_data_caught_before_merkle_check() {
        let package = build_test_package();
        let header = package.header.clone();
        let source = MockCellSource {
            package,
            peers: vec![("peer-liar".into(), PeerBehavior::Fabricate)],
            round_robin: RefCell::new(0),
            faulty_log: RefCell::new(Vec::new()),
        };
        let sampler = LightClientSampler::new(source);

        let report = sampler.sample_block(&header, 1, 4, b"test-seed-fabricate");

        // Hash-mismatch is the strictly stronger fault — it should fire
        // before the Merkle-path check rejects the proof for a different
        // reason. This guards against future verifier-ordering regressions.
        assert!(!report.all_valid);
        assert_eq!(report.faulty_peers[0].1, PeerFaultReason::HashMismatch);
    }

    /// A peer that pretends some cells are simply unavailable. Models a
    /// producer who encodes a block but withholds part of the matrix —
    /// the canonical DA-failure scenario (Celestia §3 Bad-Encoding Fraud).
    /// Sampling should fail probabilistically even though every returned
    /// cell verifies cleanly, because too few samples succeed.
    struct PartiallyWithholdingPeer {
        package: BlockDA2DPackage,
        peer_id: String,
        /// Whether to refuse cell at (row, col). Withholding pattern is
        /// deterministic so the test is reproducible.
        withhold: Box<dyn Fn(usize, usize) -> bool + Send + Sync>,
        faulty_log: RefCell<Vec<(String, PeerFaultReason)>>,
    }

    impl CellSource for PartiallyWithholdingPeer {
        fn fetch_cell(
            &self,
            _height: u64,
            row: usize,
            col: usize,
        ) -> Result<(String, CellProof), CellSourceError> {
            if (self.withhold)(row, col) {
                return Err(CellSourceError::NotFound);
            }
            let da = BlockDA2D::new();
            let proof = da
                .prove_cell(&self.package, row, col)
                .map_err(|e| CellSourceError::Malformed(format!("{e:?}")))?;
            Ok((self.peer_id.clone(), proof))
        }

        fn report_faulty(&self, peer_id: &str, reason: PeerFaultReason) {
            self.faulty_log
                .borrow_mut()
                .push((peer_id.to_string(), reason));
        }
    }

    #[test]
    fn test_partial_withholding_fails_assurance_without_marking_peer_faulty() {
        let package = build_test_package();
        let header = package.header.clone();
        // Withhold roughly the right half of the matrix — peer returns
        // valid proofs for left-half cells, NotFound for right-half.
        let extended_dim = header.extended_dim;
        let half = extended_dim / 2;
        let source = PartiallyWithholdingPeer {
            package,
            peer_id: "peer-stale".into(),
            withhold: Box::new(move |_r, c| c >= half),
            faulty_log: RefCell::new(Vec::new()),
        };
        let sampler = LightClientSampler::new(source);

        let report = sampler.sample_block(&header, 1, 32, b"test-seed-withheld");

        // all_valid is false because some samples were unreachable.
        assert!(!report.all_valid);
        // But the peer is NOT marked faulty — withholding is indistinguishable
        // from honest staleness without cross-peer corroboration. The sampler
        // is right to refuse the block while leaving the peer's reputation
        // intact. (Cross-peer corroboration is the host's responsibility.)
        assert!(
            report.faulty_peers.is_empty(),
            "withholding peer should not be marked faulty without corroboration: {:?}",
            report.faulty_peers
        );
        // Confidence drops because ~half the samples failed.
        assert!(report.metrics.valid_samples < 32);
        assert!(
            report.metrics.confidence < 1.0,
            "confidence should not be saturated when half the cells are withheld",
        );
        // Block must not pass the high-confidence threshold.
        assert!(
            !report.passes(0.999),
            "withheld block must fail HP threshold"
        );
    }

    #[test]
    fn test_sampling_report_passes_requires_all_three_conditions() {
        let confident_clean = SamplingReport {
            results: vec![],
            metrics: AvailabilityMetrics {
                total_samples: 32,
                valid_samples: 32,
                unique_rows_hit: 8,
                unique_cols_hit: 8,
                extended_dim: 8,
                recovery_possible: true,
                confidence: 0.99999,
            },
            faulty_peers: vec![],
            all_valid: true,
        };
        assert!(confident_clean.passes(0.999));

        // Confidence below threshold fails
        let low_conf = SamplingReport {
            metrics: AvailabilityMetrics {
                total_samples: 32,
                valid_samples: 5,
                unique_rows_hit: 2,
                unique_cols_hit: 2,
                extended_dim: 8,
                recovery_possible: false,
                confidence: 0.5,
            },
            ..confident_clean.clone()
        };
        assert!(!low_conf.passes(0.999));

        // all_valid=false fails even with high confidence
        let invalid = SamplingReport {
            all_valid: false,
            ..confident_clean.clone()
        };
        assert!(!invalid.passes(0.999));

        // Any faulty peer fails the gate
        let with_fault = SamplingReport {
            faulty_peers: vec![("bad".to_string(), PeerFaultReason::HashMismatch)],
            ..confident_clean.clone()
        };
        assert!(!with_fault.passes(0.999));
    }

    #[test]
    fn test_peer_fault_reason_variants_distinct() {
        // Eq + Copy on the enum lets the host compare without cloning.
        let a = PeerFaultReason::InvalidProof;
        let b = PeerFaultReason::HashMismatch;
        let c = PeerFaultReason::OutOfRange;
        let d = PeerFaultReason::Unreachable;
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(c, d);
        assert_eq!(a, PeerFaultReason::InvalidProof);
    }

    #[test]
    fn test_default_report_faulty_is_silent_noop() {
        // Hosts that don't override report_faulty get a no-op default. The
        // sampler still functions; the host just learns nothing about peers.
        struct SilentSource;
        impl CellSource for SilentSource {
            fn fetch_cell(
                &self,
                _h: u64,
                _r: usize,
                _c: usize,
            ) -> Result<(String, CellProof), CellSourceError> {
                Err(CellSourceError::NotFound)
            }
            // No override → default no-op is exercised.
        }
        let s = SilentSource;
        // Calling report_faulty must not panic.
        s.report_faulty("anyone", PeerFaultReason::Unreachable);
    }

    #[test]
    fn test_sampler_source_accessor() {
        struct TaggedSource(&'static str);
        impl CellSource for TaggedSource {
            fn fetch_cell(
                &self,
                _h: u64,
                _r: usize,
                _c: usize,
            ) -> Result<(String, CellProof), CellSourceError> {
                Err(CellSourceError::NotFound)
            }
        }
        let sampler = LightClientSampler::new(TaggedSource("hello"));
        assert_eq!(sampler.source().0, "hello");
    }

    #[test]
    fn test_mixed_honest_and_corrupt_peers() {
        let package = build_test_package();
        let header = package.header.clone();
        let source = MockCellSource {
            package,
            // Round-robin: 1 honest, 1 corrupt, 1 honest, 1 corrupt, …
            peers: vec![
                ("peer-good".into(), PeerBehavior::Honest),
                ("peer-bad".into(), PeerBehavior::Corrupt),
            ],
            round_robin: RefCell::new(0),
            faulty_log: RefCell::new(Vec::new()),
        };
        let sampler = LightClientSampler::new(source);

        let report = sampler.sample_block(&header, 1, 16, b"test-seed-mixed");

        assert!(!report.all_valid);
        assert_eq!(report.faulty_peers.len(), 1);
        assert_eq!(report.faulty_peers[0].0, "peer-bad");
        // Half the samples should still be valid (the honest peer's responses).
        assert!(
            report.metrics.valid_samples >= 6,
            "expected ≥6 valid samples from honest half, got {}",
            report.metrics.valid_samples
        );
    }
}
