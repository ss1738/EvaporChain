//! Parser-roundtrip checks for the EvaporScript stdlib (`contracts/evaporscript/`).
//!
//! Per-contract behavioural pilots already exist for the seed-3
//! (`mortal_message`, `mortal_nft`, `energy_pool`). This test file is the
//! one-shot regression check for the seed-12 stdlib contracts shipped after
//! them — it confirms each one parses + compiles cleanly, surfaces the
//! expected public methods, and wires the standard lifecycle hooks
//! (`on_grace`, `on_refresh`, `on_evaporate`).
//!
//! The intent is "if any of these break, ship a fix before adding more
//! contracts." Behavioural tests per contract (state transitions, auth
//! gates, revert paths) live in dedicated files modelled on the seed-3
//! pilots and can be added incrementally.

use evaporchain_script::{compiler, parser};

fn assert_parses_and_compiles(name: &str, source: &str, expected_methods: &[&str]) {
    let ast = parser::parse(source)
        .unwrap_or_else(|e| panic!("{name}: parse failed — {e:?}"));
    let bc = compiler::compile(&ast)
        .unwrap_or_else(|e| panic!("{name}: compile failed — {e:?}"));
    assert_eq!(
        bc.name, name,
        "{name}: contract name mismatch in compiled bytecode"
    );
    for m in expected_methods {
        assert!(
            bc.methods.contains_key(*m),
            "{name}: method `{m}` missing from compiled bytecode"
        );
    }
    // Every stdlib contract wires the canonical lifecycle hook trio. Pilot
    // contracts that only wire a subset are tracked separately; the stdlib
    // standard is all three.
    for hook in ["on_grace", "on_refresh", "on_evaporate"] {
        assert!(
            bc.methods.contains_key(hook),
            "{name}: lifecycle hook `{hook}` missing"
        );
    }
}

#[test]
fn payment_split_compiles() {
    let src = include_str!("../../../contracts/evaporscript/payment_split.es");
    assert_parses_and_compiles(
        "PaymentSplit",
        src,
        &[
            "add_recipient",
            "seal",
            "deposit",
            "claim",
            "entitlement_of",
            "pending_of",
            "share_of",
            "total_pool",
            "recipients",
        ],
    );
}

#[test]
fn sealed_bid_auction_compiles() {
    let src = include_str!("../../../contracts/evaporscript/sealed_bid_auction.es");
    assert_parses_and_compiles(
        "SealedBidAuction",
        src,
        &[
            "set_metadata",
            "set_phase",
            "commit",
            "reveal",
            "record_winner",
            "current_phase",
            "nominal_bid_of",
            "effective_bid_of",
            "winner_of",
            "is_settled",
            "commits_received",
            "reveals_received",
        ],
    );
}

#[test]
fn vesting_schedule_compiles() {
    let src = include_str!("../../../contracts/evaporscript/vesting_schedule.es");
    assert_parses_and_compiles(
        "VestingSchedule",
        src,
        &[
            "set_terms",
            "claim",
            "vested_now",
            "cancel",
            "vested_amount",
            "pending_amount",
            "beneficiary_of",
            "grant_total",
            "cliff_at",
            "fully_vested_at",
        ],
    );
}

#[test]
fn time_lock_compiles() {
    let src = include_str!("../../../contracts/evaporscript/time_lock.es");
    assert_parses_and_compiles(
        "TimeLock",
        src,
        &[
            "set_terms",
            "claim",
            "revoke",
            "beneficiary_of",
            "locked",
            "unlock_at",
            "is_unlocked",
            "is_claimed",
        ],
    );
}

#[test]
fn attestation_compiles() {
    let src = include_str!("../../../contracts/evaporscript/attestation.es");
    assert_parses_and_compiles(
        "Attestation",
        src,
        &[
            "attest",
            "endorse",
            "revoke",
            "subject_of",
            "claim_text",
            "attestor_of",
            "attested_at",
            "is_revoked",
            "endorsements_total",
            "has_endorsement",
            "age",
        ],
    );
}

#[test]
fn oracle_feed_compiles() {
    let src = include_str!("../../../contracts/evaporscript/oracle_feed.es");
    assert_parses_and_compiles(
        "OracleFeed",
        src,
        &[
            "set_feed",
            "update",
            "latest",
            "age",
            "dispute",
            "feed_label",
            "updates_total",
            "disputes_total",
            "last_updated",
            "is_fresh",
        ],
    );
}

#[test]
fn subscription_compiles() {
    let src = include_str!("../../../contracts/evaporscript/subscription.es");
    assert_parses_and_compiles(
        "Subscription",
        src,
        &[
            "set_terms",
            "pay",
            "cancel",
            "provider_of",
            "subscriber_of",
            "amount_per_period",
            "period_length",
            "periods_paid",
            "total_paid",
            "last_payment",
            "is_active",
        ],
    );
}

#[test]
fn multisig_compiles() {
    let src = include_str!("../../../contracts/evaporscript/multisig.es");
    assert_parses_and_compiles(
        "Multisig",
        src,
        &[
            "add_signer",
            "set_threshold",
            "propose",
            "sign",
            "execute",
            "signers_total",
            "threshold_required",
            "signatures_collected",
            "has_signed",
            "is_signer",
            "proposal_action",
            "is_executed",
            "is_pending",
        ],
    );
}

#[test]
fn lottery_compiles() {
    let src = include_str!("../../../contracts/evaporscript/lottery.es");
    assert_parses_and_compiles(
        "Lottery",
        src,
        &[
            "set_event",
            "enter",
            "set_winner",
            "claim_prize",
            "entries_total",
            "is_entered",
            "winner_of",
            "vrf_proof",
            "is_drawn",
            "is_voided",
            "prize_size",
            "stake_per_entry",
        ],
    );
}

#[test]
fn bounty_compiles() {
    let src = include_str!("../../../contracts/evaporscript/bounty.es");
    assert_parses_and_compiles(
        "Bounty",
        src,
        &[
            "set_bounty",
            "submit",
            "accept",
            "claim",
            "cancel",
            "task_of",
            "reward",
            "submissions_total",
            "submission_of",
            "winner_of",
            "is_accepted",
            "is_claimed",
        ],
    );
}

#[test]
fn dead_man_switch_compiles() {
    let src = include_str!("../../../contracts/evaporscript/dead_man_switch.es");
    assert_parses_and_compiles(
        "DeadManSwitch",
        src,
        &[
            "set_switch",
            "check_in",
            "disarm",
            "claim",
            "principal_of",
            "beneficiary_of",
            "last_checkin",
            "checkins_total",
            "is_released",
            "is_claimed",
            "is_disarmed",
            "silence_age",
        ],
    );
}

#[test]
fn energy_marketplace_compiles() {
    let src = include_str!("../../../contracts/evaporscript/energy_marketplace.es");
    assert_parses_and_compiles(
        "EnergyMarketplace",
        src,
        &[
            "set_market",
            "list",
            "buy",
            "cancel",
            "market_label",
            "active_listings",
            "units_sold_total",
            "evap_volume_total",
            "trades_total",
            "listing_units_of",
            "listing_price_of",
            "has_listing",
            "is_open",
        ],
    );
}
