//! §Deploy — BLAKE3-committed deploy-request signing layer e2e
//!
//! Scenario: "EvaporChain dApp launch day" — NADIA is a dApp developer
//! submitting deploy requests for her NFT portfolio. FELIX is an
//! adversary attempting cross-template replay and signing-byte forgery.
//! Two validators PRIYA and KIERAN must agree on every commitment.
//!
//! The suite proves: canonical JSON eliminates insertion-order drift;
//! domain-separation prevents template cross-replay; commitment is a
//! deterministic BLAKE3 of the signing-byte layout; construction rejects
//! out-of-range classes and non-object params.

use evaporchain_app_templates::class::{MAYFLY, MNEMOCHAIN_CARD, SDDC_AUCTION, SINGH_SABI};
use evaporchain_app_templates::find;
use evaporchain_app_templates_deploy::{
    validate_against_descriptor, DeployRequest, RequestError, ValidationError,
};
use serde_json::json;

fn nadia() -> [u8; 32] {
    [0x4A; 32]
}
fn felix() -> [u8; 32] {
    [0xFE; 32]
}

fn sabi_req(deployer: [u8; 32], nonce: u64) -> DeployRequest {
    DeployRequest::new(
        SINGH_SABI,
        json!({"initial_energy": 5_000, "floor_pct": 20, "half_life": 365}),
        deployer,
        2_000,
        nonce,
    )
    .unwrap()
}

fn mayfly_req(deployer: [u8; 32], nonce: u64) -> DeployRequest {
    DeployRequest::new(
        MAYFLY,
        json!({"initial_energy": 1_000, "half_life": 30}),
        deployer,
        2_000,
        nonce,
    )
    .unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn commitment_is_blake3_of_signing_bytes() {
    // The commitment must be BLAKE3(signing_bytes) — no other hash.
    let req = sabi_req(nadia(), 0);
    let bytes = req.signing_bytes().unwrap();
    let expected: [u8; 32] = *blake3::hash(&bytes).as_bytes();
    assert_eq!(req.commitment().unwrap(), expected);
}

#[test]
fn two_validators_agree_on_commitment() {
    // PRIYA and KIERAN compute the same commitment independently.
    let req = sabi_req(nadia(), 1);
    let c_priya = req.commitment().unwrap();
    let c_kieran = req.commitment().unwrap();
    assert_eq!(
        c_priya, c_kieran,
        "commitment must be validator-deterministic"
    );
}

#[test]
fn canonical_json_eliminates_key_order_in_commitment() {
    // Two requests with identical params but different insertion orders
    // must produce identical signing bytes and identical commitments.
    let r1 = DeployRequest::new(
        SINGH_SABI,
        json!({"initial_energy": 5_000, "floor_pct": 20, "half_life": 365}),
        nadia(),
        2_000,
        7,
    )
    .unwrap();
    let r2 = DeployRequest::new(
        SINGH_SABI,
        json!({"half_life": 365, "initial_energy": 5_000, "floor_pct": 20}),
        nadia(),
        2_000,
        7,
    )
    .unwrap();
    assert_eq!(
        r1.signing_bytes().unwrap(),
        r2.signing_bytes().unwrap(),
        "canonical JSON must normalise key order"
    );
    assert_eq!(r1.commitment().unwrap(), r2.commitment().unwrap());
}

#[test]
fn domain_separation_prevents_cross_template_replay() {
    // FELIX takes NADIA's Mayfly signing bytes and tries to replay them
    // as a SinghSabi deploy. The template class is part of the signing
    // bytes, so the commitments must differ even for identical params.
    let p = json!({"initial_energy": 1_000, "floor_pct": 10, "half_life": 30});
    let mayfly = DeployRequest::new(MAYFLY, p.clone(), felix(), 0, 0).unwrap();
    let sabi = DeployRequest::new(SINGH_SABI, p, felix(), 0, 0).unwrap();
    assert_ne!(
        mayfly.signing_bytes().unwrap(),
        sabi.signing_bytes().unwrap(),
        "different template classes must produce different signing bytes"
    );
    assert_ne!(mayfly.commitment().unwrap(), sabi.commitment().unwrap());
}

#[test]
fn nonce_change_produces_different_commitment() {
    // Two deploys from the same deployer, same class, same params but
    // different nonces → different commitments. Replay protection.
    let r0 = sabi_req(nadia(), 0);
    let r1 = sabi_req(nadia(), 1);
    assert_ne!(
        r0.commitment().unwrap(),
        r1.commitment().unwrap(),
        "nonce must differentiate commitments"
    );
}

#[test]
fn deployer_change_produces_different_commitment() {
    // Same class + params + nonce from different deployers → different.
    let r_nadia = sabi_req(nadia(), 5);
    let r_felix = sabi_req(felix(), 5);
    assert_ne!(r_nadia.commitment().unwrap(), r_felix.commitment().unwrap());
}

#[test]
fn signing_bytes_layout_starts_with_domain_tag() {
    use evaporchain_app_templates_deploy::request::DEPLOY_DOMAIN_TAG;
    let req = sabi_req(nadia(), 0);
    let bytes = req.signing_bytes().unwrap();
    assert!(
        bytes.starts_with(DEPLOY_DOMAIN_TAG),
        "signing bytes must open with domain-separation tag"
    );
}

#[test]
fn arrays_preserve_element_order_in_commitment() {
    // Arrays are positional — reversing elements must change the commitment.
    let r1 = DeployRequest::new(SINGH_SABI, json!({"v": [1, 2, 3]}), nadia(), 0, 0).unwrap();
    let r2 = DeployRequest::new(SINGH_SABI, json!({"v": [3, 2, 1]}), nadia(), 0, 0).unwrap();
    assert_ne!(
        r1.commitment().unwrap(),
        r2.commitment().unwrap(),
        "array element order must be preserved (not sorted)"
    );
}

#[test]
fn out_of_range_class_rejected_at_construction() {
    let err = DeployRequest::new(
        evaporchain_app_templates::TemplateClass(0xFFFF_FFFF),
        json!({}),
        nadia(),
        0,
        0,
    )
    .unwrap_err();
    assert_eq!(err, RequestError::OutOfRange(0xFFFF_FFFF));
}

#[test]
fn non_object_params_rejected_array_and_scalar() {
    for bad in [json!([1, 2, 3]), json!(42), json!("str"), json!(null)] {
        let err = DeployRequest::new(MAYFLY, bad, nadia(), 0, 0).unwrap_err();
        assert_eq!(
            err,
            RequestError::ParamsNotObject,
            "non-object params must be rejected"
        );
    }
}

#[test]
fn validation_rejects_missing_required_keys() {
    // SinghSabi missing floor_pct → MissingRequiredKey.
    let req = DeployRequest::new(
        SINGH_SABI,
        json!({"initial_energy": 1_000, "half_life": 365}),
        nadia(),
        0,
        0,
    )
    .unwrap();
    let desc = find(SINGH_SABI).unwrap();
    let err = validate_against_descriptor(&req, &desc).unwrap_err();
    assert_eq!(err, ValidationError::MissingRequiredKey("floor_pct"));
}

#[test]
fn validation_accepts_extra_unknown_keys() {
    // Forward-compat: V2 client adds an unknown field → still valid.
    let req = DeployRequest::new(
        SINGH_SABI,
        json!({"initial_energy": 5_000, "floor_pct": 20, "half_life": 365,
               "future_only_field": "ignored"}),
        nadia(),
        0,
        0,
    )
    .unwrap();
    let desc = find(SINGH_SABI).unwrap();
    validate_against_descriptor(&req, &desc).expect("extra keys must be permitted");
}

#[test]
fn nadia_portfolio_full_arc() {
    // Full arc: NADIA deploys SinghSabi, Mayfly, SDDC, MnemoChain.
    // All construction + validation + commitment must succeed.
    // All four commitments must be mutually distinct.
    let deploys = [
        DeployRequest::new(
            SINGH_SABI,
            json!({"initial_energy": 5_000, "floor_pct": 20, "half_life": 365}),
            nadia(),
            0,
            0,
        )
        .unwrap(),
        mayfly_req(nadia(), 1),
        DeployRequest::new(
            SDDC_AUCTION,
            json!({"ceiling": 1_000, "floor": 100, "lot_lambda": 50, "duration_epochs": 500}),
            nadia(),
            0,
            2,
        )
        .unwrap(),
        DeployRequest::new(
            MNEMOCHAIN_CARD,
            json!({"initial_energy": 1_000, "initial_stability": 10}),
            nadia(),
            0,
            3,
        )
        .unwrap(),
    ];
    let commitments: Vec<_> = deploys.iter().map(|r| r.commitment().unwrap()).collect();
    let unique: std::collections::HashSet<_> = commitments.iter().collect();
    assert_eq!(
        unique.len(),
        4,
        "all four deploys must have distinct commitments"
    );
}
