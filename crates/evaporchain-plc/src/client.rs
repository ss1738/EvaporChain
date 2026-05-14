//! `LightClient` — the topological light-client state machine.
//!
//! Tracks the most recent barcode, accepts new blocks if they
//! prove `d_B(prev, curr) ≤ bd_max` and the barcode hash
//! matches the block header's commitment.
#![allow(clippy::doc_overindented_list_items)]

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::barcode::Barcode;
use crate::bottleneck::bottleneck_distance;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LightClientError {
    #[error("barcode hash mismatch: header committed {expected:?}, supplied barcode hashes to {actual:?}")]
    BarcodeHashMismatch {
        expected: [u8; 32],
        actual: [u8; 32],
    },
    #[error(
        "stability bound violated at height {height}: bottleneck distance {observed} > committed bd_max {bd_max}"
    )]
    StabilityBoundViolated {
        height: u64,
        observed: u64,
        bd_max: u64,
    },
    #[error("non-monotone height: incoming {incoming} ≤ current {current}")]
    NonMonotoneHeight { incoming: u64, current: u64 },
}

/// Block-header data the light client cares about. The full
/// chain header is much wider; this is the projection the
/// topological client tracks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    pub height: u64,
    /// Domain-tagged hash of the barcode at this block.
    pub barcode_hash: [u8; 32],
    /// The chain's *committed* upper bound on the bottleneck
    /// distance from this block's barcode to the previous block's
    /// barcode. Provided by the chain; the light client trusts
    /// the validator set committed to it (validated upstream by
    /// the consensus layer that produces the header).
    pub bd_max: u64,
}

/// Topological light-client state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightClient {
    current_height: u64,
    current_barcode: Barcode,
}

impl LightClient {
    /// Initialise from a trusted genesis barcode (e.g., from a
    /// snapshot or a hard-coded genesis commitment).
    pub fn new(genesis_height: u64, genesis_barcode: Barcode) -> Self {
        Self {
            current_height: genesis_height,
            current_barcode: genesis_barcode,
        }
    }

    pub fn current_height(&self) -> u64 {
        self.current_height
    }

    pub fn current_barcode(&self) -> &Barcode {
        &self.current_barcode
    }

    /// Ingest a new block. Verifies:
    /// 1. Height is strictly greater than the current height.
    /// 2. The supplied barcode's domain-tagged hash matches the
    ///    header's `barcode_hash`.
    /// 3. The bottleneck distance from `current_barcode` to the
    ///    new barcode does not exceed `header.bd_max`.
    ///
    /// On success, advances the state.
    pub fn ingest(
        &mut self,
        header: &BlockHeader,
        new_barcode: Barcode,
    ) -> Result<(), LightClientError> {
        if header.height <= self.current_height {
            return Err(LightClientError::NonMonotoneHeight {
                incoming: header.height,
                current: self.current_height,
            });
        }

        let actual_hash = new_barcode.hash();
        if actual_hash != header.barcode_hash {
            return Err(LightClientError::BarcodeHashMismatch {
                expected: header.barcode_hash,
                actual: actual_hash,
            });
        }

        let observed = bottleneck_distance(&self.current_barcode, &new_barcode);
        if observed > header.bd_max {
            return Err(LightClientError::StabilityBoundViolated {
                height: header.height,
                observed,
                bd_max: header.bd_max,
            });
        }

        self.current_barcode = new_barcode;
        self.current_height = header.height;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::barcode::{Bar, Barcode};

    fn b(birth: u64, death: u64) -> Bar {
        Bar::new(birth, death).unwrap()
    }

    fn make_header(height: u64, bc: &Barcode, bd_max: u64) -> BlockHeader {
        BlockHeader {
            height,
            barcode_hash: bc.hash(),
            bd_max,
        }
    }

    #[test]
    fn new_client_holds_genesis() {
        let genesis = Barcode::from_bars(vec![b(1, 10)]);
        let lc = LightClient::new(0, genesis.clone());
        assert_eq!(lc.current_height(), 0);
        assert_eq!(lc.current_barcode(), &genesis);
    }

    #[test]
    fn ingest_advances_state_on_valid_block() {
        let genesis = Barcode::from_bars(vec![b(1, 10)]);
        let mut lc = LightClient::new(0, genesis);

        let next = Barcode::from_bars(vec![b(1, 11)]);
        let header = make_header(1, &next, 1);
        lc.ingest(&header, next.clone()).unwrap();

        assert_eq!(lc.current_height(), 1);
        assert_eq!(lc.current_barcode(), &next);
    }

    #[test]
    fn ingest_rejects_non_monotone_height() {
        let genesis = Barcode::from_bars(vec![b(1, 10)]);
        let mut lc = LightClient::new(5, genesis);
        let next = Barcode::from_bars(vec![b(1, 11)]);
        let header = make_header(5, &next, 1); // same height
        let err = lc.ingest(&header, next).unwrap_err();
        assert!(matches!(
            err,
            LightClientError::NonMonotoneHeight {
                incoming: 5,
                current: 5
            }
        ));
    }

    #[test]
    fn ingest_rejects_hash_mismatch() {
        let genesis = Barcode::from_bars(vec![b(1, 10)]);
        let mut lc = LightClient::new(0, genesis);

        let claimed = Barcode::from_bars(vec![b(1, 11)]);
        let mut header = make_header(1, &claimed, 1);
        // Tamper: change the committed hash.
        header.barcode_hash = [0xFF; 32];
        let err = lc.ingest(&header, claimed).unwrap_err();
        assert!(matches!(err, LightClientError::BarcodeHashMismatch { .. }));
    }

    #[test]
    fn ingest_rejects_stability_violation() {
        // Genesis: tight barcode. New block: pathological — a
        // huge new bar that exceeds bd_max.
        let genesis = Barcode::from_bars(vec![b(10, 20)]);
        let mut lc = LightClient::new(0, genesis);

        let pathological = Barcode::from_bars(vec![b(10, 20), b(0, 1000)]);
        let header = make_header(1, &pathological, 5); // bd_max = 5
        let err = lc.ingest(&header, pathological).unwrap_err();
        assert!(matches!(
            err,
            LightClientError::StabilityBoundViolated { .. }
        ));
        // State unchanged.
        assert_eq!(lc.current_height(), 0);
    }

    #[test]
    fn ingest_accepts_within_stability_bound() {
        let genesis = Barcode::from_bars(vec![b(10, 20)]);
        let mut lc = LightClient::new(0, genesis);

        // Small perturbation — within bd_max.
        let small_perturb = Barcode::from_bars(vec![b(10, 22)]);
        let header = make_header(1, &small_perturb, 3);
        lc.ingest(&header, small_perturb).unwrap();
        assert_eq!(lc.current_height(), 1);
    }

    #[test]
    fn aborted_ingest_leaves_state_unchanged() {
        let genesis = Barcode::from_bars(vec![b(10, 20)]);
        let mut lc = LightClient::new(0, genesis.clone());

        let pathological = Barcode::from_bars(vec![b(0, 1000)]);
        let header = make_header(1, &pathological, 5);
        let _ = lc.ingest(&header, pathological);
        assert_eq!(lc.current_height(), 0);
        assert_eq!(lc.current_barcode(), &genesis);
    }

    /// T1.20 — `ingest` rejects a strictly-lower height too, not
    /// just equal. The existing `ingest_rejects_non_monotone_height`
    /// uses `incoming == current`; this pins the strictly-less
    /// boundary so a refactor to `<` (instead of `<=`) would trip.
    #[test]
    fn t1_20_ingest_rejects_strictly_lower_height() {
        let genesis = Barcode::from_bars(vec![b(1, 10)]);
        let mut lc = LightClient::new(10, genesis);
        let next = Barcode::from_bars(vec![b(1, 11)]);
        let header = make_header(7, &next, 1); // height < current
        let err = lc.ingest(&header, next).unwrap_err();
        match err {
            LightClientError::NonMonotoneHeight { incoming, current } => {
                assert_eq!(incoming, 7);
                assert_eq!(current, 10);
            }
            other => panic!("expected NonMonotoneHeight, got {other:?}"),
        }
        // State unchanged.
        assert_eq!(lc.current_height(), 10);
    }

    /// T1.20 — each error variant carries diagnostic values that
    /// operators rely on for triage. The existing rejection tests
    /// use `matches!` without checking field values; this pins
    /// the values explicitly for all three variants.
    #[test]
    fn t1_20_error_variants_carry_diagnostic_values() {
        // StabilityBoundViolated carries height + observed + bd_max.
        let genesis = Barcode::from_bars(vec![b(10, 20)]);
        let mut lc = LightClient::new(0, genesis);
        let pathological = Barcode::from_bars(vec![b(10, 20), b(0, 1000)]);
        let header = make_header(1, &pathological, 5);
        let err = lc.ingest(&header, pathological).unwrap_err();
        match err {
            LightClientError::StabilityBoundViolated {
                height,
                observed,
                bd_max,
            } => {
                assert_eq!(height, 1);
                assert!(observed > 5, "observed={observed} must exceed bd_max=5");
                assert_eq!(bd_max, 5);
            }
            other => panic!("expected StabilityBoundViolated, got {other:?}"),
        }

        // BarcodeHashMismatch carries expected + actual.
        let mut lc = LightClient::new(0, Barcode::from_bars(vec![b(1, 10)]));
        let claimed = Barcode::from_bars(vec![b(1, 11)]);
        let mut header = make_header(1, &claimed, 5);
        let real_hash = claimed.hash();
        header.barcode_hash = [0xFFu8; 32];
        let err = lc.ingest(&header, claimed).unwrap_err();
        match err {
            LightClientError::BarcodeHashMismatch { expected, actual } => {
                assert_eq!(expected, [0xFFu8; 32], "expected = tampered header hash");
                assert_eq!(actual, real_hash, "actual = real barcode hash");
            }
            other => panic!("expected BarcodeHashMismatch, got {other:?}"),
        }
    }

    /// T1.20 — boundary: `bd_max = 0` permits ONLY a barcode
    /// identical to the previous one (bottleneck distance = 0).
    /// Any change, however small, must be rejected. Pins the
    /// strictness of the EFH stability gate.
    #[test]
    fn t1_20_bd_max_zero_requires_identical_barcode() {
        let genesis = Barcode::from_bars(vec![b(10, 20)]);
        let mut lc = LightClient::new(0, genesis.clone());

        // Identical barcode → must accept (d_B = 0).
        let header_identical = make_header(1, &genesis, 0);
        lc.ingest(&header_identical, genesis.clone()).unwrap();
        assert_eq!(lc.current_height(), 1);

        // Any change → must reject.
        let changed = Barcode::from_bars(vec![b(10, 21)]);
        let header = make_header(2, &changed, 0);
        let err = lc.ingest(&header, changed).unwrap_err();
        assert!(matches!(
            err,
            LightClientError::StabilityBoundViolated { .. }
        ));
        // State stuck at height 1.
        assert_eq!(lc.current_height(), 1);
    }

    #[test]
    fn the_press_claim_lives_as_a_test() {
        // Claim: "EvaporChain ships the first L1 with a
        // topological light client. State is a barcode
        // summary, not a header chain. Each transition is
        // bounded by EFH stability — a tampered block whose
        // barcode change exceeds bd_max is rejected even
        // though its hash chain validates."
        let genesis = Barcode::from_bars(vec![b(10, 20), b(30, 50)]);
        let mut lc = LightClient::new(0, genesis);

        // Three honest small-perturbation blocks. Each within bd_max=2.
        for h in 1..=3 {
            let bc = Barcode::from_bars(vec![b(10 + h, 20), b(30, 50 + h)]);
            let header = make_header(h, &bc, 2);
            lc.ingest(&header, bc).unwrap();
        }
        assert_eq!(lc.current_height(), 3);

        // Adversary submits a block whose hash IS the right hash
        // of a pathological barcode, but the bottleneck distance
        // exceeds bd_max=2. Rejected.
        let pathological = Barcode::from_bars(vec![b(0, 500), b(30, 50)]);
        let header = make_header(4, &pathological, 2);
        let err = lc.ingest(&header, pathological).unwrap_err();
        assert!(matches!(
            err,
            LightClientError::StabilityBoundViolated { .. }
        ));
        // State stuck at height 3 — chain hasn't moved for this client.
        assert_eq!(lc.current_height(), 3);
    }
}
