//! §App-Templates engine e2e
//!
//! Scenario: "PRIYA contract engine dispatch round" — PRIYA is the
//! chain's ContractEngine. She receives MaterialiseInstructions off
//! the queue and dispatches each to its registered typed handler.
//! She spot-checks templates from each of the six lanes, rejects
//! unknown classes, rejects malformed JSON, and rejects calldata that
//! exceeds the per-deploy cap (SUB-N2 — prevents pre-gas parse-tree
//! allocation).
//!
//! The suite proves: all six lanes have at least one template that
//! dispatches to the correct TypedInit variant; unknown class →
//! EngineError::UnknownTemplate; malformed JSON → ParseFailed;
//! calldata > MAX_INIT_CALLDATA → CalldataTooLarge; dispatch is
//! deterministic.

use evaporchain_app_templates::class::{
    GALLERY_FORGETS, MAYFLY, MNEMOCHAIN_CARD, SGB_TYPE_SYSTEM,
    SDDC_AUCTION, SINGH_SABI, CHILDKEY_LETTER,
};
use evaporchain_app_templates::TemplateClass;
use evaporchain_app_templates_engine::{materialise, EngineError, TypedInit};
use evaporchain_app_templates_engine::dispatch::MAX_INIT_CALLDATA;
use evaporchain_app_templates_materialise::MaterialiseInstruction;

fn iid(b: u8) -> evaporchain_app_templates_materialise::InstanceId {
    let mut id = [0u8; 32];
    id[0] = b;
    evaporchain_app_templates_materialise::InstanceId(id)
}

fn instr(class: TemplateClass, calldata: Vec<u8>, idx: u8) -> MaterialiseInstruction {
    MaterialiseInstruction {
        template_class: class,
        instance_id: iid(idx),
        init_calldata: calldata,
    }
}

fn mayfly_cd() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "initial_energy": 1_000u64,
        "half_life": 100u64,
    })).unwrap()
}

fn sabi_cd() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "initial_energy": 5_000u64,
        "floor_pct": 10u64,
        "half_life": 200u64,
    })).unwrap()
}

fn sddc_cd() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "ceiling": 10_000u64,
        "floor": 1_000u64,
        "lot_lambda": 50u64,
        "duration_epochs": 500u64,
    })).unwrap()
}

fn mnemochain_cd() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "initial_energy": 2_000u64,
        "initial_stability": 86400u64,
    })).unwrap()
}

fn gallery_cd() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "opened_at_epoch": 1000u64,
    })).unwrap()
}

fn sgb_cd() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "fragment": "!A ⊗ ?B",
    })).unwrap()
}

fn childkey_cd() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "unlock_age_years": 18u64,
        "epochs_per_year": 365u64,
        "m_threshold": 2u64,
        "n_committee": 3u64,
    })).unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn nft_lane_mayfly_dispatches() {
    let typed = materialise(&instr(MAYFLY, mayfly_cd(), 0x01)).unwrap();
    assert!(matches!(typed, TypedInit::Mayfly(_)),
        "MAYFLY must dispatch to Mayfly variant: {:?}", typed);
}

#[test]
fn nft_lane_singh_sabi_dispatches() {
    let typed = materialise(&instr(SINGH_SABI, sabi_cd(), 0x02)).unwrap();
    assert!(matches!(typed, TypedInit::SinghSabi(_)),
        "SINGH_SABI must dispatch to SinghSabi variant: {:?}", typed);
}

#[test]
fn marketplace_lane_sddc_dispatches() {
    let typed = materialise(&instr(SDDC_AUCTION, sddc_cd(), 0x03)).unwrap();
    assert!(matches!(typed, TypedInit::Sddc(_)),
        "SDDC_AUCTION must dispatch to Sddc variant: {:?}", typed);
}

#[test]
fn consumer_lane_mnemochain_dispatches() {
    let typed = materialise(&instr(MNEMOCHAIN_CARD, mnemochain_cd(), 0x04)).unwrap();
    assert!(matches!(typed, TypedInit::Mnemochain(_)),
        "MNEMOCHAIN_CARD must dispatch to Mnemochain variant: {:?}", typed);
}

#[test]
fn cultural_lane_gallery_dispatches() {
    let typed = materialise(&instr(GALLERY_FORGETS, gallery_cd(), 0x05)).unwrap();
    assert!(matches!(typed, TypedInit::GalleryForgets(_)),
        "GALLERY_FORGETS must dispatch to GalleryForgets variant: {:?}", typed);
}

#[test]
fn paradigm_lane_sgb_dispatches() {
    let typed = materialise(&instr(SGB_TYPE_SYSTEM, sgb_cd(), 0x06)).unwrap();
    assert!(matches!(typed, TypedInit::Sgb(_)),
        "SGB_TYPE_SYSTEM must dispatch to Sgb variant: {:?}", typed);
}

#[test]
fn childkey_letter_dispatches() {
    let typed = materialise(&instr(CHILDKEY_LETTER, childkey_cd(), 0x07)).unwrap();
    assert!(matches!(typed, TypedInit::Childkey(_)),
        "CHILDKEY_LETTER must dispatch to Childkey variant: {:?}", typed);
}

#[test]
fn unknown_class_returns_engine_error() {
    let err = materialise(&instr(TemplateClass(0xFFFF_FFFF), vec![], 0x08)).unwrap_err();
    assert!(
        matches!(err, EngineError::UnknownTemplate(0xFFFF_FFFF)),
        "unknown class must produce UnknownTemplate error: {:?}", err
    );
}

#[test]
fn malformed_json_returns_parse_failed() {
    let err = materialise(&instr(MAYFLY, vec![b'!', b'@'], 0x09)).unwrap_err();
    assert!(
        matches!(err, EngineError::ParseFailed(_)),
        "malformed JSON must produce ParseFailed: {:?}", err
    );
}

#[test]
fn calldata_too_large_rejected() {
    // SUB-N2: pre-gas parse-tree allocation prevented by cap.
    let huge = vec![0u8; MAX_INIT_CALLDATA + 1];
    let err = materialise(&instr(MAYFLY, huge, 0x0A)).unwrap_err();
    assert!(
        matches!(err, EngineError::CalldataTooLarge { got, cap }
            if got == MAX_INIT_CALLDATA + 1 && cap == MAX_INIT_CALLDATA),
        "oversized calldata must be rejected: {:?}", err
    );
}

#[test]
fn calldata_at_exact_cap_is_ok_structurally() {
    // At exactly the cap, the length check passes (only JSON parse can fail).
    // Use a valid mayfly JSON padded with trailing whitespace to fill the cap.
    let mut cd = mayfly_cd();
    while cd.len() < MAX_INIT_CALLDATA {
        cd.push(b' ');
    }
    // This may fail at JSON parse (trailing whitespace after valid JSON is
    // implementation-defined), but it must NOT fail with CalldataTooLarge.
    let result = materialise(&instr(MAYFLY, cd, 0x0B));
    assert!(
        !matches!(result, Err(EngineError::CalldataTooLarge { .. })),
        "at-cap calldata must not be rejected by length guard: {:?}", result
    );
}

#[test]
fn dispatch_is_deterministic() {
    let i1 = instr(SDDC_AUCTION, sddc_cd(), 0x0C);
    let i2 = instr(SDDC_AUCTION, sddc_cd(), 0x0C);
    assert_eq!(materialise(&i1).unwrap(), materialise(&i2).unwrap(),
        "same instruction must produce identical TypedInit");
}

#[test]
fn typed_init_carries_parsed_fields() {
    // Verify the typed struct fields are actually populated.
    let typed = materialise(&instr(SDDC_AUCTION, sddc_cd(), 0x0D)).unwrap();
    match typed {
        TypedInit::Sddc(cfg) => {
            assert_eq!(cfg.ceiling, 10_000);
            assert_eq!(cfg.floor,   1_000);
            assert_eq!(cfg.lot_lambda, 50);
            assert_eq!(cfg.duration_epochs, 500);
        }
        other => panic!("expected Sddc, got {:?}", other),
    }
}

#[test]
fn priya_engine_dispatch_full_arc() {
    // Full arc: PRIYA processes one instruction per lane.
    let cases: &[(TemplateClass, Vec<u8>)] = &[
        (MAYFLY,          mayfly_cd()),
        (SINGH_SABI,      sabi_cd()),
        (SDDC_AUCTION,    sddc_cd()),
        (MNEMOCHAIN_CARD, mnemochain_cd()),
        (GALLERY_FORGETS, gallery_cd()),
        (SGB_TYPE_SYSTEM, sgb_cd()),
        (CHILDKEY_LETTER, childkey_cd()),
    ];

    for (i, (class, cd)) in cases.iter().enumerate() {
        let result = materialise(&instr(*class, cd.clone(), i as u8));
        assert!(result.is_ok(),
            "lane dispatch for class {:#010x} must succeed: {:?}", class.0, result);
    }

    // Unknown class is a hard error, not a fallback.
    let ghost = TemplateClass(0x0001_FFFE);
    assert!(
        matches!(materialise(&instr(ghost, vec![], 0xFF)), Err(EngineError::UnknownTemplate(_))),
        "unknown class must be hard error"
    );
}
