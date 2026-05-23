//! §Materialise — deterministic deploy-instruction derivation e2e
//!
//! Scenario: "Block-producer consensus round" — PRIYA and KIERAN
//! are two independent validators who each receive the same batch
//! of deploy requests and must produce byte-identical
//! MaterialiseInstructions without coordinating. The suite proves
//! the doctrine: pure-function determinism + canonical JSON +
//! domain-separated instance ids = every validator agrees.
//!
//! OSCAR (adversarial) attempts replay attacks (same request at a
//! later nonce) and schema-invalid submits. The two-phase validator
//! catches both before any state is touched.

use evaporchain_app_templates::class::{
    GALLERY_FORGETS, MAYFLY, MNEMOCHAIN_CARD, SDDC_AUCTION, SINGH_SABI,
};
use evaporchain_app_templates_deploy::DeployRequest;
use evaporchain_app_templates_materialise::{
    derive_instance_id, materialise_request, MaterialiseError, MaterialiseInstruction,
};
use serde_json::json;

// ── Actors ────────────────────────────────────────────────────────────────
fn priya()  -> [u8; 32] { [0xA1; 32] }
fn kieran() -> [u8; 32] { [0xB2; 32] }
fn oscar()  -> [u8; 32] { [0x0C; 32] }

// ── Request helpers ───────────────────────────────────────────────────────

fn sabi_req(deployer: [u8; 32], nonce: u64) -> DeployRequest {
    DeployRequest::new(
        SINGH_SABI,
        json!({"initial_energy": 10_000, "floor_pct": 15, "half_life": 365}),
        deployer, 1_000, nonce,
    ).unwrap()
}

fn mayfly_req(deployer: [u8; 32], nonce: u64) -> DeployRequest {
    DeployRequest::new(
        MAYFLY,
        json!({"initial_energy": 500, "half_life": 30}),
        deployer, 1_000, nonce,
    ).unwrap()
}

fn sddc_req(deployer: [u8; 32], nonce: u64) -> DeployRequest {
    DeployRequest::new(
        SDDC_AUCTION,
        json!({"ceiling": 1_000, "floor": 100, "lot_lambda": 50, "duration_epochs": 500}),
        deployer, 1_000, nonce,
    ).unwrap()
}

fn mnemo_req(deployer: [u8; 32], nonce: u64) -> DeployRequest {
    DeployRequest::new(
        MNEMOCHAIN_CARD,
        json!({"initial_energy": 1_000, "initial_stability": 10}),
        deployer, 1_000, nonce,
    ).unwrap()
}

fn gallery_req(deployer: [u8; 32], nonce: u64) -> DeployRequest {
    DeployRequest::new(
        GALLERY_FORGETS,
        json!({"opened_at_epoch": 0}),
        deployer, 1_000, nonce,
    ).unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn two_validators_produce_identical_instruction() {
    // PRIYA and KIERAN each independently materialise the same SinghSabi
    // request. The resulting instructions must be byte-identical.
    let req = sabi_req(priya(), 1);
    let from_priya  = materialise_request(&req).unwrap();
    let from_kieran = materialise_request(&req).unwrap();
    assert_eq!(from_priya, from_kieran,
        "validators must agree on every byte of the dispatch envelope");
}

#[test]
fn canonical_json_eliminates_key_ordering() {
    // A deployer submits the same SinghSabi params in two different
    // key orderings — serde may preserve insertion order. Canonical
    // JSON must sort keys before hashing so both produce the same
    // init_calldata. Critical for validator agreement when JSON is
    // assembled by different client libraries.
    let req_ordered = DeployRequest::new(
        SINGH_SABI,
        json!({"initial_energy": 10_000, "floor_pct": 15, "half_life": 365}),
        priya(), 1_000, 42,
    ).unwrap();
    let req_shuffled = DeployRequest::new(
        SINGH_SABI,
        json!({"half_life": 365, "initial_energy": 10_000, "floor_pct": 15}),
        priya(), 1_000, 42,
    ).unwrap();

    let i1 = materialise_request(&req_ordered).unwrap();
    let i2 = materialise_request(&req_shuffled).unwrap();

    assert_eq!(i1.init_calldata, i2.init_calldata,
        "canonical JSON must produce identical calldata regardless of key order");
    assert_eq!(i1.instance_id, i2.instance_id);
}

#[test]
fn nonce_provides_replay_resistance() {
    // PRIYA deploys SinghSabi twice with different nonces — different
    // deploys, different instance IDs. OSCAR cannot replay nonce=0
    // as nonce=1 and get the same on-chain handle.
    let first  = materialise_request(&sabi_req(priya(), 0)).unwrap();
    let second = materialise_request(&sabi_req(priya(), 1)).unwrap();

    assert_ne!(first.instance_id, second.instance_id,
        "each nonce must produce a distinct instance id (replay resistance)");
    assert_eq!(first.template_class,  SINGH_SABI);
    assert_eq!(second.template_class, SINGH_SABI);
}

#[test]
fn instance_id_depends_only_on_class_deployer_nonce() {
    // Same class + deployer + nonce but different params → same instance ID.
    // The contract engine addresses by identity, not by params content.
    let req_a = DeployRequest::new(
        SINGH_SABI,
        json!({"initial_energy": 1_000, "floor_pct": 10, "half_life": 365}),
        priya(), 0, 7,
    ).unwrap();
    let req_b = DeployRequest::new(
        SINGH_SABI,
        json!({"initial_energy": 9_999, "floor_pct": 99, "half_life": 365}),
        priya(), 0, 7,
    ).unwrap();

    let ia = materialise_request(&req_a).unwrap();
    let ib = materialise_request(&req_b).unwrap();

    assert_eq!(ia.instance_id, ib.instance_id,
        "instance id must be param-independent");
    assert_ne!(ia.init_calldata, ib.init_calldata,
        "calldata must capture the different params");
}

#[test]
fn different_deployers_produce_different_instances() {
    // PRIYA and KIERAN deploy the same class with the same nonce —
    // their instance IDs must still differ. Two validators could not
    // accidentally share an on-chain handle.
    let priya_instr  = materialise_request(&sabi_req(priya(),  0)).unwrap();
    let kieran_instr = materialise_request(&sabi_req(kieran(), 0)).unwrap();

    assert_ne!(priya_instr.instance_id, kieran_instr.instance_id,
        "different deployers must produce different instance IDs");
}

#[test]
fn epoch_at_submit_does_not_affect_instance_id() {
    // A relayer bounces PRIYA's signed request at epoch 500 and again
    // at epoch 5_000. The same nonce produces the same instance id —
    // preventing phantom duplicate instances from epoch drift.
    let req_early = DeployRequest::new(
        MAYFLY,
        json!({"initial_energy": 500, "half_life": 30}),
        priya(), 500, 99,
    ).unwrap();
    let req_late = DeployRequest::new(
        MAYFLY,
        json!({"initial_energy": 500, "half_life": 30}),
        priya(), 5_000, 99,
    ).unwrap();

    let i_early = materialise_request(&req_early).unwrap();
    let i_late  = materialise_request(&req_late).unwrap();

    assert_eq!(i_early.instance_id, i_late.instance_id,
        "epoch at submit must not change the instance id");
}

#[test]
fn multi_template_batch_all_materialise() {
    // PRIYA deploys five primitives in a single block: SinghSabi,
    // Mayfly, SDDC, MnemoChain, GalleryForgets. All must materialise
    // and each must have a unique instance ID.
    let instrs: Vec<MaterialiseInstruction> = [
        sabi_req(priya(), 0),
        mayfly_req(priya(), 1),
        sddc_req(priya(), 2),
        mnemo_req(priya(), 3),
        gallery_req(priya(), 4),
    ].iter().map(|r| materialise_request(r).unwrap()).collect();

    // Each template class is correct.
    let classes: Vec<_> = instrs.iter().map(|i| i.template_class).collect();
    assert!(classes.contains(&SINGH_SABI));
    assert!(classes.contains(&MAYFLY));
    assert!(classes.contains(&SDDC_AUCTION));
    assert!(classes.contains(&MNEMOCHAIN_CARD));
    assert!(classes.contains(&GALLERY_FORGETS));

    // All instance IDs are distinct — five unique on-chain handles.
    let ids: std::collections::HashSet<_> = instrs.iter().map(|i| i.instance_id).collect();
    assert_eq!(ids.len(), 5, "every deploy in the batch must get a unique instance ID");
}

#[test]
fn schema_invalid_request_caught_at_materialise() {
    // OSCAR submits a SinghSabi deploy with a missing required key
    // (floor_pct). The materialiser's two-phase validation catches it
    // even if the deploy layer somehow passed it.
    let bad = DeployRequest::new(
        SINGH_SABI,
        json!({"initial_energy": 1_000, "half_life": 365}), // missing floor_pct
        oscar(), 0, 0,
    ).unwrap(); // DeployRequest itself is permissive about value shape

    let err = materialise_request(&bad).unwrap_err();
    assert!(matches!(err, MaterialiseError::SchemaInvalid(_)),
        "missing required key must produce SchemaInvalid: {:?}", err);
}

#[test]
fn unknown_template_class_rejected() {
    // An unregistered template class (within range, not in catalogue)
    // returns MaterialiseError::UnknownTemplate at materialise time.
    use evaporchain_app_templates::TemplateClass;
    let ghost_class = TemplateClass(0x0001_0FFF);
    let req = DeployRequest::new(ghost_class, json!({}), oscar(), 0, 0).unwrap();
    let err = materialise_request(&req).unwrap_err();
    assert_eq!(err, MaterialiseError::UnknownTemplate(ghost_class.0),
        "unregistered class must be rejected: {:?}", err);
}

#[test]
fn instruction_serde_round_trip() {
    // MaterialiseInstruction serialises and deserialises without loss.
    // Relay nodes pass instructions as JSON; both ends must see the
    // same instance_id and init_calldata bytes.
    let instr = materialise_request(&sddc_req(kieran(), 5)).unwrap();
    let json  = serde_json::to_string(&instr).unwrap();
    let back: MaterialiseInstruction = serde_json::from_str(&json).unwrap();
    assert_eq!(instr, back, "serialised instruction must round-trip exactly");
}

#[test]
fn instance_id_matches_derive_instance_id_directly() {
    // The instance_id inside MaterialiseInstruction must equal what
    // derive_instance_id produces for the same (class, deployer, nonce).
    // Validates that materialise_request and the public derive function
    // are consistent — both validators can precompute the id before
    // the full instruction is assembled.
    let req = sabi_req(priya(), 3);
    let instr = materialise_request(&req).unwrap();
    let expected = derive_instance_id(SINGH_SABI, &priya(), 3);
    assert_eq!(instr.instance_id, expected,
        "instruction instance_id must match derive_instance_id");
}

#[test]
fn calldata_is_non_empty_for_all_primitives() {
    // Every valid deploy must produce a non-empty init_calldata.
    // An empty calldata would silently discard the params.
    for req in [
        sabi_req(priya(), 0),
        mayfly_req(priya(), 1),
        sddc_req(priya(), 2),
        mnemo_req(priya(), 3),
        gallery_req(priya(), 4),
    ] {
        let instr = materialise_request(&req).unwrap();
        assert!(!instr.init_calldata.is_empty(),
            "{:?} produced empty calldata", instr.template_class);
    }
}

#[test]
fn full_consensus_round_priya_kieran_agree_on_batch() {
    // Full arc: PRIYA and KIERAN both materialise the same 3-deploy
    // batch. Every instruction must be byte-identical between them.
    // This is the headline guarantee the crate's doc promises.
    let batch = [
        sabi_req(priya(), 10),
        mayfly_req(priya(), 11),
        sddc_req(priya(), 12),
    ];

    for req in &batch {
        let from_priya  = materialise_request(req).unwrap();
        let from_kieran = materialise_request(req).unwrap();
        assert_eq!(from_priya, from_kieran,
            "class {:?}: validators must agree on the instruction",
            req.template_class);
    }
}
