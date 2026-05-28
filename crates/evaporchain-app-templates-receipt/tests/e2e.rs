//! §Deploy-receipt layer e2e
//!
//! Scenario: "EvaporChain dApp deploy confirmation" — NADIA submits
//! four deploy requests. The chain lands them at block height 5_000
//! and emits a DeployReceipt for each. NADIA's dApp polls by the
//! original request commitment to confirm each deploy landed.
//!
//! The suite proves: receipts carry the request commitment as a
//! back-pointer; event_id is BLAKE3(canonical_bytes); domain
//! separation prevents collisions with request / instance hashes;
//! out-of-range classes are rejected; layout is deterministically
//! ordered.

use evaporchain_app_templates::class::{CHILDKEY_LETTER, MAYFLY, MNEMOCHAIN_CARD, SINGH_SABI};
use evaporchain_app_templates_receipt::{DeployReceipt, ReceiptError, RECEIPT_DOMAIN_TAG};

fn nadia() -> [u8; 32] {
    [0x4A; 32]
}

fn receipt(
    commitment: [u8; 32],
    instance_id: [u8; 32],
    class: evaporchain_app_templates::TemplateClass,
    fee: u64,
    epoch: u64,
    height: u64,
) -> DeployReceipt {
    DeployReceipt::new(commitment, instance_id, class, nadia(), fee, epoch, height).unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn event_id_is_blake3_of_canonical_bytes() {
    let r = receipt([0x11; 32], [0x22; 32], SINGH_SABI, 1_400, 500, 5_000);
    let expected: [u8; 32] = *blake3::hash(&r.canonical_bytes()).as_bytes();
    assert_eq!(
        r.event_id(),
        expected,
        "event_id must be BLAKE3(canonical_bytes)"
    );
}

#[test]
fn canonical_bytes_start_with_domain_tag() {
    let r = receipt([0x11; 32], [0x22; 32], MAYFLY, 1_200, 100, 5_001);
    assert!(
        r.canonical_bytes().starts_with(RECEIPT_DOMAIN_TAG),
        "canonical bytes must open with the receipt domain-separation tag"
    );
}

#[test]
fn canonical_bytes_are_deterministic() {
    let r = receipt([0xAA; 32], [0xBB; 32], SINGH_SABI, 1_400, 500, 5_000);
    assert_eq!(
        r.canonical_bytes(),
        r.canonical_bytes(),
        "canonical bytes must be deterministic"
    );
}

#[test]
fn canonical_bytes_are_fixed_width() {
    // Receipt has no variable-shape data: total = tag + 32+32+4+32+8+8+8 = tag+124.
    let r = receipt([0; 32], [0; 32], MAYFLY, 0, 0, 0);
    assert_eq!(
        r.canonical_bytes().len(),
        RECEIPT_DOMAIN_TAG.len() + 124,
        "receipt canonical bytes must be fixed-width"
    );
}

#[test]
fn dapp_polls_receipt_by_commitment() {
    // NADIA knows her request commitment; she finds the receipt by matching it.
    let req_commitment = [0xC0; 32];
    let r = receipt(
        req_commitment,
        [0x22; 32],
        MNEMOCHAIN_CARD,
        1_250,
        300,
        5_002,
    );
    assert_eq!(
        r.commitment, req_commitment,
        "receipt must carry the original request commitment for polling"
    );
}

#[test]
fn different_commitments_produce_different_event_ids() {
    let r1 = receipt([0xAA; 32], [0x22; 32], SINGH_SABI, 1_400, 500, 5_000);
    let r2 = receipt([0xBB; 32], [0x22; 32], SINGH_SABI, 1_400, 500, 5_000);
    assert_ne!(
        r1.event_id(),
        r2.event_id(),
        "different commitments must produce different event IDs"
    );
}

#[test]
fn different_block_heights_produce_different_event_ids() {
    // Same deploy, different finalization height → different event ID.
    let r1 = receipt([0xAA; 32], [0x22; 32], MAYFLY, 1_200, 100, 5_000);
    let r2 = receipt([0xAA; 32], [0x22; 32], MAYFLY, 1_200, 100, 5_001);
    assert_ne!(
        r1.event_id(),
        r2.event_id(),
        "block height must participate in the event ID"
    );
}

#[test]
fn different_classes_produce_different_event_ids() {
    let r1 = receipt([0xAA; 32], [0x22; 32], SINGH_SABI, 1_400, 500, 5_000);
    let r2 = receipt([0xAA; 32], [0x22; 32], MAYFLY, 1_400, 500, 5_000);
    assert_ne!(r1.event_id(), r2.event_id());
}

#[test]
fn domain_tag_separates_from_naive_hash() {
    // event_id must differ from BLAKE3(fields_without_tag).
    let r = receipt([0x11; 32], [0x22; 32], SINGH_SABI, 1_400, 500, 5_000);
    let tagged = r.event_id();
    let mut naive = blake3::Hasher::new();
    naive.update(&r.commitment);
    naive.update(&r.instance_id);
    naive.update(&r.template_class.0.to_le_bytes());
    naive.update(&r.deployer);
    naive.update(&r.fee_paid.to_le_bytes());
    naive.update(&r.deployed_at_epoch.to_le_bytes());
    naive.update(&r.block_height.to_le_bytes());
    let untagged: [u8; 32] = *naive.finalize().as_bytes();
    assert_ne!(
        tagged, untagged,
        "RECEIPT_DOMAIN_TAG must cause the event_id to differ from a naive hash"
    );
}

#[test]
fn out_of_range_class_rejected() {
    let err = DeployReceipt::new(
        [0; 32],
        [0; 32],
        evaporchain_app_templates::TemplateClass(0xFFFF_FFFF),
        nadia(),
        0,
        0,
        0,
    )
    .unwrap_err();
    assert_eq!(err, ReceiptError::OutOfRange(0xFFFF_FFFF));
}

#[test]
fn serde_round_trip() {
    let r = receipt([0x11; 32], [0x22; 32], CHILDKEY_LETTER, 1_300, 200, 4_999);
    let json = serde_json::to_string(&r).unwrap();
    let back: DeployReceipt = serde_json::from_str(&json).unwrap();
    assert_eq!(
        r, back,
        "DeployReceipt must serialise and deserialise exactly"
    );
}

#[test]
fn nadia_batch_deploy_confirmation_arc() {
    // Full arc: NADIA's four deploys each land at block 5_000.
    // Each receipt has a unique event_id; each is findable by its commitment.
    let commitments: [[u8; 32]; 4] = [[0xA1; 32], [0xA2; 32], [0xA3; 32], [0xA4; 32]];
    let classes = [SINGH_SABI, MAYFLY, MNEMOCHAIN_CARD, CHILDKEY_LETTER];
    let receipts: Vec<DeployReceipt> = (0..4)
        .map(|i| {
            receipt(
                commitments[i],
                [i as u8 + 0x10; 32],
                classes[i],
                1_200 + i as u64 * 100,
                5_000,
                5_000 + i as u64,
            )
        })
        .collect();

    // All event IDs unique.
    let ids: std::collections::HashSet<_> = receipts.iter().map(|r| r.event_id()).collect();
    assert_eq!(
        ids.len(),
        4,
        "all four receipts must have distinct event IDs"
    );

    // Each receipt's commitment matches the original request commitment.
    for (i, r) in receipts.iter().enumerate() {
        assert_eq!(
            r.commitment, commitments[i],
            "receipt[{i}] commitment mismatch"
        );
    }
}
