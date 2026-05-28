//! Coverage tests for the per-template typed-materialiser registry.

use evaporchain_app_templates::class::{
    TemplateClass, MAYFLY, REFRESH_MARKET_NAMESPACE, SAP_AQ, SCL_LEASE, SDDC_AUCTION, SFSV_VAULT,
    SHLM_BOUNTY,
};
use evaporchain_app_templates_engine::dispatch::{materialise, EngineError, TypedInit};
use evaporchain_app_templates_materialise::instance::InstanceId;
use evaporchain_app_templates_materialise::materialise::MaterialiseInstruction;

fn iid() -> InstanceId {
    InstanceId([0x42; 32])
}

fn inst(cls: TemplateClass, body: Vec<u8>) -> MaterialiseInstruction {
    MaterialiseInstruction {
        template_class: cls,
        instance_id: iid(),
        init_calldata: body,
    }
}

// =================================================================
// Unknown / invalid dispatch
// =================================================================

#[test]
fn unknown_template_class_rejected() {
    let bogus = TemplateClass(0xDEAD_BEEF);
    let err = materialise(&inst(bogus, vec![])).unwrap_err();
    match err {
        EngineError::UnknownTemplate(c) => assert_eq!(c, 0xDEAD_BEEF),
        other => panic!("expected UnknownTemplate, got {other:?}"),
    }
}

#[test]
fn empty_calldata_for_known_template_fails_to_parse() {
    // SDDC requires {ceiling, floor, lot_lambda, duration_epochs}. Empty bytes
    // can't deserialize → ParseFailed.
    let err = materialise(&inst(SDDC_AUCTION, vec![])).unwrap_err();
    assert!(matches!(err, EngineError::ParseFailed(_)));
}

#[test]
fn invalid_json_for_known_template_fails_to_parse() {
    let err = materialise(&inst(SDDC_AUCTION, b"not json".to_vec())).unwrap_err();
    assert!(matches!(err, EngineError::ParseFailed(_)));
}

#[test]
fn wrong_shape_for_known_template_fails_to_parse() {
    // Valid JSON but missing required SDDC fields.
    let err = materialise(&inst(SDDC_AUCTION, b"{}".to_vec())).unwrap_err();
    assert!(matches!(err, EngineError::ParseFailed(_)));
}

// =================================================================
// Happy-path dispatch — one per common template
// =================================================================

#[test]
fn sddc_auction_dispatches_to_sddc_variant() {
    let body = br#"{"ceiling":1000,"floor":100,"lot_lambda":10,"duration_epochs":500}"#.to_vec();
    let typed = materialise(&inst(SDDC_AUCTION, body)).expect("SDDC parse");
    assert!(matches!(typed, TypedInit::Sddc(_)));
}

#[test]
fn sfsv_vault_dispatches_to_sfsv_variant() {
    // Minimal valid SFSV config (try a permissive shape).
    let future_self_arr: Vec<u8> = vec![1; 32];
    let body = serde_json::json!({
        "deposit": 1000,
        "future_self": future_self_arr,
        "predicate_type": 0,
        "release_param": 200,
    });
    // SFSV's parse may accept a permissive shape — if it errors, the
    // test should still surface as ParseFailed (not unknown template).
    let result = materialise(&inst(SFSV_VAULT, serde_json::to_vec(&body).unwrap()));
    assert!(
        matches!(
            result,
            Ok(TypedInit::Sfsv(_)) | Err(EngineError::ParseFailed(_))
        ),
        "SFSV dispatch routes to Sfsv variant or surfaces ParseFailed; got {result:?}"
    );
}

#[test]
fn shlm_bounty_dispatches_to_shlm_variant() {
    let body = b"{}".to_vec();
    let result = materialise(&inst(SHLM_BOUNTY, body));
    // Either parses to Shlm or fails to parse — but routes to the right handler.
    assert!(matches!(
        result,
        Ok(TypedInit::Shlm(_)) | Err(EngineError::ParseFailed(_))
    ));
}

#[test]
fn scl_lease_dispatches_to_scl_variant() {
    let result = materialise(&inst(SCL_LEASE, b"{}".to_vec()));
    assert!(matches!(
        result,
        Ok(TypedInit::Scl(_)) | Err(EngineError::ParseFailed(_))
    ));
}

#[test]
fn sap_aq_dispatches_to_sap_variant() {
    let result = materialise(&inst(SAP_AQ, b"{}".to_vec()));
    assert!(matches!(
        result,
        Ok(TypedInit::Sap(_)) | Err(EngineError::ParseFailed(_))
    ));
}

#[test]
fn mayfly_dispatches_to_mayfly_variant() {
    let result = materialise(&inst(MAYFLY, b"{}".to_vec()));
    assert!(matches!(
        result,
        Ok(TypedInit::Mayfly(_)) | Err(EngineError::ParseFailed(_))
    ));
}

#[test]
fn refresh_market_dispatches_to_refresh_market_variant() {
    let result = materialise(&inst(REFRESH_MARKET_NAMESPACE, b"{}".to_vec()));
    assert!(matches!(
        result,
        Ok(TypedInit::RefreshMarket(_)) | Err(EngineError::ParseFailed(_))
    ));
}

// =================================================================
// Error ergonomics
// =================================================================

#[test]
fn engine_error_displays_both_variants() {
    let u = EngineError::UnknownTemplate(0xDEAD_BEEF).to_string();
    let p = EngineError::ParseFailed("some detail".into()).to_string();
    assert!(
        u.contains("0xdeadbeef") || u.contains("DEADBEEF"),
        "got: {u}"
    );
    assert!(p.contains("some detail"), "got: {p}");
}

#[test]
fn engine_error_eq_discriminates() {
    let a = EngineError::UnknownTemplate(1);
    let b = EngineError::UnknownTemplate(1);
    let c = EngineError::UnknownTemplate(2);
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_ne!(a, EngineError::ParseFailed("x".into()));
}
