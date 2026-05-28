//! §Contracts e2e
//!
//! Scenario: "KAITO DecayingToken arc" — KAITO is the DecayingToken issuer.
//! He deploys KAITO-COIN, mints tokens for RAFAEL, who transfers to IGOR.
//! IGOR burns their tokens. The rule engine rejects an unauthorized mint
//! attempt and emits an event on a valid one. Two tokens have isolated
//! state. A short-lived token evaporates under tick().
//!
//! The suite proves: ContractEngine::deploy assigns sequential ids; mint
//! is creator-only (PermissionDenied otherwise); balance_of round-trips;
//! transfer requires caller == from (PermissionDenied otherwise);
//! InsufficientFunds fires when balance < amount; burn removes balance;
//! NotFound fires on unknown id; UnknownMethod fires for unmapped methods;
//! Rule(OnMint, Always, Reject) blocks a mint (RejectedByRule); Rule
//! EmitEvent surfaces in CallResult.events; tick() evaporates zero-energy
//! contracts; calling an evaporated contract returns ContractError::Evaporated.

use evaporchain_contracts::{
    CallResult, ContractEngine, ContractError, ContractTemplate, Rule, RuleAction, RuleCondition,
    RuleTrigger,
};

fn addr(b: u8) -> [u8; 32] {
    [b; 32]
}
fn hex_addr(b: u8) -> String {
    format!("0x{}", hex::encode(addr(b)))
}

fn deploy_token(engine: &mut ContractEngine, name: &str, creator: u8) -> u64 {
    let params = serde_json::json!({
        "name": name,
        "symbol": &name[..3].to_uppercase(),
        "total_supply": 0u64,
        "decay_half_life": 1_000u64,
        "owner": hex_addr(creator),
    });
    engine
        .deploy(
            ContractTemplate::DecayingToken,
            params,
            vec![],
            addr(creator),
            10_000,
            100,
            0,
        )
        .unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[test]
fn deploy_assigns_sequential_ids() {
    let mut engine = ContractEngine::new();
    let id1 = deploy_token(&mut engine, "FIRST", 0xAA);
    let id2 = deploy_token(&mut engine, "SECOND", 0xAA);
    assert_eq!(id1, 1);
    assert_eq!(id2, 2);
}

#[test]
fn mint_and_balance_of_round_trip() {
    let mut engine = ContractEngine::new();
    let creator = 0xAA_u8;
    let id = deploy_token(&mut engine, "TOKEN", creator);

    engine
        .call(
            id,
            "mint",
            &serde_json::json!({"to": hex_addr(0xBB), "amount": 100u64}),
            &addr(creator),
            0,
        )
        .unwrap();

    let r = engine
        .call(
            id,
            "balance_of",
            &serde_json::json!({"addr": hex_addr(0xBB)}),
            &addr(creator),
            0,
        )
        .unwrap();
    assert_eq!(r.return_value["balance"], serde_json::json!(100u64));
}

#[test]
fn mint_requires_creator() {
    let mut engine = ContractEngine::new();
    let id = deploy_token(&mut engine, "TOKEN", 0xAA);
    let err = engine
        .call(
            id,
            "mint",
            &serde_json::json!({"to": hex_addr(0xBB), "amount": 50u64}),
            &addr(0xBB),
            0,
        ) // 0xBB != creator 0xAA
        .unwrap_err();
    assert!(
        matches!(err, ContractError::PermissionDenied(_)),
        "non-creator mint must be PermissionDenied, got {err:?}"
    );
}

#[test]
fn transfer_reduces_sender_and_increases_receiver() {
    let mut engine = ContractEngine::new();
    let creator = 0xAA_u8;
    let id = deploy_token(&mut engine, "TOKEN", creator);

    // Mint 100 to RAFAEL (0xBB).
    engine
        .call(
            id,
            "mint",
            &serde_json::json!({"to": hex_addr(0xBB), "amount": 100u64}),
            &addr(creator),
            0,
        )
        .unwrap();

    // RAFAEL transfers 30 to IGOR (0xCC); caller must equal from.
    engine
        .call(
            id,
            "transfer",
            &serde_json::json!({"from": hex_addr(0xBB), "to": hex_addr(0xCC), "amount": 30u64}),
            &addr(0xBB),
            0,
        )
        .unwrap();

    let r_b = engine
        .call(
            id,
            "balance_of",
            &serde_json::json!({"addr": hex_addr(0xBB)}),
            &addr(creator),
            0,
        )
        .unwrap();
    let r_c = engine
        .call(
            id,
            "balance_of",
            &serde_json::json!({"addr": hex_addr(0xCC)}),
            &addr(creator),
            0,
        )
        .unwrap();
    assert_eq!(r_b.return_value["balance"], serde_json::json!(70u64));
    assert_eq!(r_c.return_value["balance"], serde_json::json!(30u64));
}

#[test]
fn transfer_requires_caller_equals_from() {
    let mut engine = ContractEngine::new();
    let creator = 0xAA_u8;
    let id = deploy_token(&mut engine, "TOKEN", creator);
    engine
        .call(
            id,
            "mint",
            &serde_json::json!({"to": hex_addr(0xBB), "amount": 100u64}),
            &addr(creator),
            0,
        )
        .unwrap();

    // KAITO (0xAA) tries to move RAFAEL's (0xBB) tokens → denied.
    let err = engine
        .call(
            id,
            "transfer",
            &serde_json::json!({"from": hex_addr(0xBB), "to": hex_addr(0xCC), "amount": 10u64}),
            &addr(creator),
            0,
        )
        .unwrap_err();
    assert!(
        matches!(err, ContractError::PermissionDenied(_)),
        "caller != from must be PermissionDenied, got {err:?}"
    );
}

#[test]
fn transfer_insufficient_funds() {
    let mut engine = ContractEngine::new();
    let creator = 0xAA_u8;
    let id = deploy_token(&mut engine, "TOKEN", creator);
    engine
        .call(
            id,
            "mint",
            &serde_json::json!({"to": hex_addr(0xBB), "amount": 10u64}),
            &addr(creator),
            0,
        )
        .unwrap();

    // RAFAEL tries to transfer 999 but only has 10.
    let err = engine
        .call(
            id,
            "transfer",
            &serde_json::json!({"from": hex_addr(0xBB), "to": hex_addr(0xCC), "amount": 999u64}),
            &addr(0xBB),
            0,
        )
        .unwrap_err();
    assert!(
        matches!(err, ContractError::InsufficientFunds { .. }),
        "over-transfer must be InsufficientFunds, got {err:?}"
    );
}

#[test]
fn burn_removes_from_balance() {
    let mut engine = ContractEngine::new();
    let creator = 0xAA_u8;
    let id = deploy_token(&mut engine, "TOKEN", creator);
    engine
        .call(
            id,
            "mint",
            &serde_json::json!({"to": hex_addr(0xBB), "amount": 50u64}),
            &addr(creator),
            0,
        )
        .unwrap();

    engine
        .call(
            id,
            "burn",
            &serde_json::json!({"from": hex_addr(0xBB), "amount": 20u64}),
            &addr(0xBB),
            0,
        )
        .unwrap();

    let r = engine
        .call(
            id,
            "balance_of",
            &serde_json::json!({"addr": hex_addr(0xBB)}),
            &addr(creator),
            0,
        )
        .unwrap();
    assert_eq!(r.return_value["balance"], serde_json::json!(30u64));
}

#[test]
fn not_found_for_unknown_contract_id() {
    let mut engine = ContractEngine::new();
    let err = engine
        .call(
            999,
            "balance_of",
            &serde_json::json!({"addr": hex_addr(0xBB)}),
            &addr(0xAA),
            0,
        )
        .unwrap_err();
    assert!(
        matches!(err, ContractError::NotFound(999)),
        "unknown id must be NotFound, got {err:?}"
    );
}

#[test]
fn unknown_method_rejected() {
    let mut engine = ContractEngine::new();
    let id = deploy_token(&mut engine, "TOKEN", 0xAA);
    let err = engine
        .call(
            id,
            "fly_to_the_moon",
            &serde_json::json!({}),
            &addr(0xAA),
            0,
        )
        .unwrap_err();
    assert!(
        matches!(err, ContractError::UnknownMethod(_)),
        "unmapped method must be UnknownMethod, got {err:?}"
    );
}

#[test]
fn rule_engine_rejects_mint() {
    let mut engine = ContractEngine::new();
    let rules = vec![Rule {
        trigger: RuleTrigger::OnMint,
        condition: RuleCondition::Always,
        action: RuleAction::Reject,
    }];
    let params = serde_json::json!({
        "name": "GATED",
        "symbol": "GAT",
        "total_supply": 0u64,
        "decay_half_life": 1_000u64,
        "owner": hex_addr(0xAA),
    });
    let id = engine
        .deploy(
            ContractTemplate::DecayingToken,
            params,
            rules,
            addr(0xAA),
            10_000,
            100,
            0,
        )
        .unwrap();

    let err = engine
        .call(
            id,
            "mint",
            &serde_json::json!({"to": hex_addr(0xBB), "amount": 1u64}),
            &addr(0xAA),
            0,
        )
        .unwrap_err();
    assert!(
        matches!(err, ContractError::RejectedByRule(_)),
        "OnMint Reject rule must fire RejectedByRule, got {err:?}"
    );
}

#[test]
fn rule_engine_emits_event_on_mint() {
    let mut engine = ContractEngine::new();
    let rules = vec![Rule {
        trigger: RuleTrigger::OnMint,
        condition: RuleCondition::Always,
        action: RuleAction::EmitEvent("mint_alert".to_string()),
    }];
    let params = serde_json::json!({
        "name": "EVENTED",
        "symbol": "EVT",
        "total_supply": 0u64,
        "decay_half_life": 1_000u64,
        "owner": hex_addr(0xAA),
    });
    let id = engine
        .deploy(
            ContractTemplate::DecayingToken,
            params,
            rules,
            addr(0xAA),
            10_000,
            100,
            0,
        )
        .unwrap();

    let result: CallResult = engine
        .call(
            id,
            "mint",
            &serde_json::json!({"to": hex_addr(0xBB), "amount": 10u64}),
            &addr(0xAA),
            0,
        )
        .unwrap();
    assert!(
        result.events.iter().any(|e| e.contains("mint_alert")),
        "OnMint EmitEvent rule must surface in CallResult.events: {:?}",
        result.events
    );
}

#[test]
fn tick_evaporates_zero_energy_contract() {
    let mut engine = ContractEngine::new();
    // energy=64, half_life=1: energy_at_epoch(64, 1, 7) = 64>>7 = 0
    let params = serde_json::json!({
        "name": "MORTAL",
        "symbol": "MRT",
        "total_supply": 0u64,
        "decay_half_life": 1_000u64,
        "owner": hex_addr(0xAA),
    });
    let id = engine
        .deploy(
            ContractTemplate::DecayingToken,
            params,
            vec![],
            addr(0xAA),
            64,
            1,
            0,
        )
        .unwrap();

    // Callable at epoch 0 (elapsed=0, energy=64).
    engine
        .call(
            id,
            "balance_of",
            &serde_json::json!({"addr": hex_addr(0xBB)}),
            &addr(0xAA),
            0,
        )
        .unwrap();

    let tick = engine.tick(7);
    assert!(
        tick.contracts_evaporated.contains(&id),
        "zero-energy contract must be in contracts_evaporated"
    );
}

#[test]
fn call_on_evaporated_contract_is_evaporated_error() {
    let mut engine = ContractEngine::new();
    let params = serde_json::json!({
        "name": "GHOST",
        "symbol": "GHO",
        "total_supply": 0u64,
        "decay_half_life": 1_000u64,
        "owner": hex_addr(0xAA),
    });
    let id = engine
        .deploy(
            ContractTemplate::DecayingToken,
            params,
            vec![],
            addr(0xAA),
            64,
            1,
            0,
        )
        .unwrap();
    engine.tick(7); // evaporate

    let err = engine
        .call(
            id,
            "balance_of",
            &serde_json::json!({"addr": hex_addr(0xBB)}),
            &addr(0xAA),
            7,
        )
        .unwrap_err();
    assert!(
        matches!(err, ContractError::Evaporated(_)),
        "call on evaporated contract must return Evaporated, got {err:?}"
    );
}

#[test]
fn two_contracts_have_isolated_state() {
    let mut engine = ContractEngine::new();
    let id_a = deploy_token(&mut engine, "TOKENA", 0xAA);
    let id_b = deploy_token(&mut engine, "TOKENB", 0xAA);

    // Mint 100 to RAFAEL in token A.
    engine
        .call(
            id_a,
            "mint",
            &serde_json::json!({"to": hex_addr(0xBB), "amount": 100u64}),
            &addr(0xAA),
            0,
        )
        .unwrap();

    // Check RAFAEL's balance in token B is still 0.
    let r = engine
        .call(
            id_b,
            "balance_of",
            &serde_json::json!({"addr": hex_addr(0xBB)}),
            &addr(0xAA),
            0,
        )
        .unwrap();
    assert_eq!(
        r.return_value["balance"],
        serde_json::json!(0u64),
        "minting in A must not affect B's state"
    );
}

#[test]
fn kaito_token_full_arc() {
    // Full arc: KAITO deploys KAITO-COIN, mints for RAFAEL, RAFAEL
    // transfers to IGOR, IGOR burns their tokens. The rule engine
    // rejects an unauthorized call. Short-lived token evaporates on tick.

    let mut engine = ContractEngine::new();
    let kaito = 0xAA_u8;
    let rafael = 0xBB_u8;
    let igor = 0xCC_u8;

    let id = deploy_token(&mut engine, "KAITO-COIN", kaito);

    // KAITO mints 1_000 tokens to RAFAEL.
    engine
        .call(
            id,
            "mint",
            &serde_json::json!({"to": hex_addr(rafael), "amount": 1_000u64}),
            &addr(kaito),
            0,
        )
        .unwrap();

    // RAFAEL checks their balance.
    let r = engine
        .call(
            id,
            "balance_of",
            &serde_json::json!({"addr": hex_addr(rafael)}),
            &addr(kaito),
            0,
        )
        .unwrap();
    assert_eq!(r.return_value["balance"], serde_json::json!(1_000u64));

    // IGOR tries to steal RAFAEL's tokens — denied.
    let steal_err = engine
        .call(
            id,
            "transfer",
            &serde_json::json!({"from": hex_addr(rafael), "to": hex_addr(igor), "amount": 500u64}),
            &addr(igor),
            0,
        )
        .unwrap_err();
    assert!(matches!(steal_err, ContractError::PermissionDenied(_)));

    // RAFAEL transfers 400 to IGOR legitimately.
    engine
        .call(
            id,
            "transfer",
            &serde_json::json!({"from": hex_addr(rafael), "to": hex_addr(igor), "amount": 400u64}),
            &addr(rafael),
            0,
        )
        .unwrap();

    // IGOR burns 200.
    engine
        .call(
            id,
            "burn",
            &serde_json::json!({"from": hex_addr(igor), "amount": 200u64}),
            &addr(igor),
            0,
        )
        .unwrap();

    // Final balances: RAFAEL 600, IGOR 200.
    let r_r = engine
        .call(
            id,
            "balance_of",
            &serde_json::json!({"addr": hex_addr(rafael)}),
            &addr(kaito),
            0,
        )
        .unwrap();
    let r_i = engine
        .call(
            id,
            "balance_of",
            &serde_json::json!({"addr": hex_addr(igor)}),
            &addr(kaito),
            0,
        )
        .unwrap();
    assert_eq!(r_r.return_value["balance"], serde_json::json!(600u64));
    assert_eq!(r_i.return_value["balance"], serde_json::json!(200u64));

    // Short-lived token evaporates under tick.
    let params = serde_json::json!({
        "name": "EPHEMERAL",
        "symbol": "EPH",
        "total_supply": 0u64,
        "decay_half_life": 1_000u64,
        "owner": hex_addr(kaito),
    });
    let short_id = engine
        .deploy(
            ContractTemplate::DecayingToken,
            params,
            vec![],
            addr(kaito),
            64,
            1,
            0,
        )
        .unwrap();
    let tick = engine.tick(7);
    assert!(tick.contracts_evaporated.contains(&short_id));
    assert!(
        !tick.contracts_evaporated.contains(&id),
        "KAITO-COIN must survive"
    );
}
