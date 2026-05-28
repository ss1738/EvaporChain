//! End-to-end integration tests for evaporchain-plc.
//!
//! Non-trivial fixture: "3-block honest chain, adversarial block rejected".
//!
//!   Genesis (height 0): barcode = [b(10,20), b(30,50)].
//!
//!   Block 1 (height 1): barcode = [b(10,21), b(30,50)]. bd_max=2.
//!     d_B(genesis, block1):
//!       match (10,20)<->(10,21): L_inf = max(0,1) = 1.
//!       match (30,50)<->(30,50): L_inf = 0. max = 1. <= 2. PASS.
//!
//!   Block 2 (height 2): barcode = [b(10,21), b(30,52)]. bd_max=3.
//!     d_B(block1, block2):
//!       match (10,21)<->(10,21): 0. match (30,50)<->(30,52): max(0,2)=2. max=2. <= 3. PASS.
//!
//!   Block 3 (height 3): barcode = [b(10,22), b(30,52)]. bd_max=3.
//!     d_B(block2, block3):
//!       match (10,21)<->(10,22): max(0,1)=1. match (30,52)<->(30,52): 0. max=1. <= 3. PASS.
//!
//!   After 3 blocks, client is at height=3, barcode=[b(10,22), b(30,52)].
//!
//!   Adversarial block (height 4): barcode = [b(10,22), b(30,52), b(0,500)].
//!     d_B(block3, adversarial): the extra bar b(0,500) must match either an
//!     existing bar or its own diagonal projection (250,250). The best matching
//!     gives the extra bar distance max(|0-10|, |500-22|) = max(10, 478) = 478
//!     or matching to diagonal at 250: max(|0-22|, |500-52|) = max(22, 448) = 448.
//!     The minimum over all matchings is at least the half-persistence of b(0,500),
//!     which is (500-0)/2 = 250 >> bd_max=5. bd_max=5 -> REJECT.
//!
//!   bd_max=0 edge case: identical barcode passes (d_B=0); any perturbation fails.
//!
//! Doctrine claim (INVENTION_STACK.md §4.3 / PLC):
//! "EvaporChain ships the first L1 with a topological light client. State
//!  is a barcode summary, not a header chain. Each transition is bounded by
//!  the EFH stability theorem (Cohen-Steiner-Edelsbrunner-Harer 2007):
//!  d_B(prev, curr) <= bd_max. A tampered block whose barcode change exceeds
//!  bd_max is rejected even if its hash chain validates. bd_max=0 pins the
//!  state; only incremental honest evolution is accepted."
//!
//! Adversarial fixture: BarcodeHashMismatch, StabilityBoundViolated (exact
//! diagnostic values), NonMonotoneHeight, aborted ingest leaves state unchanged.
//!
//! INVENTION_STACK §4.3: PLC (Topological Light Clients, derives from EFH §9).

use evaporchain_plc::barcode::INF_DEATH;
use evaporchain_plc::{Bar, Barcode, BlockHeader, LightClient, LightClientError};

// -- Helpers ------------------------------------------------------------------

fn b(birth: u64, death: u64) -> Bar {
    Bar::new(birth, death).unwrap()
}

fn make_barcode(bars: Vec<Bar>) -> Barcode {
    Barcode::from_bars(bars)
}

fn header_for(height: u64, bc: &Barcode, bd_max: u64) -> BlockHeader {
    BlockHeader {
        height,
        barcode_hash: bc.hash(),
        bd_max,
    }
}

// -- Non-trivial fixture ------------------------------------------------------

#[test]
fn fixture_3_block_honest_chain() {
    // Genesis.
    let genesis = make_barcode(vec![b(10, 20), b(30, 50)]);
    let mut lc = LightClient::new(0, genesis);

    // Block 1: one bar's death nudges by 1. d_B=1, bd_max=2.
    let bc1 = make_barcode(vec![b(10, 21), b(30, 50)]);
    let h1 = header_for(1, &bc1, 2);
    lc.ingest(&h1, bc1.clone()).unwrap();
    assert_eq!(lc.current_height(), 1);
    assert_eq!(lc.current_barcode(), &bc1);

    // Block 2: other bar's death shifts by 2. d_B=2, bd_max=3.
    let bc2 = make_barcode(vec![b(10, 21), b(30, 52)]);
    let h2 = header_for(2, &bc2, 3);
    lc.ingest(&h2, bc2.clone()).unwrap();
    assert_eq!(lc.current_height(), 2);

    // Block 3: first bar's death nudges by 1. d_B=1, bd_max=3.
    let bc3 = make_barcode(vec![b(10, 22), b(30, 52)]);
    let h3 = header_for(3, &bc3, 3);
    lc.ingest(&h3, bc3.clone()).unwrap();
    assert_eq!(lc.current_height(), 3);
    assert_eq!(lc.current_barcode(), &bc3);
}

#[test]
fn fixture_adversarial_extra_bar_exceeds_bound() {
    // Advance to block 3.
    let genesis = make_barcode(vec![b(10, 20), b(30, 50)]);
    let mut lc = LightClient::new(0, genesis);

    let bc1 = make_barcode(vec![b(10, 21), b(30, 50)]);
    lc.ingest(&header_for(1, &bc1, 2), bc1).unwrap();
    let bc2 = make_barcode(vec![b(10, 21), b(30, 52)]);
    lc.ingest(&header_for(2, &bc2, 3), bc2).unwrap();
    let bc3 = make_barcode(vec![b(10, 22), b(30, 52)]);
    lc.ingest(&header_for(3, &bc3, 3), bc3).unwrap();

    // Adversarial block 4: adds b(0,500). Half-persistence = 250 >> bd_max=5.
    let adv = make_barcode(vec![b(10, 22), b(30, 52), b(0, 500)]);
    let adv_header = header_for(4, &adv, 5);
    let err = lc.ingest(&adv_header, adv).unwrap_err();

    match err {
        LightClientError::StabilityBoundViolated {
            height,
            observed,
            bd_max,
        } => {
            assert_eq!(height, 4);
            assert!(
                observed > 5,
                "adversarial d_B={observed} must exceed bd_max=5"
            );
            assert_eq!(bd_max, 5);
        }
        other => panic!("expected StabilityBoundViolated, got {other:?}"),
    }

    // State stuck at block 3.
    assert_eq!(lc.current_height(), 3);
}

#[test]
fn fixture_bd_max_zero_pins_barcode() {
    // bd_max=0: only the IDENTICAL barcode is accepted (d_B must be 0).
    let genesis = make_barcode(vec![b(10, 20)]);
    let mut lc = LightClient::new(0, genesis.clone());

    // Identical: pass.
    let h1 = header_for(1, &genesis, 0);
    lc.ingest(&h1, genesis.clone()).unwrap();
    assert_eq!(lc.current_height(), 1);

    // Smallest perturbation: death shifts by 1 -> d_B = 1 > 0 -> reject.
    let perturbed = make_barcode(vec![b(10, 21)]);
    let h2 = header_for(2, &perturbed, 0);
    let err = lc.ingest(&h2, perturbed).unwrap_err();
    assert!(matches!(
        err,
        LightClientError::StabilityBoundViolated { .. }
    ));
    // Height pinned at 1.
    assert_eq!(lc.current_height(), 1);
}

// -- Doctrine tests -----------------------------------------------------------

#[test]
fn doctrine_efh_stability_small_perturbation_within_bd_max() {
    // Cohen-Steiner-Edelsbrunner-Harer 2007 stability: a shift of delta
    // in the filtration yields d_B <= delta. Verify with birth-shift of 3.
    let prev = make_barcode(vec![b(10, 20), b(30, 50)]);
    let next = make_barcode(vec![b(13, 20), b(33, 50)]); // births shift by 3

    let mut lc = LightClient::new(0, prev);
    let h1 = header_for(1, &next, 5); // bd_max=5 >= 3
    lc.ingest(&h1, next).unwrap();
    assert_eq!(lc.current_height(), 1);
}

#[test]
fn doctrine_tampered_hash_mismatch() {
    // Adversary presents the correct barcode but a tampered barcode_hash.
    // BarcodeHashMismatch is returned with expected=tampered and actual=real.
    let genesis = make_barcode(vec![b(10, 20)]);
    let mut lc = LightClient::new(0, genesis);

    let next = make_barcode(vec![b(10, 21)]);
    let real_hash = next.hash();
    let mut h1 = header_for(1, &next, 2);
    h1.barcode_hash = [0xAB; 32]; // tamper

    let err = lc.ingest(&h1, next).unwrap_err();
    match err {
        LightClientError::BarcodeHashMismatch { expected, actual } => {
            assert_eq!(expected, [0xAB; 32], "expected = tampered header hash");
            assert_eq!(actual, real_hash, "actual = real barcode hash");
        }
        other => panic!("expected BarcodeHashMismatch, got {other:?}"),
    }
    // State unchanged.
    assert_eq!(lc.current_height(), 0);
}

#[test]
fn doctrine_aborted_ingest_state_unchanged() {
    // Any rejection (hash mismatch OR stability violation) leaves the
    // client in its previous state -- no partial update.
    let genesis = make_barcode(vec![b(10, 20)]);
    let genesis_clone = genesis.clone();
    let mut lc = LightClient::new(0, genesis);

    // Stability violation.
    let huge = make_barcode(vec![b(0, 1000)]);
    let _ = lc.ingest(&header_for(1, &huge, 5), huge);
    assert_eq!(lc.current_height(), 0);
    assert_eq!(lc.current_barcode(), &genesis_clone);
}

// -- Adversarial fixture ------------------------------------------------------

#[test]
fn adversarial_non_monotone_height_rejected() {
    let genesis = make_barcode(vec![b(10, 20)]);
    let mut lc = LightClient::new(5, genesis.clone());

    // Same height.
    let next = make_barcode(vec![b(10, 21)]);
    let err = lc
        .ingest(&header_for(5, &next, 2), next.clone())
        .unwrap_err();
    match err {
        LightClientError::NonMonotoneHeight { incoming, current } => {
            assert_eq!(incoming, 5);
            assert_eq!(current, 5);
        }
        other => panic!("expected NonMonotoneHeight, got {other:?}"),
    }

    // Strictly lower height.
    let err2 = lc.ingest(&header_for(3, &next, 2), next).unwrap_err();
    assert!(matches!(
        err2,
        LightClientError::NonMonotoneHeight {
            incoming: 3,
            current: 5
        }
    ));

    // State unchanged.
    assert_eq!(lc.current_height(), 5);
}

#[test]
fn adversarial_pathological_barcode_correct_hash_still_rejected() {
    // The adversary correctly computes the hash of their pathological
    // barcode, so BarcodeHashMismatch does NOT fire. But d_B >> bd_max,
    // so StabilityBoundViolated fires.
    let genesis = make_barcode(vec![b(10, 20), b(30, 50)]);
    let mut lc = LightClient::new(0, genesis);

    let pathological = make_barcode(vec![b(10, 20), b(30, 50), b(0, 10_000)]);
    // Hash is correct (no hash mismatch), bd_max=5.
    let h1 = header_for(1, &pathological, 5);
    let err = lc.ingest(&h1, pathological).unwrap_err();
    assert!(
        matches!(err, LightClientError::StabilityBoundViolated { .. }),
        "correct hash but exceeding bd_max must yield StabilityBoundViolated"
    );
    assert_eq!(lc.current_height(), 0);
}

#[test]
fn adversarial_unequal_infinite_bar_count_gives_large_distance() {
    // A barcode with an extra infinite bar has INF_DEATH bottleneck distance
    // from one without -- StabilityBoundViolated unless bd_max=INF_DEATH.

    let genesis = make_barcode(vec![b(10, 20)]);
    let mut lc = LightClient::new(0, genesis);

    let with_inf = make_barcode(vec![b(10, 20), b(5, INF_DEATH)]);
    let h1 = header_for(1, &with_inf, 100);
    let err = lc.ingest(&h1, with_inf).unwrap_err();
    match err {
        LightClientError::StabilityBoundViolated { observed, .. } => {
            assert_eq!(
                observed, INF_DEATH,
                "unequal inf count -> INF_DEATH bottleneck"
            );
        }
        other => panic!("expected StabilityBoundViolated, got {other:?}"),
    }
}

// -- Cross-cuts ---------------------------------------------------------------

#[test]
fn serde_round_trip_preserves_light_client() {
    let genesis = make_barcode(vec![b(10, 20), b(30, 50)]);
    let mut lc = LightClient::new(0, genesis);

    let bc1 = make_barcode(vec![b(10, 21), b(30, 50)]);
    lc.ingest(&header_for(1, &bc1, 2), bc1).unwrap();

    let json = serde_json::to_string(&lc).unwrap();
    let back: LightClient = serde_json::from_str(&json).unwrap();
    assert_eq!(lc.current_height(), back.current_height());
    assert_eq!(lc.current_barcode(), back.current_barcode());
}

#[test]
fn serde_round_trip_preserves_block_header() {
    let bc = make_barcode(vec![b(1, 5)]);
    let h = header_for(42, &bc, 10);
    let json = serde_json::to_string(&h).unwrap();
    let back: BlockHeader = serde_json::from_str(&json).unwrap();
    assert_eq!(h, back);
}
